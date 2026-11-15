//! Online cover-art fallback via MusicBrainz and the Cover Art Archive.
//!
//! MPD's `readpicture`/`albumart` only see art embedded in the file or stored
//! next to it. When both are absent and `[artwork] online` is enabled, we search
//! MusicBrainz for the release, fetch its front cover, and keep a small on-disk
//! cache so repeated plays do not hit the network.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use ascolta_mpd::Picture;
use md5::{Digest, Md5};
use serde::Deserialize;
use tokio::sync::{Mutex, mpsc};

use crate::mpd_worker::Msg;

const USER_AGENT: &str = concat!(
    "ascolta/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/eval-exec/ascolta)"
);
const MAX_COVER_BYTES: usize = 5 * 1024 * 1024;
/// Failed lookups are remembered for a week to avoid querying every play.
const NEGATIVE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const MUSICBRAINZ_SPACING: Duration = Duration::from_millis(1100);

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static MB_LIMITER: OnceLock<Mutex<Instant>> = OnceLock::new();

#[derive(Debug, Clone, Default)]
pub struct Request {
    pub uri: String,
    pub artist: String,
    pub album: String,
    pub title: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseSearch {
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct RecordingSearch {
    #[serde(default)]
    recordings: Vec<Recording>,
}

#[derive(Debug, Deserialize)]
struct Recording {
    #[serde(default)]
    score: Option<u32>,
    #[serde(default)]
    releases: Vec<Release>,
}

#[derive(Debug, Deserialize)]
struct Release {
    id: String,
    #[serde(default)]
    score: Option<u32>,
}

/// Starts a background lookup. The result always arrives as one `OnlineCover`.
pub fn spawn(request: Request, tx: mpsc::UnboundedSender<Msg>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        let picture = match fetch(&request).await {
            Ok(picture) => picture,
            Err(error) => {
                tracing::debug!(%error, uri = %request.uri, "online cover lookup failed");
                None
            }
        };
        let _ = tx.send(Msg::OnlineCover {
            uri: request.uri,
            picture,
        });
    });
}

async fn fetch(request: &Request) -> anyhow::Result<Option<Picture>> {
    let cache = default_cache();
    let key = cache_key(request);
    if let Some(cache) = &cache {
        if let Some(picture) = cache.load(&key) {
            tracing::debug!(uri = %request.uri, "online cover cache hit");
            return Ok(Some(picture));
        }
        if cache.recent_miss(&key) {
            tracing::debug!(uri = %request.uri, "online cover miss is cached");
            return Ok(None);
        }
    }

    let Some(release) = search_release(request).await? else {
        if let Some(cache) = &cache {
            cache.mark_miss(&key);
        }
        return Ok(None);
    };
    let picture = fetch_front_cover(&release).await?;
    if let Some(cache) = &cache {
        match &picture {
            Some(picture) => cache.store(&key, picture),
            None => cache.mark_miss(&key),
        }
    }
    Ok(picture)
}

async fn search_release(request: &Request) -> anyhow::Result<Option<String>> {
    let artist = lucene_term(&request.artist);
    let album = lucene_term(&request.album);
    let title = lucene_term(&request.title);

    if !album.is_empty() && !artist.is_empty() {
        let query = format!("artist:\"{artist}\" AND release:\"{album}\"");
        throttle_musicbrainz().await;
        let search: ReleaseSearch = search_json("release", &query).await?;
        if let Some(release) = best_release(search.releases) {
            return Ok(Some(release));
        }
    }

    if !title.is_empty() && !artist.is_empty() {
        let query = format!("artist:\"{artist}\" AND recording:\"{title}\"");
        throttle_musicbrainz().await;
        let search: RecordingSearch = search_json("recording", &query).await?;
        if let Some(release) = search
            .recordings
            .into_iter()
            .find(|recording| recording.score.unwrap_or(0) >= 70)
            .and_then(|recording| recording.releases.into_iter().next())
        {
            return Ok(Some(release.id));
        }
    }

    Ok(None)
}

fn best_release(releases: Vec<Release>) -> Option<String> {
    releases
        .into_iter()
        .find(|release| release.score.unwrap_or(0) >= 70)
        .map(|release| release.id)
}

async fn search_json<T: for<'de> Deserialize<'de>>(entity: &str, query: &str) -> anyhow::Result<T> {
    let response = client()
        .get(format!("https://musicbrainz.org/ws/2/{entity}/"))
        .query(&[("query", query), ("fmt", "json"), ("limit", "1")])
        .send()
        .await?
        .error_for_status()?;
    Ok(response.json().await?)
}

async fn fetch_front_cover(release: &str) -> anyhow::Result<Option<Picture>> {
    let response = client()
        .get(format!(
            "https://coverartarchive.org/release/{release}/front-500"
        ))
        .send()
        .await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if !mime.starts_with("image/") {
        return Ok(None);
    }
    let bytes = response.bytes().await?;
    if bytes.is_empty() || bytes.len() > MAX_COVER_BYTES || !image_magic(&bytes) {
        return Ok(None);
    }
    Ok(Some(Picture {
        mime: Arc::from(mime),
        data: bytes.to_vec(),
    }))
}

async fn throttle_musicbrainz() {
    let limiter = MB_LIMITER.get_or_init(|| Mutex::new(Instant::now() - MUSICBRAINZ_SPACING));
    let mut last = limiter.lock().await;
    let elapsed = last.elapsed();
    if elapsed < MUSICBRAINZ_SPACING {
        tokio::time::sleep(MUSICBRAINZ_SPACING - elapsed).await;
    }
    *last = Instant::now();
}

fn client() -> &'static reqwest::Client {
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(12))
            .build()
            .expect("valid HTTP client configuration")
    })
}

