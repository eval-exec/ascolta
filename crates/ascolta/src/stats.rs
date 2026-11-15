//! Local play history backed by SQLite.
//!
//! This powers the History screen and is independent of scrobbling: plays are
//! recorded locally even when no network service is configured.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};

const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS plays (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    uri TEXT NOT NULL,
    artist TEXT NOT NULL,
    title TEXT NOT NULL,
    album TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    played_seconds INTEGER NOT NULL,
    completed INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS plays_started ON plays(started_at);
CREATE INDEX IF NOT EXISTS plays_uri ON plays(uri);
";

#[derive(Debug, Clone, Default)]
pub struct StatsReport {
    pub total_plays: u64,
    pub total_seconds: u64,
    pub top_songs: Vec<CountedSong>,
    pub top_artists: Vec<Counted>,
    pub top_albums: Vec<Counted>,
    pub recent: Vec<RecentPlay>,
}

#[derive(Debug, Clone)]
pub struct CountedSong {
    pub uri: String,
    pub artist: String,
    pub title: String,
    pub plays: u64,
}

#[derive(Debug, Clone)]
pub struct Counted {
    pub name: String,
    pub plays: u64,
}

#[derive(Debug, Clone)]
pub struct RecentPlay {
    pub artist: String,
    pub title: String,
    pub played_at: u64,
}

pub fn db_path() -> Option<PathBuf> {
    Some(crate::config::state_dir()?.join("ascolta.db"))
}

fn open_at(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    connection
        .execute_batch(SCHEMA)
        .map_err(|error| format!("could not initialise stats database: {error}"))?;
    Ok(connection)
}

#[allow(clippy::too_many_arguments)]
pub fn record_play_at(
    path: &Path,
    uri: &str,
    artist: &str,
    title: &str,
    album: &str,
    started_at: u64,
    played_seconds: u64,
    completed: bool,
) -> Result<(), String> {
    let connection = open_at(path)?;
    connection
        .execute(
            "INSERT INTO plays (uri, artist, title, album, started_at, played_seconds, completed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uri,
                artist,
                title,
                album,
                started_at as i64,
                played_seconds as i64,
                i64::from(completed)
            ],
        )
        .map_err(|error| format!("could not record play: {error}"))?;
    Ok(())
}

/// Records a play on a background thread. Errors are logged, never fatal.
pub fn spawn_record(
    uri: String,
    artist: String,
    title: String,
    album: String,
    started_at: u64,
    played_seconds: u64,
    completed: bool,
) {
    let Some(path) = db_path() else {
        return;
    };
    std::thread::spawn(move || {
        if let Err(error) = record_play_at(
            &path,
            &uri,
            &artist,
            &title,
            &album,
            started_at,
            played_seconds,
            completed,
        ) {
            tracing::warn!(%error, "could not record play");
        }
    });
}

pub fn load_report() -> Result<StatsReport, String> {
    let path = db_path().ok_or_else(|| "could not determine state directory".to_string())?;
    load_report_at(&path)
}

pub fn load_report_at(path: &Path) -> Result<StatsReport, String> {
    let connection = open_at(path)?;
    let (total_plays, total_seconds): (i64, i64) = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(played_seconds), 0) FROM plays",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("could not read totals: {error}"))?;

    let mut statement = connection
        .prepare(
            "SELECT uri, artist, title, COUNT(*) AS plays
             FROM plays GROUP BY uri ORDER BY plays DESC, title COLLATE NOCASE LIMIT 20",
        )
        .map_err(|error| format!("could not query top songs: {error}"))?;
    let top_songs = statement
        .query_map([], |row| {
            Ok(CountedSong {
                uri: row.get(0)?,
                artist: row.get(1)?,
                title: row.get(2)?,
                plays: row.get::<_, i64>(3)? as u64,
            })
        })
        .map_err(|error| format!("could not read top songs: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not decode top songs: {error}"))?;

    let mut statement = connection
        .prepare(
            "SELECT artist, COUNT(*) AS plays FROM plays
             GROUP BY artist ORDER BY plays DESC, artist COLLATE NOCASE LIMIT 20",
        )
        .map_err(|error| format!("could not query top artists: {error}"))?;
    let top_artists = statement
        .query_map([], |row| {
            Ok(Counted {
                name: row.get(0)?,
                plays: row.get::<_, i64>(1)? as u64,
            })
        })
        .map_err(|error| format!("could not read top artists: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not decode top artists: {error}"))?;

    let mut statement = connection
        .prepare(
            "SELECT album, COUNT(*) AS plays FROM plays
             GROUP BY album ORDER BY plays DESC, album COLLATE NOCASE LIMIT 20",
        )
        .map_err(|error| format!("could not query top albums: {error}"))?;
    let top_albums = statement
        .query_map([], |row| {
            Ok(Counted {
                name: row.get(0)?,
                plays: row.get::<_, i64>(1)? as u64,
            })
        })
        .map_err(|error| format!("could not read top albums: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not decode top albums: {error}"))?;

    let mut statement = connection
        .prepare(
            "SELECT artist, title, started_at FROM plays
             ORDER BY started_at DESC LIMIT 12",
        )
        .map_err(|error| format!("could not query recent plays: {error}"))?;
    let recent = statement
        .query_map([], |row| {
            Ok(RecentPlay {
                artist: row.get(0)?,
                title: row.get(1)?,
                played_at: row.get::<_, i64>(2)? as u64,
            })
        })
        .map_err(|error| format!("could not read recent plays: {error}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not decode recent plays: {error}"))?;

    Ok(StatsReport {
        total_plays: total_plays as u64,
        total_seconds: total_seconds as u64,
        top_songs,
        top_artists,
        top_albums,
        recent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregates_plays_by_song_artist_and_album() {
        let directory = std::env::temp_dir().join(format!("ascolta-stats-{}", std::process::id()));
        let path = directory.join("stats.db");
        let _ = std::fs::remove_file(&path);

        record_play_at(&path, "a/one", "Artist", "One", "Album", 1, 200, true).unwrap();
        record_play_at(&path, "a/one", "Artist", "One", "Album", 2, 200, true).unwrap();
        record_play_at(&path, "a/two", "Other", "Two", "Other Album", 3, 100, true).unwrap();

        let report = load_report_at(&path).unwrap();
        assert_eq!(report.total_plays, 3);
        assert_eq!(report.total_seconds, 500);
        assert_eq!(report.top_songs.len(), 2);
        assert_eq!(report.top_songs[0].uri, "a/one");
        assert_eq!(report.top_songs[0].plays, 2);
        assert_eq!(report.top_artists.len(), 2);
        assert_eq!(report.top_albums.len(), 2);
        assert_eq!(report.recent.len(), 3);
        assert_eq!(report.recent[0].title, "Two");

        let _ = std::fs::remove_dir_all(directory);
    }
}
