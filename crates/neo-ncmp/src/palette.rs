//! Command palette: a searchable list of global actions.

use crate::input::Action;

#[derive(Debug, Clone, Copy)]
pub struct PaletteItem {
    pub label: &'static str,
    pub action: Action,
}

const ITEMS: &[PaletteItem] = &[
    PaletteItem {
        label: "Play / pause",
        action: Action::TogglePlay,
    },
    PaletteItem {
        label: "Stop",
        action: Action::Stop,
    },
    PaletteItem {
        label: "Next track",
        action: Action::NextTrack,
    },
    PaletteItem {
        label: "Previous track",
        action: Action::PreviousTrack,
    },
    PaletteItem {
        label: "Volume up",
        action: Action::VolumeUp,
    },
    PaletteItem {
        label: "Volume down",
        action: Action::VolumeDown,
    },
    PaletteItem {
        label: "Repeat: toggle",
        action: Action::ToggleRepeat,
    },
    PaletteItem {
        label: "Random: toggle",
        action: Action::ToggleRandom,
    },
    PaletteItem {
        label: "Single: toggle",
        action: Action::ToggleSingle,
    },
    PaletteItem {
        label: "Consume: toggle",
        action: Action::ToggleConsume,
    },
    PaletteItem {
        label: "Go to queue",
        action: Action::GotoScreen(0),
    },
    PaletteItem {
        label: "Go to library",
        action: Action::GotoScreen(1),
    },
    PaletteItem {
        label: "Go to playlists",
        action: Action::GotoScreen(2),
    },
    PaletteItem {
        label: "Go to outputs",
        action: Action::GotoScreen(3),
    },
    PaletteItem {
        label: "Go to server stats",
        action: Action::GotoScreen(4),
    },
    PaletteItem {
        label: "Go to lyrics",
        action: Action::GotoScreen(5),
    },
    PaletteItem {
        label: "Go to visualizer",
        action: Action::GotoScreen(6),
    },
    PaletteItem {
        label: "Go to tags",
        action: Action::GotoScreen(7),
    },
    PaletteItem {
        label: "Go to history",
        action: Action::GotoScreen(8),
    },
    PaletteItem {
        label: "Search library",
        action: Action::OpenSearch,
    },
    PaletteItem {
        label: "Reload everything",
        action: Action::Refresh,
    },
    PaletteItem {
        label: "Help",
        action: Action::ToggleHelp,
    },
    PaletteItem {
        label: "Quit",
        action: Action::Quit,
    },
];

#[derive(Debug, Default)]
pub struct PaletteState {
    pub active: bool,
    pub query: String,
    pub results: Vec<usize>,
    pub cursor: usize,
}

impl PaletteState {
    pub fn open(&mut self) {
        self.active = true;
        self.query.clear();
        self.refresh();
    }

    pub fn close(&mut self) {
        self.active = false;
    }

    pub fn refresh(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.results = ITEMS
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

    pub fn selected(&self) -> Option<PaletteItem> {
        self.results
            .get(self.cursor)
            .and_then(|&index| ITEMS.get(index))
            .copied()
    }

    pub fn result_label(&self, position: usize) -> Option<&'static str> {
        self.results
            .get(position)
            .and_then(|&index| ITEMS.get(index))
            .map(|item| item.label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_commands_case_insensitively() {
        let mut palette = PaletteState::default();
        palette.open();
        assert!(palette.results.len() > 5);
        palette.query = "GO TO LIBR".into();
        palette.refresh();
        assert_eq!(palette.results.len(), 1);
        assert_eq!(palette.selected().unwrap().label, "Go to library");
    }

    #[test]
    fn cursor_is_bounded() {
        let mut palette = PaletteState::default();
        palette.open();
        palette.move_cursor(10_000);
        assert_eq!(palette.cursor, palette.results.len() - 1);
        palette.move_cursor(-10_000);
        assert_eq!(palette.cursor, 0);
    }
}
