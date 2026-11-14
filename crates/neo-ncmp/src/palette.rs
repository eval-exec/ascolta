//! Command palette: a searchable list of global actions and saved searches.

use std::collections::BTreeMap;

use crate::input::Action;

#[derive(Debug, Clone)]
pub enum PaletteAction {
    Action(Action),
    /// Open the search overlay with a saved filter query.
    Search(String),
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub label: String,
    pub action: PaletteAction,
}

const COMMANDS: &[(&str, Action)] = &[
    ("Play / pause", Action::TogglePlay),
    ("Stop", Action::Stop),
    ("Next track", Action::NextTrack),
    ("Previous track", Action::PreviousTrack),
    ("Volume up", Action::VolumeUp),
    ("Volume down", Action::VolumeDown),
    ("Repeat: toggle", Action::ToggleRepeat),
    ("Random: toggle", Action::ToggleRandom),
    ("Single: toggle", Action::ToggleSingle),
    ("Consume: toggle", Action::ToggleConsume),
    ("Go to queue", Action::GotoScreen(0)),
    ("Go to library", Action::GotoScreen(1)),
    ("Go to playlists", Action::GotoScreen(2)),
    ("Go to outputs", Action::GotoScreen(3)),
    ("Go to server stats", Action::GotoScreen(4)),
    ("Go to lyrics", Action::GotoScreen(5)),
    ("Go to visualizer", Action::GotoScreen(6)),
    ("Go to tags", Action::GotoScreen(7)),
    ("Go to history", Action::GotoScreen(8)),
    ("Go to file browser", Action::GotoScreen(9)),
    ("Search library", Action::OpenSearch),
    ("Retry pending scrobbles", Action::RetryScrobbles),
    ("Reload everything", Action::Refresh),
    ("Help", Action::ToggleHelp),
    ("Quit", Action::Quit),
];

#[derive(Debug, Default)]
pub struct PaletteState {
    pub active: bool,
    pub query: String,
    pub results: Vec<usize>,
    pub cursor: usize,
    items: Vec<PaletteItem>,
}

impl PaletteState {
    /// Opens the palette with `[search.saved]` appended as smart playlists.
    pub fn open(&mut self, saved: &BTreeMap<String, String>) {
        self.active = true;
        self.query.clear();
        self.items.clear();
        self.items
            .extend(COMMANDS.iter().map(|(label, action)| PaletteItem {
                label: (*label).to_string(),
                action: PaletteAction::Action(*action),
            }));
        self.items.extend(
            saved
                .iter()
                .filter(|(_, query)| !query.trim().is_empty())
                .map(|(name, query)| PaletteItem {
                    label: format!("Smart playlist: {name}"),
                    action: PaletteAction::Search(query.clone()),
                }),
        );
        self.refresh();
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    pub fn refresh(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.results = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                query.is_empty() || item.label.to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.cursor = 0;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.results.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = self
                .cursor
                .saturating_add_signed(delta)
                .min(self.results.len() - 1);
        }
    }

    pub fn selected(&self) -> Option<&PaletteItem> {
        self.results
            .get(self.cursor)
            .and_then(|&index| self.items.get(index))
    }

    pub fn result_label(&self, position: usize) -> Option<&str> {
        self.results
            .get(position)
            .and_then(|&index| self.items.get(index))
            .map(|item| item.label.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> PaletteState {
        let mut palette = PaletteState::default();
        palette.open(&BTreeMap::new());
        palette
    }

    #[test]
    fn filters_commands_case_insensitively() {
        let mut palette = open();
        assert!(palette.results.len() > 5);
        palette.query = "GO TO LIBR".into();
        palette.refresh();
        assert_eq!(palette.results.len(), 1);
        assert_eq!(palette.selected().unwrap().label, "Go to library");

        palette.query = "retry".into();
        palette.refresh();
        assert_eq!(palette.selected().unwrap().label, "Retry pending scrobbles");
    }

    #[test]
    fn cursor_is_bounded() {
        let mut palette = open();
        palette.move_cursor(10_000);
        assert_eq!(palette.cursor, palette.results.len() - 1);
        palette.move_cursor(-10_000);
        assert_eq!(palette.cursor, 0);
    }

    #[test]
    fn saved_queries_become_smart_playlists() {
        let mut saved = BTreeMap::new();
        saved.insert("90s".into(), "year:1990-1999".into());
        let mut palette = PaletteState::default();
        palette.open(&saved);
        palette.query = "smart".into();
        palette.refresh();
        assert_eq!(palette.results.len(), 1);
        match &palette.selected().unwrap().action {
            PaletteAction::Search(query) => assert_eq!(query, "year:1990-1999"),
            PaletteAction::Action(_) => panic!("expected a saved search"),
        }
    }
}
