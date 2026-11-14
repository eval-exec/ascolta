//! Tag reading and writing via `lofty`.
//!
//! Writing is deliberately conservative: the original file is copied to the
//! XDG state directory before anything is modified. If the copy fails, the
//! write is aborted.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{Accessor, ItemKey, Tag};

#[derive(Debug, Clone, Default)]
pub struct TagUpdate {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub date: String,
    pub genre: String,
    pub comment: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
}

/// Writes `update` to `path`, returning the backup file that was created.
pub fn save(path: &Path, update: &TagUpdate) -> Result<PathBuf, String> {
    if !path.is_file() {
        return Err(format!("file not found: {}", path.display()));
    }
    let backup = backup_path(path)?;
    fs::copy(path, &backup)
        .map_err(|error| format!("could not back up {}: {error}", path.display()))?;

    let mut tagged =
        lofty::read_from_path(path).map_err(|error| format!("could not read tags: {error}"))?;
    let primary = tagged.primary_tag_type();
    if tagged.primary_tag_mut().is_none() {
        tagged.insert_tag(Tag::new(primary));
    }

    {
        let tag = tagged
            .primary_tag_mut()
            .ok_or_else(|| "this file format has no writable tag".to_string())?;

        if update.title.trim().is_empty() {
            tag.remove_title();
        } else {
            tag.set_title(update.title.clone());
        }
        if update.artist.trim().is_empty() {
            tag.remove_artist();
        } else {
            tag.set_artist(update.artist.clone());
        }
        if update.album.trim().is_empty() {
            tag.remove_album();
        } else {
            tag.set_album(update.album.clone());
        }
        if update.album_artist.trim().is_empty() {
            tag.remove_key(ItemKey::AlbumArtist);
        } else {
            tag.insert_text(ItemKey::AlbumArtist, update.album_artist.clone());
        }
        if update.date.trim().is_empty() {
            tag.remove_key(ItemKey::RecordingDate);
        } else {
            tag.insert_text(ItemKey::RecordingDate, update.date.clone());
        }
        if update.genre.trim().is_empty() {
            tag.remove_genre();
        } else {
            tag.set_genre(update.genre.clone());
        }
        if update.comment.trim().is_empty() {
            tag.remove_comment();
        } else {
            tag.set_comment(update.comment.clone());
        }
        match update.track {
            Some(track) => tag.set_track(track),
            None => tag.remove_track(),
        }
        match update.disc {
            Some(disc) => tag.set_disk(disc),
            None => tag.remove_disk(),
        }
    }

    tagged
        .save_to_path(path, WriteOptions::default())
        .map_err(|error| format!("could not write tags: {error}"))?;
    Ok(backup)
}

fn backup_path(path: &Path) -> Result<PathBuf, String> {
    let state = crate::config::state_dir()
        .ok_or_else(|| "could not determine the XDG state directory".to_string())?;
    let directory = state.join("tag-backups");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("track");
    Ok(directory.join(format!("{stamp}-{name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_are_rejected() {
        let error = save(
            Path::new("/definitely/not/a/real/file.mp3"),
            &TagUpdate::default(),
        )
        .unwrap_err();
        assert!(error.contains("file not found"));
    }

    #[test]
    #[ignore = "requires ffmpeg to synthesize a test MP3"]
    fn writes_tags_and_keeps_a_backup() {
        let directory = std::env::temp_dir().join(format!("neo-ncmp-tags-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("track.mp3");
        let status = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=0.2",
                "-q:a",
                "9",
            ])
            .arg(&path)
            .status()
            .expect("ffmpeg should be installed");
        assert!(status.success());

        let original = fs::read(&path).unwrap();
        let update = TagUpdate {
            title: "New Title".into(),
            artist: "New Artist".into(),
            album: "New Album".into(),
            album_artist: "New Album Artist".into(),
            date: "2026".into(),
            genre: "Test".into(),
            comment: "hello".into(),
            track: Some(3),
            disc: Some(1),
        };
        let backup = save(&path, &update).unwrap();
        assert!(backup.exists());
        assert_eq!(fs::read(&backup).unwrap(), original);

        let tagged = lofty::read_from_path(&path).unwrap();
        let tag = tagged.primary_tag().unwrap();
        assert_eq!(tag.title().as_deref(), Some("New Title"));
        assert_eq!(tag.artist().as_deref(), Some("New Artist"));
        assert_eq!(tag.album().as_deref(), Some("New Album"));
        assert_eq!(tag.genre().as_deref(), Some("Test"));
        assert_eq!(tag.track(), Some(3));
        assert_eq!(tag.disk(), Some(1));
        assert_eq!(
            tag.get_string(ItemKey::AlbumArtist),
            Some("New Album Artist")
        );

        let _ = fs::remove_file(backup);
        let _ = fs::remove_dir_all(directory);
    }
}
