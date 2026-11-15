//! Lyrics lookup and LRC parsing.
//!
//! Local `.lrc` files are preferred; when none exists, LRCLIB is queried.
//! Fetching happens on a background task, never on the render thread.

use std::path::Path;
use std::time::Duration;

pub const LRCLIB_URL: &str = "https://lrclib.net/api/get";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsSource {
    Local,
    Lrclib,
}

impl LyricsSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local .lrc",
            Self::Lrclib => "LRCLIB",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    pub at: Duration,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct LyricsDoc {
    pub source: Option<LyricsSource>,
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
}

impl LyricsDoc {
    pub fn is_empty(&self) -> bool {
        self.synced.is_empty()
            && self
                .plain
                .as_deref()
                .is_none_or(|text| text.trim().is_empty())
    }

    /// Index of the last synced line at or before `position`.
    pub fn active_index(&self, position: Duration) -> Option<usize> {
        if self.synced.is_empty() {
            return None;
        }
        let index = self.synced.partition_point(|line| line.at <= position);
        Some(index.saturating_sub(1))
    }
}

pub struct LyricsRequest<'a> {
    pub uri: &'a str,
    pub artist: &'a str,
    pub title: &'a str,
    pub album: &'a str,
    pub duration_seconds: Option<u64>,
    pub music_directory: Option<&'a Path>,
}

/// Resolves lyrics for a track, preferring a local `.lrc` file.
pub async fn fetch(request: LyricsRequest<'_>) -> Option<LyricsDoc> {
    if let Some(directory) = request.music_directory
        && let Some(document) = local_lyrics(directory, request.uri).await
    {
        return Some(document);
    }
    lrclib_lyrics(request).await
}

async fn local_lyrics(directory: &Path, uri: &str) -> Option<LyricsDoc> {
    if uri.contains("://") {
        return None;
    }
    let path = directory.join(uri).with_extension("lrc");
    let text = tokio::fs::read_to_string(path).await.ok()?;
    let document = document_from_lrc(&text, LyricsSource::Local);
    (!document.is_empty()).then_some(document)
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LrclibResponse {
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

async fn lrclib_lyrics(request: LyricsRequest<'_>) -> Option<LyricsDoc> {
    if request.artist.trim().is_empty() || request.title.trim().is_empty() {
        return None;
    }

    let client = reqwest::Client::builder()
        .user_agent("ascolta/0.1.0 (https://github.com/eval-exec/ascolta)")
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;

    let mut query: Vec<(&str, String)> = vec![
        ("artist_name", request.artist.to_owned()),
        ("track_name", request.title.to_owned()),
    ];
    if !request.album.trim().is_empty() {
        query.push(("album_name", request.album.to_owned()));
    }
    if let Some(duration) = request.duration_seconds {
        query.push(("duration", duration.to_string()));
    }

    let response = match client.get(LRCLIB_URL).query(&query).send().await {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(%error, "LRCLIB request failed");
            return None;
        }
    };
    if !response.status().is_success() {
        tracing::debug!(status = %response.status(), "LRCLIB returned no lyrics");
        return None;
    }
    let body: LrclibResponse = match response.json().await {
        Ok(body) => body,
        Err(error) => {
            tracing::debug!(%error, "LRCLIB response was not valid JSON");
            return None;
        }
    };

    let mut document = LyricsDoc::default();
    if let Some(synced) = body.synced_lyrics.filter(|text| !text.trim().is_empty()) {
        document = document_from_lrc(&synced, LyricsSource::Lrclib);
        if !document.synced.is_empty() {
            return Some(document);
        }
    }
    if let Some(plain) = body.plain_lyrics.filter(|text| !text.trim().is_empty()) {
        document.source = Some(LyricsSource::Lrclib);
        document.plain = Some(plain);
        return Some(document);
    }
    None
}

fn document_from_lrc(text: &str, source: LyricsSource) -> LyricsDoc {
    let synced = parse_lrc(text);
    if synced.is_empty() {
        LyricsDoc {
            source: Some(source),
            synced,
            plain: Some(text.to_owned()),
        }
    } else {
        LyricsDoc {
            source: Some(source),
            synced,
            plain: None,
        }
    }
}

/// Parses LRC timestamps `[mm:ss.xx]` (hundredths or milliseconds).
///
/// Lines may carry multiple timestamps, and metadata tags such as `[ar:...]`
/// are ignored. Output is sorted by time.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim_start();
        let mut stamps = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some((tag, after)) = inner.split_once(']') else {
                break;
            };
            if let Some(timestamp) = parse_timestamp(tag) {
                stamps.push(timestamp);
            }
            rest = after.trim_start();
        }
        if stamps.is_empty() {
            continue;
        }
        let lyric = rest.trim().to_owned();
        for at in stamps {
            lines.push(LyricLine {
                at,
                text: lyric.clone(),
            });
        }
    }
    lines.sort_by_key(|line| line.at);
    lines
}

