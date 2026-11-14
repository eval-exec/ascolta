//! ListenBrainz and Last.fm scrobbling.
//!
//! Events are sent by a background task. Failed scrobbles are appended to a
//! JSONL queue in the XDG state directory and retried on the next start.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc;

use crate::config::ScrobblingConfig;
use neo_ncmp_domain::Song;

const LISTENBRAINZ_URL: &str = "https://api.listenbrainz.org/1/submit-listens";
const LASTFM_URL: &str = "https://ws.audioscrobbler.com/2.0/";
const USER_AGENT: &str = "neo-ncmp/0.1.0 (https://github.com/eval-exec/neo-ncmp)";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScrobbleKind {
    NowPlaying,
    Single,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrobbleTrack {
    pub artist: String,
    pub title: String,
    pub album: String,
    pub duration_seconds: Option<u64>,
}

impl ScrobbleTrack {
    pub fn from_song(song: &Song) -> Self {
        Self {
            artist: song.display_artist().to_owned(),
            title: song.display_title().to_owned(),
            album: song.display_album().to_owned(),
            duration_seconds: song.duration.map(|duration| duration.as_secs()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrobbleEvent {
    pub kind: ScrobbleKind,
    pub track: ScrobbleTrack,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Service {
    ListenBrainz,
    LastFm,
}

impl Service {
    fn name(self) -> &'static str {
        match self {
            Self::ListenBrainz => "listenbrainz",
            Self::LastFm => "lastfm",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "listenbrainz" => Some(Self::ListenBrainz),
            "lastfm" => Some(Self::LastFm),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct ScrobbleHandle {
    tx: mpsc::UnboundedSender<ScrobbleEvent>,
}

impl ScrobbleHandle {
    pub fn send(&self, event: ScrobbleEvent) {
        let _ = self.tx.send(event);
    }

    #[cfg(test)]
    pub(crate) fn test_channel() -> (Self, mpsc::UnboundedReceiver<ScrobbleEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

pub fn spawn(config: ScrobblingConfig) -> ScrobbleHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(run(config, rx));
    ScrobbleHandle { tx }
}

async fn run(config: ScrobblingConfig, mut rx: mpsc::UnboundedReceiver<ScrobbleEvent>) {
    let client = match reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            tracing::warn!(%error, "could not create scrobbling client");
            return;
        }
    };

    flush_queue(&client, &config).await;

    while let Some(event) = rx.recv().await {
        for service in enabled_services(&config) {
            if let Err(error) = send(&client, &config, service, &event).await {
                tracing::warn!(%error, service = service.name(), "scrobble failed");
                if event.kind == ScrobbleKind::Single {
                    persist_failure(service, &event);
                }
            }
        }
    }
}

fn enabled_services(config: &ScrobblingConfig) -> Vec<Service> {
    let mut services = Vec::new();
    if config.listenbrainz_enabled() {
        services.push(Service::ListenBrainz);
    }
    if config.lastfm_enabled() {
        services.push(Service::LastFm);
    }
    services
}

async fn send(
    client: &reqwest::Client,
    config: &ScrobblingConfig,
    service: Service,
    event: &ScrobbleEvent,
) -> Result<()> {
    match service {
        Service::ListenBrainz => send_listenbrainz(client, config, event).await,
        Service::LastFm => send_lastfm(client, config, event).await,
    }
}

async fn send_listenbrainz(
    client: &reqwest::Client,
    config: &ScrobblingConfig,
    event: &ScrobbleEvent,
) -> Result<()> {
    let response = client
        .post(LISTENBRAINZ_URL)
        .header(
            "Authorization",
            format!("Token {}", config.listenbrainz_token),
        )
        .json(&listenbrainz_payload(event))
        .send()
        .await
        .context("ListenBrainz request failed")?;
    if !response.status().is_success() {
        bail!("ListenBrainz returned {}", response.status());
    }
    Ok(())
}

fn listenbrainz_payload(event: &ScrobbleEvent) -> serde_json::Value {
    let mut metadata = json!({
        "artist_name": event.track.artist,
        "track_name": event.track.title,
        "release_name": event.track.album,
    });
    if let Some(duration) = event.track.duration_seconds {
        metadata["additional_info"] = json!({ "duration_ms": duration * 1000 });
    }
    match event.kind {
        ScrobbleKind::NowPlaying => json!({
            "listen_type": "playing_now",
            "payload": [{ "track_metadata": metadata }],
        }),
        ScrobbleKind::Single => json!({
            "listen_type": "single",
            "payload": [{
                "listened_at": event.timestamp,
                "track_metadata": metadata,
            }],
        }),
    }
}

async fn send_lastfm(
    client: &reqwest::Client,
    config: &ScrobblingConfig,
    event: &ScrobbleEvent,
) -> Result<()> {
    let mut params: BTreeMap<String, String> = BTreeMap::new();
    params.insert(
        "method".into(),
        match event.kind {
            ScrobbleKind::NowPlaying => "track.updateNowPlaying",
            ScrobbleKind::Single => "track.scrobble",
        }
        .into(),
    );
    params.insert("api_key".into(), config.lastfm_api_key.clone());
    params.insert("sk".into(), config.lastfm_session_key.clone());
    params.insert("artist".into(), event.track.artist.clone());
    params.insert("track".into(), event.track.title.clone());
    if !event.track.album.is_empty() {
        params.insert("album".into(), event.track.album.clone());
    }
    if let Some(duration) = event.track.duration_seconds {
        params.insert("duration".into(), duration.to_string());
    }
    if event.kind == ScrobbleKind::Single {
        params.insert("timestamp".into(), event.timestamp.to_string());
    }
    let signature = lastfm_signature(&params, &config.lastfm_api_secret);
    params.insert("api_sig".into(), signature);
    params.insert("format".into(), "json".into());

    let response = client
        .post(LASTFM_URL)
        .form(&params)
        .send()
        .await
        .context("Last.fm request failed")?;
    let status = response.status();
    let body: serde_json::Value = response.json().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Last.fm returned {status}");
    }
    if let Some(code) = body.get("error") {
        bail!(
            "Last.fm error {code}: {}",
            body.get("message")
                .and_then(|message| message.as_str())
                .unwrap_or("unknown error")
        );
    }
    Ok(())
}

/// MD5 over alphabetically sorted `keyvalue` pairs, followed by the secret.
pub fn lastfm_signature(params: &BTreeMap<String, String>, secret: &str) -> String {
    let mut input = String::new();
    for (key, value) in params {
        input.push_str(key);
        input.push_str(value);
    }
    input.push_str(secret);
    hex_digest(&Md5::digest(input.as_bytes()))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[derive(Debug, Serialize, Deserialize)]
struct QueuedFailure {
    service: String,
    event: ScrobbleEvent,
}

fn queue_path() -> Option<PathBuf> {
    Some(crate::config::state_dir()?.join("scrobble-queue.jsonl"))
}

fn persist_failure(service: Service, event: &ScrobbleEvent) {
    let Some(path) = queue_path() else {
        return;
    };
    let queued = QueuedFailure {
        service: service.name().to_owned(),
        event: event.clone(),
    };
    let Ok(line) = serde_json::to_string(&queued) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

async fn flush_queue(client: &reqwest::Client, config: &ScrobblingConfig) {
    let Some(path) = queue_path() else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return;
    };
    if contents.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
        return;
    }

    let mut remaining = Vec::new();
    for line in contents.lines() {
        let Ok(queued) = serde_json::from_str::<QueuedFailure>(line) else {
            continue;
        };
        let Some(service) = Service::parse(&queued.service) else {
            continue;
        };
        if let Err(error) = send(client, config, service, &queued.event).await {
            tracing::debug!(%error, "queued scrobble still failing");
            remaining.push(line.to_owned());
        }
    }

    if remaining.is_empty() {
        let _ = std::fs::remove_file(&path);
    } else {
        let _ = std::fs::write(&path, remaining.join("\n") + "\n");
    }
}

/// The standard scrobble threshold: half the track, capped at four minutes.
pub fn scrobble_threshold(duration: Option<Duration>) -> Duration {
    duration
        .map(|duration| (duration / 2).min(Duration::from_secs(240)))
        .unwrap_or(Duration::from_secs(240))
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: ScrobbleKind) -> ScrobbleEvent {
        ScrobbleEvent {
            kind,
            track: ScrobbleTrack {
                artist: "Artist".into(),
                title: "Title".into(),
                album: "Album".into(),
                duration_seconds: Some(200),
            },
            timestamp: 1_700_000_000,
        }
    }

    #[test]
    fn listenbrainz_payloads_have_the_right_shape() {
        let playing = listenbrainz_payload(&event(ScrobbleKind::NowPlaying));
        assert_eq!(playing["listen_type"], "playing_now");
        assert_eq!(
            playing["payload"][0]["track_metadata"]["artist_name"],
            "Artist"
        );

        let single = listenbrainz_payload(&event(ScrobbleKind::Single));
        assert_eq!(single["listen_type"], "single");
        assert_eq!(single["payload"][0]["listened_at"], 1_700_000_000u64);
        assert_eq!(
            single["payload"][0]["track_metadata"]["additional_info"]["duration_ms"],
            200_000
        );
    }

    #[test]
    fn lastfm_signature_sorts_keys_and_appends_the_secret() {
        let mut params = BTreeMap::new();
        params.insert("b".into(), "2".into());
        params.insert("a".into(), "1".into());
        let expected = hex_digest(&Md5::digest(b"a1b2secret"));
        assert_eq!(lastfm_signature(&params, "secret"), expected);
        assert_eq!(lastfm_signature(&params, "secret").len(), 32);
    }

    #[test]
    fn thresholds_follow_the_half_or_four_minute_rule() {
        assert_eq!(
            scrobble_threshold(Some(Duration::from_secs(100))),
            Duration::from_secs(50)
        );
        assert_eq!(
            scrobble_threshold(Some(Duration::from_secs(1_000))),
            Duration::from_secs(240)
        );
        assert_eq!(scrobble_threshold(None), Duration::from_secs(240));
    }
}
