use std::sync::Arc;
use std::time::Duration;

use neo_ncmp_domain::{
    AudioFormat, DirEntry, EntryKind, MpdOutput, MpdStats, MpdStatus, PlaybackState, Song,
};

use crate::protocol::Response;

/// A stored playlist entry from `listplaylists`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub name: Arc<str>,
    pub last_modified: Option<Arc<str>>,
}

/// Parses an `lsinfo` response into directories, playlists, and tagged files.
pub fn entries(response: &Response) -> Vec<DirEntry> {
    let mut entries = Vec::new();
    let mut current: Option<Song> = None;

    fn flush(entries: &mut Vec<DirEntry>, current: &mut Option<Song>) {
        if let Some(song) = current.take() {
            entries.push(DirEntry {
                kind: EntryKind::File,
                name: Arc::from(last_segment(&song.uri)),
                uri: song.uri.clone(),
                song: Some(song),
            });
        }
    }

    for (key, value) in response.pairs() {
        match key.as_str() {
            "file" => {
                flush(&mut entries, &mut current);
                current = Some(Song {
                    uri: Arc::from(value.as_str()),
                    ..Song::default()
                });
            }
            "directory" => {
                flush(&mut entries, &mut current);
                entries.push(DirEntry {
                    kind: EntryKind::Directory,
                    uri: Arc::from(value.as_str()),
                    name: Arc::from(last_segment(value)),
                    song: None,
                });
            }
            "playlist" => {
                flush(&mut entries, &mut current);
                entries.push(DirEntry {
                    kind: EntryKind::Playlist,
                    uri: Arc::from(value.as_str()),
                    name: Arc::from(value.as_str()),
                    song: None,
                });
            }
            _ => {
                if let Some(song) = &mut current {
                    apply_tag(song, key, value);
                }
            }
        }
    }
    flush(&mut entries, &mut current);
    entries
}

