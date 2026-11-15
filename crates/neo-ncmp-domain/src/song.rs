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
    /// Title with a readable fallback derived from the file name.
    ///
    /// Redundant artist prefixes or album suffixes (common in downloader
    /// libraries) are removed so the title column does not repeat the
    /// neighbouring artist/album columns.
    pub fn display_title(&self) -> &str {
        let raw = self
            .title
            .as_deref()
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| self.fallback_title());
        self.clean_title(raw).unwrap_or(raw)
    }

    /// Artist with a fallback to the album artist, inferred path/filename
    /// metadata, then a placeholder.
    pub fn display_artist(&self) -> &str {
        self.artist_name().unwrap_or("Unknown Artist")
    }

    /// The artist used to group albums in the library.
    pub fn grouping_artist(&self) -> &str {
        match (&self.album_artist, &self.artist) {
            (Some(artist), _) if !artist.is_empty() => artist,
            (_, Some(artist)) if !artist.is_empty() => artist,
            _ => self.derived_artist().unwrap_or("Unknown Artist"),
        }
    }

    pub fn display_album(&self) -> &str {
        self.album_name().unwrap_or("Unknown Album")
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

    fn artist_name(&self) -> Option<&str> {
        self.artist
            .as_deref()
            .filter(|artist| !artist.is_empty())
            .or_else(|| {
                self.album_artist
                    .as_deref()
                    .filter(|artist| !artist.is_empty())
            })
            .or_else(|| self.derived_artist())
    }

    fn album_name(&self) -> Option<&str> {
        self.album
            .as_deref()
            .filter(|album| !album.is_empty())
            .or_else(|| self.derived_album())
    }

    fn fallback_title(&self) -> &str {
        let Some(info) = self.uri_info() else {
            return file_name(&self.uri);
        };
        if let Some((filename_artist, filename_title)) = split_filename_artist(info.stem)
            && self
                .artist_name()
                .is_some_and(|artist| artist.eq_ignore_ascii_case(filename_artist))
        {
            return filename_title;
        }
        info.stem
    }

    fn clean_title<'a>(&self, raw: &'a str) -> Option<&'a str> {
        let mut title = strip_noise_suffixes(raw);
        title = strip_track_prefix(title);
        if let Some(artist) = self.artist_name()
            && let Some(stripped) = strip_artist_prefix(title, artist)
        {
            title = stripped;
        }
        if let Some(album) = self.album_name()
            && let Some(stripped) = strip_album_suffix(title, album)
        {
            title = stripped;
        }
        let title = title.trim();
        (!title.is_empty()).then_some(title)
    }

    fn uri_info(&self) -> Option<UriInfo<'_>> {
        let uri = self.uri.as_ref();
        if uri.is_empty() || uri.contains("://") {
            return None;
        }
        let mut parts = uri.rsplit('/');
        let file = parts.next()?;
        if file.is_empty() {
            return None;
        }
        Some(UriInfo {
            stem: audio_stem(file),
            parent: parts.next().filter(|part| !part.is_empty()),
            grandparent: parts.next().filter(|part| !part.is_empty()),
        })
    }

    fn derived_artist(&self) -> Option<&str> {
        let info = self.uri_info()?;
        let file_artist = split_filename_artist(info.stem).map(|(artist, _)| artist);
        if let Some(grandparent) = info.grandparent {
            if meaningful_dir(grandparent) {
                return Some(grandparent);
            }
            if let Some(parent) = info.parent.filter(|part| meaningful_dir(part)) {
                // `spotdl/Artist/Artist - Title.mp3`: the parent is the artist.
                if file_artist.is_none_or(|artist| artist.eq_ignore_ascii_case(parent)) {
                    return Some(parent);
                }
            }
            return file_artist;
        }
        if let Some(parent) = info.parent.filter(|part| meaningful_dir(part)) {
            // With one directory, a filename artist wins (`Album/Artist - Title`).
            if file_artist.is_some() {
                return file_artist;
            }
            return Some(parent);
        }
        file_artist
    }

    fn derived_album(&self) -> Option<&str> {
        let info = self.uri_info()?;
        let parent = info.parent.filter(|part| meaningful_dir(part))?;
        if info.grandparent.is_some_and(meaningful_dir) {
            // `Artist/Album/file`.
            return Some(parent);
        }
        let file_artist = split_filename_artist(info.stem).map(|(artist, _)| artist);
        // `Album/Artist - Title` has an album directory; `Artist/file` does not.
        file_artist
            .filter(|artist| !artist.eq_ignore_ascii_case(parent))
            .map(|_| parent)
    }
}

struct UriInfo<'a> {
    stem: &'a str,
    parent: Option<&'a str>,
    grandparent: Option<&'a str>,
}

