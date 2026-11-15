//! Online artist portraits.
//!
//! MPD only exposes track and album artwork, so artist rows use Deezer's public
//! search API (no key required). Results are cached on disk and misses are
//! remembered briefly, keeping library browsing from hammering the service.

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use neo_ncmp_mpd::Picture;
use serde::Deserialize;
use tokio::sync::{Mutex, mpsc};

use crate::coverart::image_magic;
use crate::mpd_worker::Msg;

const USER_AGENT: &str = "neo-ncmp/0.1.0 (https://github.com/eval-exec/neo-ncmp)";
const SEARCH_URL: &str = "https://api.deezer.com/search/artist";
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const NEGATIVE_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SEARCH_SPACING: Duration = Duration::from_millis(250);

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static LIMITER: OnceLock<Mutex<Instant>> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<ArtistResult>,
}

#[derive(Debug, Deserialize)]
struct ArtistResult {
    name: String,
    #[serde(default)]
    picture_big: String,
    #[serde(default)]
    picture_medium: String,
}

/// Looks up a portrait and reports exactly one result through `Msg`.
pub fn spawn(artist: String, tx: mpsc::UnboundedSender<Msg>) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return;
    };
    runtime.spawn(async move {
        let picture = match fetch(&artist).await {
            Ok(picture) => picture,
            Err(error) => {
                tracing::debug!(%error, artist, "artist image lookup failed");
                None
            }
        };
        let _ = tx.send(Msg::ArtistImage { artist, picture });
    });
}

async fn fetch(artist: &str) -> anyhow::Result<Option<Picture>> {
    let cache = default_cache();
    let key = cache_key(artist);
    if let Some(cache) = &cache {
        if let Some(picture) = cache.load(&key) {
            return Ok(Some(picture));
        }
        if cache.recent_miss(&key) {
            return Ok(None);
        }
    }

    throttle().await;
    let response = client()
        .get(SEARCH_URL)
        .query(&[("q", artist), ("limit", "1")])
        .send()
        .await?
        .error_for_status()?;
    let search: SearchResponse = response.json().await?;
    let Some(url) = portrait_url(search, artist) else {
        if let Some(cache) = &cache {
            cache.mark_miss(&key);
        }
        return Ok(None);
    };

    let response = client().get(url).send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("image/jpeg")
        .split(';')
        .next()
        .unwrap_or("image/jpeg")
        .trim()
        .to_string();
    if !mime.starts_with("image/") {
        return Ok(None);
    }
    let bytes = response.bytes().await?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES || !image_magic(&bytes) {
        return Ok(None);
    }
    let picture = Picture {
        mime: Arc::from(mime),
        data: bytes.to_vec(),
    };
    if let Some(cache) = &cache {
        cache.store(&key, &picture);
    }
    Ok(Some(picture))
}

fn portrait_url(search: SearchResponse, artist: &str) -> Option<String> {
    let result = search
        .data
        .into_iter()
        .find(|result| result.name.eq_ignore_ascii_case(artist))?;
    if result.picture_big.is_empty() {
        Some(result.picture_medium).filter(|url| !url.is_empty())
    } else {
        Some(result.picture_big)
    }
}

async fn throttle() {
    let limiter = LIMITER.get_or_init(|| Mutex::new(Instant::now() - SEARCH_SPACING));
    let mut last = limiter.lock().await;
    let elapsed = last.elapsed();
    if elapsed < SEARCH_SPACING {
        tokio::time::sleep(SEARCH_SPACING - elapsed).await;
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
    crate::config::cache_dir().map(|dir| Cache::new(dir.join("artists")))
}

fn cache_key(artist: &str) -> String {
    let digest = Md5::digest(artist.to_lowercase().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
        if data.is_empty() || data.len() > MAX_IMAGE_BYTES || !image_magic(&data) {
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

    #[test]
    fn cache_keys_are_case_insensitive_and_name_scoped() {
        assert_eq!(cache_key("Portishead"), cache_key("portishead"));
        assert_ne!(cache_key("Portishead"), cache_key("Massive Attack"));
    }

    #[test]
    fn picks_only_exact_artist_matches() {
        let search: SearchResponse = serde_json::from_str(
            r#"{"data":[
                {"name":"Portishead Tribute","picture_big":"wrong","picture_medium":""},
                {"name":"portishead","picture_big":"right","picture_medium":"small"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(portrait_url(search, "Portishead").as_deref(), Some("right"));

        let search: SearchResponse =
            serde_json::from_str(r#"{"data":[{"name":"Other","picture_big":"x"}]}"#).unwrap();
        assert_eq!(portrait_url(search, "Portishead"), None);
    }

    #[tokio::test]
    #[ignore = "network"]
    async fn fetches_a_known_artist_portrait() {
        let picture = fetch("Portishead").await.unwrap().expect("artist portrait");
        assert!(picture.mime.starts_with("image/"));
        assert!(image_magic(&picture.data));
    }
}
