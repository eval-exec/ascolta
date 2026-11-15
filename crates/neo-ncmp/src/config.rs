use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mpd: MpdConfig,
    pub ui: UiConfig,
    pub library: LibraryConfig,
    pub visualizer: VisualizerConfig,
    pub scrobbling: ScrobblingConfig,
    pub stats: StatsConfig,
    pub search: SearchConfig,
    pub artwork: ArtworkConfig,
    pub ipc: IpcConfig,
    pub layout: LayoutConfig,
    /// `[keys.<context>]` tables: key spec -> action name.
    pub keys: HashMap<String, HashMap<String, String>>,
    pub theme: ThemeConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MpdConfig {
    pub host: String,
    pub port: u16,
    /// Reserved for servers that require `password` before other commands.
    pub password: Option<String>,
}

impl Default for MpdConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 6600,
            password: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Event loop tick, which also drives the progress bar interpolation.
    pub tick_rate_ms: u64,
    /// One of: queue, library, outputs, stats.
    pub start_screen: String,
    /// Keep the queue cursor on the playing song as tracks change.
    pub follow_now_playing: bool,
    /// Show a desktop notification on track change.
    pub notifications: bool,
    /// Restore the screen, cursors, and lyrics offset on start.
    pub restore_session: bool,
    /// Capture mouse events: wheel scrolls, clicking a tab switches screens,
    /// clicking a row selects it, and double-clicking activates it.
    pub mouse: bool,
    /// Fetch lyrics for the current song so the Now Playing card can show them.
    pub lyrics_in_card: bool,
    /// Draw small album/artist thumbnails at the start of list rows.
    pub inline_art: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            tick_rate_ms: 250,
            start_screen: "queue".into(),
            follow_now_playing: false,
            notifications: false,
            restore_session: true,
            mouse: false,
            lyrics_in_card: true,
            inline_art: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LibraryConfig {
    /// Root of MPD's music directory. Used for local `.lrc` files and, later,
    /// tag editing. Optional; remote lyrics still work without it.
    pub music_directory: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct VisualizerConfig {
    /// `fifo` (requires an MPD fifo output) or `pipewire` (captures the default sink).
    pub enabled: bool,
    pub source: String,
    pub fifo: PathBuf,
    /// Optional PipeWire sink name; defaults to the current default sink.
    pub target: Option<String>,
    pub sample_rate: u32,
    pub channels: u8,
    pub bits: u8,
    pub bars: usize,
}

impl Default for VisualizerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            source: "fifo".into(),
            fifo: PathBuf::from("/tmp/mpd.fifo"),
            target: None,
            sample_rate: 44_100,
            channels: 2,
            bits: 16,
            bars: 64,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ScrobblingConfig {
    pub enabled: bool,
    pub listenbrainz_token: String,
    pub lastfm_api_key: String,
    pub lastfm_api_secret: String,
    pub lastfm_session_key: String,
}

impl ScrobblingConfig {
    pub fn listenbrainz_enabled(&self) -> bool {
        self.enabled && !self.listenbrainz_token.is_empty()
    }

    pub fn lastfm_enabled(&self) -> bool {
        self.enabled
            && !self.lastfm_api_key.is_empty()
            && !self.lastfm_api_secret.is_empty()
            && !self.lastfm_session_key.is_empty()
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// Library pane percentages; each pane needs at least a few columns.
    pub artists_width: u16,
    pub albums_width: u16,
    pub songs_width: u16,
    /// Width of the playlist list on the Playlists screen.
    pub playlists_width: u16,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            artists_width: 30,
            albums_width: 32,
            songs_width: 38,
            playlists_width: 30,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct IpcConfig {
    /// Listen on a per-user Unix socket for app-specific commands.
    pub enabled: bool,
    /// Override the default runtime-directory socket path.
    pub path: Option<PathBuf>,
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
        }
    }
}

impl IpcConfig {
    pub fn socket_path(&self) -> PathBuf {
        self.path.clone().unwrap_or_else(ipc_socket_path)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ArtworkConfig {
    /// Fetch missing embedded art from the Cover Art Archive.
    pub online: bool,
    /// Fetch artist portraits online (Deezer) for artist rows.
    pub artist_images: bool,
}

impl Default for ArtworkConfig {
    fn default() -> Self {
        Self {
            online: false,
            artist_images: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Named filter queries offered by the command palette as smart playlists.
    pub saved: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StatsConfig {
    /// Record completed plays in the local SQLite history.
    pub enabled: bool,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub bg: String,
    pub fg: String,
    pub dim: String,
    pub accent: String,
    pub green: String,
    pub red: String,
    pub yellow: String,
    pub purple: String,
    pub cyan: String,
    pub border: String,
    pub selected_bg: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            bg: "#1a1b26".into(),
            fg: "#c0caf5".into(),
            dim: "#565f89".into(),
            accent: "#7aa2f7".into(),
            green: "#9ece6a".into(),
            red: "#f7768e".into(),
            yellow: "#e0af68".into(),
            purple: "#bb9af7".into(),
            cyan: "#7dcfff".into(),
            border: "#3b4267".into(),
            selected_bg: "#283457".into(),
        }
    }
}

impl Config {
    /// Loads layered configuration.
    ///
    /// An explicitly supplied path must exist. The default XDG path is
    /// optional: a missing file means "use defaults".
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = match explicit {
            Some(path) => Some(path.to_path_buf()),
            None => default_config_path(),
        };

        let mut config = match path {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("could not read {}", path.display()))?;
                toml::from_str(&text)
                    .with_context(|| format!("invalid configuration in {}", path.display()))?
            }
            Some(path) if explicit.is_some() => {
                bail!("configuration file does not exist: {}", path.display())
            }
            _ => Self::default(),
        };

        config.apply_environment();
        Ok(config)
    }

    /// Environment overrides, convenient for tests and one-off invocations.
    pub fn apply_environment(&mut self) {
        if let Ok(host) = std::env::var("NEO_NCMP_MPD_HOST")
            && !host.is_empty()
        {
            self.mpd.host = host;
        }
        if let Ok(port) = std::env::var("NEO_NCMP_MPD_PORT")
            && let Ok(port) = port.parse()
        {
            self.mpd.port = port;
        }
        if let Ok(token) = std::env::var("NEO_NCMP_LISTENBRAINZ_TOKEN") {
            self.scrobbling.listenbrainz_token = token;
        }
        if let Ok(key) = std::env::var("NEO_NCMP_LASTFM_API_KEY") {
            self.scrobbling.lastfm_api_key = key;
        }
        if let Ok(secret) = std::env::var("NEO_NCMP_LASTFM_API_SECRET") {
            self.scrobbling.lastfm_api_secret = secret;
        }
        if let Ok(session) = std::env::var("NEO_NCMP_LASTFM_SESSION_KEY") {
            self.scrobbling.lastfm_session_key = session;
        }
    }

    pub fn mpd_addr(&self) -> String {
        format!("{}:{}", self.mpd.host, self.mpd.port)
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "eval-exec", "neo-ncmp")
        .map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Per-user runtime socket, private to the current user.
pub fn ipc_socket_path() -> PathBuf {
    ProjectDirs::from("dev", "eval-exec", "neo-ncmp")
        .and_then(|dirs| dirs.runtime_dir().map(|dir| dir.join("socket")))
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("neo-ncmp-{}.sock", unsafe { libc::getuid() }))
        })
}