fn default_cache() -> Option<Cache> {
    crate::config::cache_dir().map(|dir| Cache::new(dir.join("covers")))
}

/// Stable filename for an album (or a track when the album is unknown).
fn cache_key(request: &Request) -> String {
    let basis = if request.album.is_empty() {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            request.artist.to_lowercase(),
            request.album.to_lowercase(),
            request.title.to_lowercase()
        )
    } else {
        format!(
            "{}\u{1f}{}",
            request.artist.to_lowercase(),
            request.album.to_lowercase()
        )
    };
    let digest = Md5::digest(basis.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// MusicBrainz treats quotes and backslashes specially; keep phrases readable.
fn lucene_term(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character == '"' || character == '\\' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn image_magic(data: &[u8]) -> bool {
    data.starts_with(&[0xFF, 0xD8, 0xFF])
        || data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || data.starts_with(b"GIF8")
        || (data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP"))
}

struct Cache {
    root: PathBuf,
}

impl Cache {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, key: &str, extension: &str) -> PathBuf {
        self.root.join(format!("{key}.{extension}"))
    }

    fn load(&self, key: &str) -> Option<Picture> {
        let mime = fs::read_to_string(self.path(key, "mime")).ok()?;
        let mime = mime.trim();
        if !mime.starts_with("image/") {
            return None;
        }
        let data = fs::read(self.path(key, "img")).ok()?;
        if data.is_empty() || data.len() > MAX_COVER_BYTES || !image_magic(&data) {
            return None;
        }
        Some(Picture {
            mime: Arc::from(mime),
            data,
        })
    }

    fn store(&self, key: &str, picture: &Picture) {
        if fs::create_dir_all(&self.root).is_err() {
            return;
        }
        write_atomic(&self.path(key, "img"), &picture.data);
        write_atomic(&self.path(key, "mime"), picture.mime.as_bytes());
        let _ = fs::remove_file(self.path(key, "miss"));
    }

    fn recent_miss(&self, key: &str) -> bool {
        fs::metadata(self.path(key, "miss"))
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age < NEGATIVE_TTL)
    }

    fn mark_miss(&self, key: &str) {
        if fs::create_dir_all(&self.root).is_ok() {
            let _ = fs::write(self.path(key, "miss"), []);
        }
    }
}

fn write_atomic(path: &std::path::Path, bytes: &[u8]) {
    let temporary = path.with_extension("tmp");
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(&temporary, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request {
            uri: "Portishead/Dummy/01.flac".into(),
            artist: "Portishead".into(),
            album: "Dummy".into(),
            title: "Mysterons".into(),
        }
    }

    fn temp_cache() -> (Cache, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "ascolta-cover-cache-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        (Cache::new(root.clone()), root)
    }

    #[test]
    fn cache_keys_are_stable_and_album_scoped() {
        let key = cache_key(&request());
        assert_eq!(key, cache_key(&request()));
        let mut other = request();
        other.title = "Sour Times".into();
        assert_eq!(key, cache_key(&other));
        other.album = "Third".into();
        assert_ne!(key, cache_key(&other));
    }

    #[test]
    fn cache_roundtrips_and_records_misses() {
        let (cache, root) = temp_cache();
        let key = cache_key(&request());
        assert!(!cache.recent_miss(&key));
        cache.mark_miss(&key);
        assert!(cache.recent_miss(&key));

        let picture = Picture {
            mime: Arc::from("image/png"),
            data: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01],
        };
        cache.store(&key, &picture);
        let loaded = cache.load(&key).expect("cached picture");
        assert_eq!(loaded.mime.as_ref(), "image/png");
        assert_eq!(loaded.data, picture.data);
        assert!(!cache.recent_miss(&key));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_rejects_non_image_data() {
        let (cache, root) = temp_cache();
        let key = cache_key(&request());
        fs::create_dir_all(&root).unwrap();
        fs::write(cache.path(&key, "mime"), "image/png").unwrap();
        fs::write(cache.path(&key, "img"), b"<html>not art</html>").unwrap();
        assert!(cache.load(&key).is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lucene_terms_are_safely_quoted() {
        assert_eq!(lucene_term(r#"Portis"head"#), "Portis head");
        assert_eq!(lucene_term(r"Massive\Attack"), "Massive Attack");
        assert_eq!(lucene_term("  Dummy  "), "Dummy");
    }

    #[test]
    fn release_search_prefers_confident_scores() {
        let search: ReleaseSearch = serde_json::from_str(
            r#"{"releases":[
                {"id":"weak","score":40},
                {"id":"strong","score":95}
            ]}"#,
        )
        .unwrap();
        assert_eq!(best_release(search.releases).as_deref(), Some("strong"));

        let search: ReleaseSearch =
            serde_json::from_str(r#"{"releases":[{"id":"weak","score":10}]}"#).unwrap();
        assert_eq!(best_release(search.releases), None);
    }

    #[test]
    fn magic_recognises_common_cover_formats() {
        assert!(image_magic(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(image_magic(b"\x89PNG\r\n\x1a\nrest"));
        assert!(image_magic(b"GIF89a"));
        assert!(image_magic(b"RIFF0000WEBP"));
        assert!(!image_magic(b"<html>"));
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_a_known_album_cover() {
        let release = search_release(&request())
            .await
            .unwrap()
            .expect("MusicBrainz release");
        let picture = fetch_front_cover(&release)
            .await
            .unwrap()
            .expect("Cover Art Archive image");
        assert!(picture.mime.starts_with("image/"));
        assert!(picture.data.len() > 1_000);
        assert!(image_magic(&picture.data));
    }
}
