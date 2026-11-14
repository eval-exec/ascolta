//! Small, human-readable session file.
//!
//! Only UI state is persisted; MPD remains the source of truth for playback
//! and the queue.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionState {
    pub screen: Option<String>,
    pub queue_cursor: usize,
    pub library_pane: usize,
    pub library_artist: usize,
    pub library_album: usize,
    pub library_song: usize,
    pub playlist_cursor: usize,
    pub playlist_pane: usize,
    pub history_cursor: usize,
    pub lyrics_offset_ms: i64,
}

impl SessionState {
    pub fn path() -> Option<PathBuf> {
        Some(crate::config::state_dir()?.join("session.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|path| Self::load_at(&path).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::path()
            && let Err(error) = self.save_at(&path)
        {
            tracing::warn!(%error, "could not save session");
        }
    }

    pub fn load_at(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|error| format!("could not parse {}: {error}", path.display()))
    }

    pub fn save_at(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("could not serialise session: {error}"))?;
        std::fs::write(path, text)
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_session_state() {
        let directory =
            std::env::temp_dir().join(format!("neo-ncmp-session-{}", std::process::id()));
        let path = directory.join("session.json");
        let state = SessionState {
            screen: Some("library".into()),
            queue_cursor: 7,
            library_pane: 2,
            library_artist: 3,
            library_album: 4,
            library_song: 5,
            playlist_cursor: 1,
            playlist_pane: 1,
            history_cursor: 6,
            lyrics_offset_ms: -500,
        };
        state.save_at(&path).unwrap();
        let loaded = SessionState::load_at(&path).unwrap();
        assert_eq!(loaded.screen.as_deref(), Some("library"));
        assert_eq!(loaded.queue_cursor, 7);
        assert_eq!(loaded.library_pane, 2);
        assert_eq!(loaded.lyrics_offset_ms, -500);
        let _ = std::fs::remove_dir_all(directory);
    }
}
