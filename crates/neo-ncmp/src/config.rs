use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mpd: MpdConfig,
    pub ui: UiConfig,
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
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            tick_rate_ms: 250,
            start_screen: "queue".into(),
            follow_now_playing: false,
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
    }

    pub fn mpd_addr(&self) -> String {
        format!("{}:{}", self.mpd.host, self.mpd.port)
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "eval-exec", "neo-ncmp")
        .map(|dirs| dirs.config_dir().join("config.toml"))
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
            "#,
        )
        .unwrap();
        assert_eq!(config.mpd_addr(), "music.local:7777");
        assert_eq!(config.ui.start_screen, "library");
        assert!(!config.ui.follow_now_playing);
    }

    #[test]
    fn rejects_unknown_types() {
        assert!(toml::from_str::<Config>("[mpd]\nport = \"six thousand\"").is_err());
    }
}