fn last_segment(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

pub fn songs(response: &Response) -> Vec<Song> {
    let mut songs: Vec<Song> = Vec::new();
    for (key, value) in response.pairs() {
        if key == "file" {
            songs.push(Song {
                uri: Arc::from(value.as_str()),
                ..Song::default()
            });
            continue;
        }
        let Some(song) = songs.last_mut() else {
            // `lsinfo` also returns directory: and playlist: records.
            continue;
        };
        apply_tag(song, key, value);
    }
    songs
}

fn apply_tag(song: &mut Song, key: &str, value: &str) {
    let tag: Arc<str> = Arc::from(value);
    match key {
        "Id" => song.id = value.parse().ok(),
        "Pos" => song.pos = value.parse().ok(),
        "Title" => song.title = Some(tag),
        "Artist" => song.artist = Some(tag),
        "Album" => song.album = Some(tag),
        "AlbumArtist" => song.album_artist = Some(tag),
        "Track" => song.track = leading_number(value),
        "Disc" => song.disc = leading_number(value),
        "Date" => song.date = Some(tag),
        "Genre" => song.genre = Some(tag),
        "Format" => song.format = Some(tag),
        // `Time` has second precision; `duration` is preferred when present.
        "Time" => {
            if song.duration.is_none() {
                song.duration = value.parse::<u64>().ok().map(Duration::from_secs);
            }
        }
        "duration" => song.duration = seconds(value),
        "Last-Modified" | "Added" => {}
        _ => song.tags.push((Arc::from(key), tag)),
    }
}

fn leading_number(value: &str) -> Option<u32> {
    value.split(['/', '-']).next()?.trim().parse().ok()
}

fn seconds(value: &str) -> Option<Duration> {
    let seconds: f64 = value.parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

pub fn status(response: &Response) -> MpdStatus {
    let mut status = MpdStatus::default();
    for (key, value) in response.pairs() {
        match key.as_str() {
            "volume" => status.volume = value.parse::<u8>().ok().filter(|v| *v <= 100),
            "repeat" => status.repeat = value == "1",
            "random" => status.random = value == "1",
            "single" => status.single = value == "1",
            "consume" => status.consume = value == "1",
            // MPD calls this `xfade` in status responses; accept both spellings.
            "crossfade" | "xfade" => status.crossfade = value.parse().ok(),
            "partition" => status.partition = Some(Arc::from(value.as_str())),
            "playlist" => status.playlist_version = value.parse().ok(),
            "playlistlength" => status.playlist_length = value.parse().unwrap_or(0),
            "state" => {
                status.state = match value.as_str() {
                    "play" => PlaybackState::Playing,
                    "pause" => PlaybackState::Paused,
                    _ => PlaybackState::Stopped,
                }
            }
            "song" => status.song_pos = value.parse().ok(),
            "songid" => status.song_id = value.parse().ok(),
            "elapsed" => status.elapsed = seconds(value).unwrap_or_default(),
            "duration" => status.duration = seconds(value),
            "time" => {
                // Legacy fallback: "elapsed:duration" in whole seconds.
                if let Some((elapsed, duration)) = value.split_once(':') {
                    if status.elapsed.is_zero() {
                        status.elapsed = elapsed
                            .parse::<u64>()
                            .map(Duration::from_secs)
                            .unwrap_or_default();
                    }
                    if status.duration.is_none() {
                        status.duration = duration.parse::<u64>().ok().map(Duration::from_secs);
                    }
                }
            }
            "bitrate" => status.bitrate = value.parse().ok(),
            "audio" => status.audio = AudioFormat::parse(value),
            "mixrampdb" => status.mixramp_db = value.parse().ok(),
            "mixrampdelay" => status.mixramp_delay = value.parse().ok(),
            "error" if !value.is_empty() => status.error = Some(value.clone()),
            _ => {}
        }
    }
    status
}

pub fn outputs(response: &Response) -> Vec<MpdOutput> {
    let mut outputs: Vec<MpdOutput> = Vec::new();
    for (key, value) in response.pairs() {
        match key.as_str() {
            "outputid" => outputs.push(MpdOutput {
                id: value.parse().unwrap_or(0),
                ..MpdOutput::default()
            }),
            "outputname" => {
                if let Some(output) = outputs.last_mut() {
                    output.name = Arc::from(value.as_str());
                }
            }
            "outputenabled" => {
                if let Some(output) = outputs.last_mut() {
                    output.enabled = value == "1";
                }
            }
            _ => {}
        }
    }
    outputs
}

pub fn stats(response: &Response) -> MpdStats {
    let mut stats = MpdStats::default();
    for (key, value) in response.pairs() {
        match key.as_str() {
            "artists" => stats.artists = value.parse().unwrap_or(0),
            "albums" => stats.albums = value.parse().unwrap_or(0),
            "songs" => stats.songs = value.parse().unwrap_or(0),
            "uptime" => stats.uptime = Duration::from_secs(value.parse().unwrap_or(0)),
            "playtime" => stats.playtime = Duration::from_secs(value.parse().unwrap_or(0)),
            "db_playtime" => stats.db_playtime = Duration::from_secs(value.parse().unwrap_or(0)),
            "db_update" => stats.db_update = value.parse().ok(),
            _ => {}
        }
    }
    stats
}

/// Parses `listpartitions`.
pub fn partitions(response: &Response) -> Vec<String> {
    response
        .pairs()
        .iter()
        .filter(|(key, _)| key == "partition")
        .map(|(_, value)| value.clone())
        .collect()
}

pub fn playlists(response: &Response) -> Vec<Playlist> {
    let mut playlists: Vec<Playlist> = Vec::new();
    for (key, value) in response.pairs() {
        match key.as_str() {
            "playlist" => playlists.push(Playlist {
                name: Arc::from(value.as_str()),
                last_modified: None,
            }),
            "Last-Modified" => {
                if let Some(playlist) = playlists.last_mut() {
                    playlist.last_modified = Some(Arc::from(value.as_str()));
                }
            }
            _ => {}
        }
    }
    playlists
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(pairs: &[(&str, &str)]) -> Response {
        Response::from_pairs(
            pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        )
    }

    #[test]
    fn parses_songs() {
        let response = response(&[
            ("file", "a/one.flac"),
            ("Artist", "Portishead"),
            ("Album", "Dummy"),
            ("Track", "3/10"),
            ("Time", "206"),
            ("duration", "206.123"),
            ("Pos", "0"),
            ("Id", "7"),
            ("MUSICBRAINZ_TRACKID", "abc"),
            ("file", "a/two.flac"),
            ("Pos", "1"),
            ("Id", "8"),
        ]);

        let songs = songs(&response);
        assert_eq!(songs.len(), 2);
        assert_eq!(songs[0].display_title(), "one");
        assert_eq!(songs[0].artist.as_deref(), Some("Portishead"));
        assert_eq!(songs[0].track, Some(3));
        assert_eq!(songs[0].pos, Some(0));
        assert_eq!(songs[0].id, Some(7));
        assert_eq!(songs[0].duration, Some(Duration::from_millis(206_123)));
        assert_eq!(songs[0].tags.len(), 1);
        assert_eq!(songs[0].tags[0].0.as_ref(), "MUSICBRAINZ_TRACKID");
        assert_eq!(songs[1].id, Some(8));
    }

    #[test]
    fn parses_browser_entries() {
        let response = response(&[
            ("directory", "Albums"),
            ("directory", "Live"),
            ("playlist", "Late Night.m3u"),
            ("file", "Albums/One/track.flac"),
            ("Artist", "Portishead"),
            ("directory", "Albums/Two"),
        ]);
        let entries = entries(&response);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[0].kind, EntryKind::Directory);
        assert_eq!(entries[0].name.as_ref(), "Albums");
        assert_eq!(entries[2].kind, EntryKind::Playlist);
        assert_eq!(entries[3].kind, EntryKind::File);
        assert_eq!(entries[3].name.as_ref(), "track.flac");
        assert_eq!(
            entries[3].song.as_ref().unwrap().artist.as_deref(),
            Some("Portishead")
        );
        assert_eq!(entries[4].name.as_ref(), "Two");
    }

    #[test]
    fn parses_status() {
        let response = response(&[
            ("volume", "87"),
            ("repeat", "1"),
            ("random", "0"),
            ("single", "1"),
            ("consume", "0"),
            ("playlist", "3"),
            ("playlistlength", "82"),
            ("state", "play"),
            ("song", "0"),
            ("songid", "1"),
            ("elapsed", "112.008"),
            ("duration", "366.893"),
            ("bitrate", "128"),
            ("audio", "48000:16:2"),
            ("xfade", "5"),
        ]);
        let status = status(&response);
        assert_eq!(status.volume, Some(87));
        assert!(status.repeat);
        assert!(!status.random);
        assert!(status.single);
        assert_eq!(status.playlist_version, Some(3));
        assert_eq!(status.playlist_length, 82);
        assert_eq!(status.state, PlaybackState::Playing);
        assert_eq!(status.song_pos, Some(0));
        assert_eq!(status.elapsed, Duration::from_millis(112_008));
        assert_eq!(status.duration, Some(Duration::from_millis(366_893)));
        assert_eq!(
            status.audio,
            Some(AudioFormat {
                sample_rate: 48_000,
                bits: 16,
                channels: 2
            })
        );
        assert_eq!(status.crossfade, Some(5));
        assert_eq!(status.progress(), Some(112_008.0 / 366_893.0));
    }

    #[test]
    fn parses_stopped_status_with_legacy_time() {
        let response = response(&[("state", "stop"), ("time", "10:200")]);
        let status = status(&response);
        assert_eq!(status.state, PlaybackState::Stopped);
        assert_eq!(status.elapsed, Duration::from_secs(10));
        assert_eq!(status.duration, Some(Duration::from_secs(200)));
    }

    #[test]
    fn parses_outputs() {
        let response = response(&[
            ("outputid", "0"),
            ("outputname", "PipeWire"),
            ("outputenabled", "1"),
            ("outputid", "1"),
            ("outputname", "FIFO"),
            ("outputenabled", "0"),
        ]);
        let outputs = outputs(&response);
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs[0].name.as_ref(), "PipeWire");
        assert!(outputs[0].enabled);
        assert_eq!(outputs[1].name.as_ref(), "FIFO");
        assert!(!outputs[1].enabled);
    }

    #[test]
    fn parses_partitions() {
        let response = response(&[("partition", "default"), ("partition", "headphones")]);
        assert_eq!(partitions(&response), vec!["default", "headphones"]);
    }

    #[test]
    fn parses_stats_and_playlists() {
        let stats = stats(&response(&[
            ("artists", "944"),
            ("albums", "1269"),
            ("songs", "5955"),
            ("uptime", "3600"),
            ("db_playtime", "1383933"),
            ("db_update", "1763138216"),
        ]));
        assert_eq!(stats.artists, 944);
        assert_eq!(stats.songs, 5955);
        assert_eq!(stats.uptime, Duration::from_secs(3600));
        assert_eq!(stats.db_update, Some(1_763_138_216));

        let playlists = playlists(&response(&[
            ("playlist", "Late Night"),
            ("Last-Modified", "2026-11-14T17:00:00Z"),
        ]));
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].name.as_ref(), "Late Night");
        assert!(playlists[0].last_modified.is_some());
    }
}