fn audio_stem(file: &str) -> &str {
    if let Some((stem, extension)) = file.rsplit_once('.')
        && matches!(
            extension.to_ascii_lowercase().as_str(),
            "mp3"
                | "flac"
                | "m4a"
                | "m4b"
                | "aac"
                | "ogg"
                | "opus"
                | "wav"
                | "wma"
                | "alac"
                | "ape"
                | "wv"
                | "mp4"
                | "webm"
                | "mka"
                | "aiff"
                | "aif"
        )
    {
        return stem;
    }
    file
}

fn meaningful_dir(dir: &str) -> bool {
    dir.chars().count() > 1 && !is_generic_dir(dir)
}

fn is_generic_dir(dir: &str) -> bool {
    const GENERIC: &[&str] = &[
        "spotdl",
        "spotify",
        "downloads",
        "download",
        "yt-dlp",
        "youtube",
        "music",
        "audio",
        "video",
        "videos",
        "mixes",
        "mix",
        "compilation",
        "compilations",
        "unknown",
        "unknown artist",
        "unknown album",
        "new folder",
        "temp",
        "tmp",
    ];
    GENERIC
        .iter()
        .any(|generic| dir.eq_ignore_ascii_case(generic))
}

/// Splits `Artist - Title` filenames, ignoring separators inside a title.
fn split_filename_artist(stem: &str) -> Option<(&str, &str)> {
    let stem = strip_track_prefix(strip_noise_suffixes(stem));
    for separator in [" - ", " – ", " — "] {
        if let Some((first, second)) = stem.split_once(separator) {
            let first = clean_name_part(first);
            let second = clean_name_part(second);
            if let Some(pair) = choose_artist_title(first, second) {
                return Some(pair);
            }
        }
    }
    // Some rips use `Title -[ Artist ] notes`.
    if let Some((first, rest)) = stem.split_once(" -[") {
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('[').unwrap_or(rest);
        if let Some(end) = rest.find(']') {
            let first = clean_name_part(first);
            let second = clean_name_part(&rest[..end]);
            if let Some(pair) = choose_artist_title(first, second) {
                return Some(pair);
            }
        }
    }
    None
}

fn choose_artist_title<'a>(first: &'a str, second: &'a str) -> Option<(&'a str, &'a str)> {
    if first.is_empty()
        || second.is_empty()
        || first.chars().all(|character| character.is_ascii_digit())
    {
        return None;
    }
    // Most files are `Artist - Title`, but some are `Title - Artist`.
    // A comma/ampersand/feature credit or a short `X` collaboration is a
    // strong signal for the artist side.
    let first_artist = looks_like_artist(first);
    let second_artist = looks_like_artist(second);
    if !first_artist && second_artist {
        Some((second, first))
    } else {
        Some((first, second))
    }
}

fn looks_like_artist(value: &str) -> bool {
    if value.contains(',')
        || value.contains('&')
        || value.contains('₰')
        || value.contains(" feat. ")
        || value.contains(" ft. ")
    {
        return true;
    }
    let has_collaboration_x = value.contains(" x ") || value.contains(" X ");
    has_collaboration_x && value.split_whitespace().count() <= 4
}

fn clean_name_part(part: &str) -> &str {
    part.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '"' | '＂' | '“' | '”' | '«' | '»')
    })
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let end = prefix.len();
    if end > 0
        && text.len() >= end
        && text.is_char_boundary(end)
        && text[..end].eq_ignore_ascii_case(prefix)
    {
        Some(&text[end..])
    } else {
        None
    }
}

fn strip_artist_prefix<'a>(title: &'a str, artist: &str) -> Option<&'a str> {
    let rest = strip_prefix_ci(title, artist)?.trim_start();
    let rest = rest.strip_prefix('-')?.trim_start();
    (!rest.is_empty()).then_some(rest)
}

fn strip_album_suffix<'a>(title: &'a str, album: &str) -> Option<&'a str> {
    let (head, tail) = title.rsplit_once(" - ")?;
    let tail = tail.trim();
    let tail = tail
        .strip_prefix('[')
        .and_then(|tail| tail.strip_suffix(']'))
        .or_else(|| {
            tail.strip_prefix('(')
                .and_then(|tail| tail.strip_suffix(')'))
        })
        .unwrap_or(tail);
    if !tail.eq_ignore_ascii_case(album) {
        return None;
    }
    let head = head.trim_end();
    (!head.is_empty()).then_some(head)
}

fn strip_track_prefix(title: &str) -> &str {
    let digits = title
        .as_bytes()
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 || digits > 3 || digits == title.len() {
        return title;
    }
    let rest = title[digits..].trim_start();
    if !rest.starts_with(['-', '.', '_', ')']) {
        return title;
    }
    let rest = rest[1..].trim_start();
    if rest.is_empty() { title } else { rest }
}