pub fn cache_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "eval-exec", "neo-ncmp").map(|dirs| dirs.cache_dir().to_path_buf())
}

pub fn state_dir() -> Option<PathBuf> {
    ProjectDirs::from("dev", "eval-exec", "neo-ncmp")
        .and_then(|dirs| dirs.state_dir().map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_usable() {
        let config = Config::default();
        assert_eq!(config.mpd_addr(), "127.0.0.1:6600");
        assert_eq!(config.ui.tick_rate_ms, 250);
        assert!(!config.ui.mouse);
        assert!(config.ui.lyrics_in_card);
        assert!(config.ui.inline_art);
        assert!(!config.artwork.online);
        assert!(config.artwork.artist_images);
    }

    #[test]
    fn parses_toml_overrides() {
        let config: Config = toml::from_str(
            r#"
            [mpd]
            host = "music.local"
            port = 7777

            [ui]
            tick_rate_ms = 100
            start_screen = "library"
            mouse = true
            lyrics_in_card = false

            [library]
            music_directory = "/home/me/Music"

            [visualizer]
            enabled = true
            fifo = "/tmp/test.fifo"
            bars = 32

            [scrobbling]
            enabled = true
            listenbrainz_token = "token"

            [keys.queue]
            "d" = "play_selected"

            [keys.global]
            "ctrl+j" = "next_track"

            [search.saved]
            "90s" = "year:1990-1999"

            [ipc]
            enabled = false
            path = "/tmp/neo-ncmp-test.sock"

            [artwork]
            online = true
            artist_images = false

            [layout]
            artists_width = 25
            albums_width = 35
            songs_width = 40

            [theme]
            accent = '#ff00ff'
            "#,
        )
        .unwrap();
        assert_eq!(config.mpd_addr(), "music.local:7777");
        assert_eq!(config.ui.start_screen, "library");
        assert!(!config.ui.follow_now_playing);
        assert_eq!(
            config.library.music_directory.as_deref(),
            Some(std::path::Path::new("/home/me/Music"))
        );
        assert!(config.visualizer.enabled);
        assert_eq!(config.visualizer.bars, 32);
        assert_eq!(
            config.search.saved.get("90s").map(String::as_str),
            Some("year:1990-1999")
        );
        assert!(!config.ipc.enabled);
        assert_eq!(
            config.ipc.socket_path(),
            std::path::PathBuf::from("/tmp/neo-ncmp-test.sock")
        );
        assert!(config.ui.mouse);
        assert!(!config.ui.lyrics_in_card);
        assert!(config.artwork.online);
        assert!(!config.artwork.artist_images);
        assert_eq!(config.layout.songs_width, 40);
        assert!(config.scrobbling.listenbrainz_enabled());
        assert!(!config.scrobbling.lastfm_enabled());
        assert_eq!(config.keys["queue"]["d"], "play_selected");
        assert_eq!(config.keys["global"]["ctrl+j"], "next_track");
        assert_eq!(config.theme.accent, "#ff00ff");
        assert_eq!(config.theme.bg, "#1a1b26");
    }

    #[test]
    fn rejects_unknown_types() {
        assert!(toml::from_str::<Config>("[mpd]\nport = \"six thousand\"").is_err());
    }
}