fn parse_timestamp(tag: &str) -> Option<Duration> {
    let (minutes, seconds) = tag.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let seconds: f64 = seconds.replace(':', ".").parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(minutes as f64 * 60.0 + seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timestamps_and_metadata() {
        let text = "\
[ar:Portishead]
[ti:Mysterons]
[00:12.34]First line
[00:15.00][00:40.00]Repeated line
[01:02.5]Last line
";
        let lines = parse_lrc(text);
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0].at, Duration::from_millis(12_340));
        assert_eq!(lines[0].text, "First line");
        assert_eq!(lines[1].at, Duration::from_secs(15));
        assert_eq!(lines[1].text, "Repeated line");
        assert_eq!(lines[2].at, Duration::from_secs(40));
        assert_eq!(lines[3].at, Duration::from_millis(62_500));
        assert_eq!(lines[3].text, "Last line");
    }

    #[test]
    fn lrc_without_timestamps_becomes_plain_text() {
        let document = document_from_lrc("just some words\nand more", LyricsSource::Local);
        assert!(document.synced.is_empty());
        assert_eq!(document.plain.as_deref(), Some("just some words\nand more"));
        assert_eq!(document.source, Some(LyricsSource::Local));
    }

    #[test]
    fn active_line_tracks_position() {
        let document = document_from_lrc(
            "[00:10.00]one\n[00:20.00]two\n[00:30.00]three",
            LyricsSource::Lrclib,
        );
        assert_eq!(
            document.active_index(Duration::from_secs(5)),
            Some(0),
            "before the first line, highlight the first"
        );
        assert_eq!(document.active_index(Duration::from_secs(10)), Some(0));
        assert_eq!(document.active_index(Duration::from_secs(25)), Some(1));
        assert_eq!(document.active_index(Duration::from_secs(99)), Some(2));
    }

    #[test]
    fn parses_lrclib_json_shape() {
        let json = r#"{"syncedLyrics":"[00:01.00]hello","plainLyrics":"hello"}"#;
        let response: LrclibResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.synced_lyrics.as_deref(), Some("[00:01.00]hello"));
        assert_eq!(response.plain_lyrics.as_deref(), Some("hello"));
    }

    #[tokio::test]
    async fn prefers_a_local_lrc_file() {
        let directory = std::env::temp_dir().join(format!("ascolta-lyrics-{}", std::process::id()));
        let track = directory.join("Artist/Album/Track.lrc");
        std::fs::create_dir_all(track.parent().unwrap()).unwrap();
        std::fs::write(&track, "[00:01.00]local line").unwrap();

        let document = fetch(LyricsRequest {
            uri: "Artist/Album/Track.mp3",
            artist: "Artist",
            title: "Track",
            album: "Album",
            duration_seconds: Some(60),
            music_directory: Some(&directory),
        })
        .await
        .unwrap();
        assert_eq!(document.source, Some(LyricsSource::Local));
        assert_eq!(document.synced.len(), 1);
        assert_eq!(document.synced[0].text, "local line");

        let _ = std::fs::remove_dir_all(directory);
    }

    #[tokio::test]
    #[ignore = "requires network access to lrclib.net"]
    async fn fetches_lyrics_from_lrclib() {
        let document = fetch(LyricsRequest {
            uri: "Massive Attack/Mezzanine/06 Dissolved Girl.mp3",
            artist: "Massive Attack",
            title: "Dissolved Girl",
            album: "Mezzanine",
            duration_seconds: Some(367),
            music_directory: None,
        })
        .await;
        println!("{document:#?}");
        let document = document.expect("LRCLIB should know this track");
        assert!(!document.is_empty());
    }

    #[test]
    fn empty_documents_are_empty() {
        assert!(LyricsDoc::default().is_empty());
        assert!(!document_from_lrc("[00:01.00]x", LyricsSource::Lrclib).is_empty());
    }
}
