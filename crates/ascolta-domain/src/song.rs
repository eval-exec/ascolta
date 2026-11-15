use std::sync::Arc;
use std::time::Duration;

/// The position-independent identifier MPD assigns to a song in the queue.
pub type SongId = u32;

/// A zero-based position in the current queue.
pub type QueuePos = u32;

/// A song as reported by MPD.
///
/// Tags are stored as `Arc<str>` because the same strings repeat constantly
/// across an album or artist; cloning a [`Song`] is therefore cheap.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Song {
    /// Queue id, present for songs in the queue.
    pub id: Option<SongId>,
    /// Queue position, present for songs in the queue.
    pub pos: Option<QueuePos>,
    /// MPD URI (a path relative to the music directory, or a stream URL).
    pub uri: Arc<str>,
    pub title: Option<Arc<str>>,
    pub artist: Option<Arc<str>>,
    pub album: Option<Arc<str>>,
    pub album_artist: Option<Arc<str>>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub date: Option<Arc<str>>,
    pub genre: Option<Arc<str>>,
    pub duration: Option<Duration>,
    pub format: Option<Arc<str>>,
    /// Every non-structural tag, in the order MPD sent it.
    ///
    /// Kept verbatim so the info view and tag editor can display tags this
    /// client does not know about.
    pub tags: Vec<(Arc<str>, Arc<str>)>,
}

impl Song {
    /// Title with a readable fallback to the file name.
    pub fn display_title(&self) -> &str {
        match &self.title {
            Some(title) if !title.is_empty() => title,
            _ => file_name(&self.uri),
        }
    }

    /// Artist with a fallback to the album artist, then a placeholder.
    pub fn display_artist(&self) -> &str {
        match (&self.artist, &self.album_artist) {
            (Some(artist), _) if !artist.is_empty() => artist,
            (_, Some(artist)) if !artist.is_empty() => artist,
            _ => "Unknown Artist",
        }
    }

    /// The artist used to group albums in the library.
    pub fn grouping_artist(&self) -> &str {
        match (&self.album_artist, &self.artist) {
            (Some(artist), _) if !artist.is_empty() => artist,
            (_, Some(artist)) if !artist.is_empty() => artist,
            _ => "Unknown Artist",
        }
    }

    pub fn display_album(&self) -> &str {
        match &self.album {
            Some(album) if !album.is_empty() => album,
            _ => "Unknown Album",
        }
    }

    /// Four-digit year extracted from the `Date` tag, when it looks like one.
    pub fn year(&self) -> Option<&str> {
        let date = self.date.as_deref()?;
        let year = date.get(..4)?;
        year.chars().all(|c| c.is_ascii_digit()).then_some(year)
    }

    /// A stable key for comparisons across queue updates.
    pub fn key(&self) -> (&str, Option<SongId>) {
        (&self.uri, self.id)
    }

    pub fn duration_millis(&self) -> u64 {
        self.duration
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

fn file_name(uri: &str) -> &str {
    uri.rsplit(['/', '\\']).next().unwrap_or(uri)
}

/// Formats a duration the way MPD clients conventionally do: `m:ss`, or
/// `h:mm:ss` for long tracks.
pub fn format_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(uri: &str) -> Song {
        Song {
            uri: Arc::from(uri),
            ..Song::default()
        }
    }

    #[test]
    fn title_falls_back_to_file_name() {
        let song = song("spotdl/Artist/Album/01 - Track.flac");
        assert_eq!(song.display_title(), "01 - Track.flac");
        assert_eq!(song.display_artist(), "Unknown Artist");
        assert_eq!(song.display_album(), "Unknown Album");
    }

    #[test]
    fn artist_prefers_artist_then_album_artist() {
        let mut song = song("a");
        song.album_artist = Some(Arc::from("Various Artists"));
        assert_eq!(song.display_artist(), "Various Artists");
        song.artist = Some(Arc::from("Portishead"));
        assert_eq!(song.display_artist(), "Portishead");
        assert_eq!(song.grouping_artist(), "Various Artists");
    }

    #[test]
    fn year_requires_four_digits() {
        let mut song = song("a");
        song.date = Some(Arc::from("1998-01-01"));
        assert_eq!(song.year(), Some("1998"));
        song.date = Some(Arc::from("98"));
        assert_eq!(song.year(), None);
        song.date = Some(Arc::from("abcd-01-01"));
        assert_eq!(song.year(), None);
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(Duration::from_secs(0)), "0:00");
        assert_eq!(format_duration(Duration::from_secs(67)), "1:07");
        assert_eq!(format_duration(Duration::from_secs(366)), "6:06");
        assert_eq!(format_duration(Duration::from_secs(3_723)), "1:02:03");
    }
}