fn strip_noise_suffixes(mut title: &str) -> &str {
    loop {
        let trimmed = title.trim_end();
        let Some(last) = trimmed.chars().last() else {
            break;
        };
        let (open, close) = match last {
            ')' => ('(', ')'),
            ']' => ('[', ']'),
            '）' => ('（', '）'),
            '】' => ('【', '】'),
            _ => break,
        };
        let Some(start) = trimmed.rfind(open) else {
            break;
        };
        let inner = &trimmed[start + open.len_utf8()..trimmed.len() - close.len_utf8()];
        if !is_noise_group(inner) {
            break;
        }
        title = &trimmed[..start];
    }
    title.trim_end()
}

fn is_noise_group(inner: &str) -> bool {
    let inner = inner.to_ascii_lowercase();
    [
        "official",
        "lyrics",
        "lyric video",
        "music video",
        "video",
        "audio",
        "visualizer",
        "4k",
        "8k",
        "1080p",
        "720p",
        "mv",
        "m/v",
        "color coded",
        "eng sub",
    ]
    .iter()
    .any(|noise| inner.contains(noise))
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
    fn title_and_artist_fall_back_to_the_path() {
        let song = song("spotdl/Artist/Album/01 - Track.flac");
        assert_eq!(song.display_title(), "Track");
        assert_eq!(song.display_artist(), "Artist");
        assert_eq!(song.display_album(), "Album");
    }

    #[test]
    fn flat_downloader_files_derive_artist_from_the_filename() {
        let flat = song("@Artist - Song (Official Video) [4K].mp3");
        assert_eq!(flat.display_title(), "Song");
        assert_eq!(flat.display_artist(), "@Artist");
        assert_eq!(flat.display_album(), "Unknown Album");

        let phonk = song("15 PHONK in 15 COUNTRIES.mp3");
        assert_eq!(phonk.display_title(), "15 PHONK in 15 COUNTRIES");
        assert_eq!(phonk.display_artist(), "Unknown Artist");

        let collaboration = song("Azaleh & Eikona - In Aeternum.mp3");
        assert_eq!(collaboration.display_title(), "In Aeternum");
        assert_eq!(collaboration.display_artist(), "Azaleh & Eikona");

        let reversed = song("Beggin X Give It To Me - Madcon X Timbaland [Edit Audio].mp3");
        assert_eq!(reversed.display_title(), "Beggin X Give It To Me");
        assert_eq!(reversed.display_artist(), "Madcon X Timbaland");

        let reversed_list =
            song("give it to me - timbaland, nelly furtado & justin timberlake.mp3");
        assert_eq!(reversed_list.display_title(), "give it to me");
        assert_eq!(
            reversed_list.display_artist(),
            "timbaland, nelly furtado & justin timberlake"
        );

        let bracketed = song("Beggin X Give It to me -[ Madcon ₰  Timbaland ] Áudio Edit.mp3");
        assert_eq!(bracketed.display_title(), "Beggin X Give It to me");
        assert_eq!(bracketed.display_artist(), "Madcon ₰  Timbaland");
    }

    #[test]
    fn spotdl_artist_folders_do_not_become_album_names() {
        let song = song("spotdl/ELMAN/ELMAN - Loki.mp3");
        assert_eq!(song.display_artist(), "ELMAN");
        assert_eq!(song.display_title(), "Loki");
        assert_eq!(song.display_album(), "Unknown Album");
    }

    #[test]
    fn redundant_artist_and_album_suffixes_are_trimmed() {
        let mut song = song("spotdl/BIGBANG/Album/01 - BLUE - Album.mp3");
        song.artist = Some(Arc::from("BIGBANG"));
        song.album = Some(Arc::from("Album"));
        song.title = Some(Arc::from("BLUE - Album"));
        assert_eq!(song.display_title(), "BLUE");

        song.title = Some(Arc::from("BIGBANG - FANTASTIC BABY"));
        assert_eq!(song.display_title(), "FANTASTIC BABY");

        song.title = Some(Arc::from("Sour Times"));
        song.album = Some(Arc::from("Dummy"));
        assert_eq!(song.display_title(), "Sour Times");
    }

    #[test]
    fn noise_brackets_and_track_numbers_are_removed() {
        let noisy = song("Song (Official Lyric Video) [1080p].flac");
        assert_eq!(noisy.display_title(), "Song");

        let numbered = song("01. Track Title (Lyrics).flac");
        assert_eq!(numbered.display_title(), "Track Title");

        let nested = song("Album/01 - Artist - Title - Album.mp3");
        assert_eq!(nested.display_title(), "Title");
        assert_eq!(nested.display_artist(), "Artist");
        assert_eq!(nested.display_album(), "Album");
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
