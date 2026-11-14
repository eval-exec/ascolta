use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::Screen;

/// A user intention, independent of the key that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    OpenSearch,
    OpenPalette,
    NextScreen,
    PreviousScreen,
    GotoScreen(usize),
    Refresh,

    MoveDown,
    MoveUp,
    PageDown,
    PageUp,
    First,
    Last,

    TogglePlay,
    Stop,
    NextTrack,
    PreviousTrack,
    VolumeUp,
    VolumeDown,
    SeekForward,
    SeekBackward,
    ToggleRepeat,
    ToggleRandom,
    ToggleSingle,
    ToggleConsume,

    PlaySelected,
    EnqueueSelected,
    ReplaceAndPlay,
    RemoveSelected,
    MoveSelectionDown,
    MoveSelectionUp,
    ClearQueue,
    ShuffleQueue,
    PaneLeft,
    PaneRight,
    ToggleOutput,
    SavePlaylist,
    RenamePlaylist,
    DeletePlaylist,
    ReloadLyrics,
    LyricsEarlier,
    LyricsLater,
    EditTag,
    SaveTags,
    RetryScrobbles,
    BrowserUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyChord {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyChord {
    fn from_event(key: KeyEvent) -> Self {
        let mut modifiers = key.modifiers;
        // For character keys the case is encoded in the code itself; drop the
        // redundant SHIFT. `BackTab` already means shift+tab.
        if matches!(key.code, KeyCode::Char(_) | KeyCode::BackTab) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
        Self {
            code: key.code,
            modifiers,
        }
    }
}

/// A keymap with a global layer and a per-screen layer.
///
/// Screen bindings win over global bindings, which is what lets `d` mean
/// "delete" in the queue but nothing in the library.
#[derive(Debug, Clone, Default)]
pub struct Keymap {
    global: HashMap<KeyChord, Action>,
    screens: Vec<HashMap<KeyChord, Action>>,
}

impl Keymap {
    pub fn default_bindings() -> Self {
        let mut keymap = Self {
            global: HashMap::new(),
            screens: (0..Screen::ALL.len()).map(|_| HashMap::new()).collect(),
        };

        let key = |spec: &str| parse_chord(spec).expect("valid default binding");
        let global = |keymap: &mut Self, spec: &str, action: Action| {
            keymap.global.insert(key(spec), action);
        };
        let screen = |keymap: &mut Self, screen: Screen, spec: &str, action: Action| {
            keymap.screens[screen.index()].insert(key(spec), action);
        };

        global(&mut keymap, "q", Action::Quit);
        global(&mut keymap, "ctrl+c", Action::Quit);
        global(&mut keymap, "?", Action::ToggleHelp);
        global(&mut keymap, "/", Action::OpenSearch);
        global(&mut keymap, ":", Action::OpenPalette);
        global(&mut keymap, "tab", Action::NextScreen);
        global(&mut keymap, "shift+tab", Action::PreviousScreen);
        for (index, spec) in ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
            .into_iter()
            .enumerate()
        {
            global(&mut keymap, spec, Action::GotoScreen(index));
        }
        global(&mut keymap, "0", Action::GotoScreen(9));

        for spec in ["j", "down"] {
            global(&mut keymap, spec, Action::MoveDown);
        }
        for spec in ["k", "up"] {
            global(&mut keymap, spec, Action::MoveUp);
        }
        global(&mut keymap, "ctrl+d", Action::PageDown);
        global(&mut keymap, "ctrl+u", Action::PageUp);
        global(&mut keymap, "pagedown", Action::PageDown);
        global(&mut keymap, "pageup", Action::PageUp);
        for spec in ["home", "g"] {
            global(&mut keymap, spec, Action::First);
        }
        for spec in ["end", "G"] {
            global(&mut keymap, spec, Action::Last);
        }

        for spec in ["p", "space"] {
            global(&mut keymap, spec, Action::TogglePlay);
        }
        global(&mut keymap, "s", Action::Stop);
        for spec in [">", ".", "n"] {
            global(&mut keymap, spec, Action::NextTrack);
        }
        for spec in ["<", ",", "N"] {
            global(&mut keymap, spec, Action::PreviousTrack);
        }
        for spec in ["+", "="] {
            global(&mut keymap, spec, Action::VolumeUp);
        }
        global(&mut keymap, "-", Action::VolumeDown);
        global(&mut keymap, "f", Action::SeekForward);
        global(&mut keymap, "b", Action::SeekBackward);
        global(&mut keymap, "r", Action::ToggleRepeat);
        global(&mut keymap, "z", Action::ToggleRandom);
        global(&mut keymap, "x", Action::ToggleSingle);
        global(&mut keymap, "c", Action::ToggleConsume);
        global(&mut keymap, "R", Action::Refresh);
        global(&mut keymap, "enter", Action::PlaySelected);

        screen(&mut keymap, Screen::Queue, "d", Action::RemoveSelected);
        screen(&mut keymap, Screen::Queue, "J", Action::MoveSelectionDown);
        screen(&mut keymap, Screen::Queue, "K", Action::MoveSelectionUp);
        screen(&mut keymap, Screen::Queue, "C", Action::ClearQueue);
        screen(&mut keymap, Screen::Queue, "S", Action::ShuffleQueue);

        screen(&mut keymap, Screen::Library, "a", Action::EnqueueSelected);
        screen(&mut keymap, Screen::Library, "A", Action::ReplaceAndPlay);
        for spec in ["h", "left"] {
            screen(&mut keymap, Screen::Library, spec, Action::PaneLeft);
        }
        for spec in ["l", "right"] {
            screen(&mut keymap, Screen::Library, spec, Action::PaneRight);
        }

        screen(&mut keymap, Screen::Playlists, "a", Action::EnqueueSelected);
        for spec in ["h", "left"] {
            screen(&mut keymap, Screen::Playlists, spec, Action::PaneLeft);
        }
        for spec in ["l", "right"] {
            screen(&mut keymap, Screen::Playlists, spec, Action::PaneRight);
        }
        screen(&mut keymap, Screen::Playlists, "d", Action::RemoveSelected);
        screen(
            &mut keymap,
            Screen::Playlists,
            "J",
            Action::MoveSelectionDown,
        );
        screen(&mut keymap, Screen::Playlists, "K", Action::MoveSelectionUp);
        screen(&mut keymap, Screen::Playlists, "S", Action::SavePlaylist);
        screen(&mut keymap, Screen::Playlists, "e", Action::RenamePlaylist);
        screen(&mut keymap, Screen::Playlists, "x", Action::DeletePlaylist);

        screen(&mut keymap, Screen::Outputs, "enter", Action::ToggleOutput);

        screen(&mut keymap, Screen::Lyrics, "o", Action::ReloadLyrics);
        screen(&mut keymap, Screen::Lyrics, "[", Action::LyricsEarlier);
        screen(&mut keymap, Screen::Lyrics, "]", Action::LyricsLater);

        screen(&mut keymap, Screen::TagEditor, "enter", Action::EditTag);
        screen(&mut keymap, Screen::TagEditor, "w", Action::SaveTags);

        for spec in ["h", "left", "backspace"] {
            screen(&mut keymap, Screen::Browser, spec, Action::BrowserUp);
        }
        screen(&mut keymap, Screen::Browser, "a", Action::EnqueueSelected);
        screen(&mut keymap, Screen::Browser, "A", Action::ReplaceAndPlay);

        keymap
    }

    /// Applies `[keys.<context>]` overrides from configuration.
    ///
    /// `none` removes a binding. Invalid entries are logged and skipped so a
    /// typo does not make the application unusable.
    pub fn from_config(config: &HashMap<String, HashMap<String, String>>) -> Self {
        let mut keymap = Self::default_bindings();
        for (section, bindings) in config {
            let screen = if section.eq_ignore_ascii_case("global") {
                None
            } else {
                match parse_screen_name(section) {
                    Ok(screen) => Some(screen),
                    Err(error) => {
                        tracing::warn!(%section, %error, "ignoring keymap section");
                        continue;
                    }
                }
            };

            for (spec, action_spec) in bindings {
                let chord = match parse_chord(spec) {
                    Ok(chord) => chord,
                    Err(error) => {
                        tracing::warn!(%spec, %error, "ignoring key binding");
                        continue;
                    }
                };
                if action_spec.trim().eq_ignore_ascii_case("none") {
                    match screen {
                        Some(screen) => {
                            keymap.screens[screen.index()].remove(&chord);
                        }
                        None => {
                            keymap.global.remove(&chord);
                        }
                    }
                    continue;
                }
                match parse_action(action_spec) {
                    Ok(action) => match screen {
                        Some(screen) => {
                            keymap.screens[screen.index()].insert(chord, action);
                        }
                        None => {
                            keymap.global.insert(chord, action);
                        }
                    },
                    Err(error) => {
                        tracing::warn!(%action_spec, %error, "ignoring key binding");
                    }
                }
            }
        }
        keymap
    }

    pub fn action_for(&self, key: KeyEvent, screen: Screen) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let chord = KeyChord::from_event(key);
        self.screens
            .get(screen.index())
            .and_then(|bindings| bindings.get(&chord))
            .or_else(|| self.global.get(&chord))
            .copied()
    }
}

fn parse_chord(spec: &str) -> Result<KeyChord, String> {
    let mut rest = spec.trim();
    if rest.is_empty() {
        return Err("empty key specification".into());
    }
    let mut modifiers = KeyModifiers::NONE;
    loop {
        let lower = rest.to_ascii_lowercase();
        if lower.starts_with("ctrl+") {
            modifiers |= KeyModifiers::CONTROL;
            rest = &rest[5..];
        } else if lower.starts_with("alt+") {
            modifiers |= KeyModifiers::ALT;
            rest = &rest[4..];
        } else if lower.starts_with("shift+") {
            modifiers |= KeyModifiers::SHIFT;
            rest = &rest[6..];
        } else {
            break;
        }
    }
    if rest.is_empty() {
        return Err("key specification has modifiers but no key".into());
    }

    let lower = rest.to_ascii_lowercase();
    let mut code = match lower.as_str() {
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "backtab" => KeyCode::BackTab,
        "enter" => KeyCode::Enter,
        "esc" | "escape" => KeyCode::Esc,
        "backspace" => KeyCode::Backspace,
        "delete" | "del" => KeyCode::Delete,
        "insert" | "ins" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "pageup" | "pgup" => KeyCode::PageUp,
        "pagedown" | "pgdn" => KeyCode::PageDown,
        _ => {
            let mut chars = rest.chars();
            let character = chars
                .next()
                .ok_or_else(|| format!("unknown key {rest:?}"))?;
            if chars.next().is_some() {
                return Err(format!("unknown key {rest:?}"));
            }
            KeyCode::Char(character)
        }
    };

    if modifiers.contains(KeyModifiers::SHIFT) {
        if code == KeyCode::Tab {
            code = KeyCode::BackTab;
        }
        if matches!(code, KeyCode::Char(_) | KeyCode::BackTab) {
            modifiers.remove(KeyModifiers::SHIFT);
        }
    }

    Ok(KeyChord { code, modifiers })
}

pub(crate) fn parse_action(spec: &str) -> Result<Action, String> {
    let spec = spec.trim();
    let lower = spec.to_ascii_lowercase();
    if let Some(screen) = lower
        .strip_prefix("screen:")
        .or_else(|| lower.strip_prefix("goto:"))
    {
        return Ok(Action::GotoScreen(parse_screen_name(screen)?.index()));
    }
    Ok(match lower.as_str() {
        "quit" => Action::Quit,
        "help" | "toggle_help" => Action::ToggleHelp,
        "search" | "open_search" => Action::OpenSearch,
        "palette" | "command_palette" => Action::OpenPalette,
        "next_screen" => Action::NextScreen,
        "prev_screen" | "previous_screen" => Action::PreviousScreen,
        "refresh" => Action::Refresh,
        "move_down" | "down" => Action::MoveDown,
        "move_up" | "up" => Action::MoveUp,
        "page_down" => Action::PageDown,
        "page_up" => Action::PageUp,
        "first" | "top" => Action::First,
        "last" | "bottom" => Action::Last,
        "toggle_play" | "play_pause" => Action::TogglePlay,
        "stop" => Action::Stop,
        "next_track" | "next" => Action::NextTrack,
        "previous_track" | "prev_track" | "previous" => Action::PreviousTrack,
        "volume_up" => Action::VolumeUp,
        "volume_down" => Action::VolumeDown,
        "seek_forward" => Action::SeekForward,
        "seek_backward" => Action::SeekBackward,
        "toggle_repeat" | "repeat" => Action::ToggleRepeat,
        "toggle_random" | "random" => Action::ToggleRandom,
        "toggle_single" | "single" => Action::ToggleSingle,
        "toggle_consume" | "consume" => Action::ToggleConsume,
        "play_selected" | "play" => Action::PlaySelected,
        "enqueue" | "append" => Action::EnqueueSelected,
        "replace_and_play" | "replace" => Action::ReplaceAndPlay,
        "remove" | "delete" => Action::RemoveSelected,
        "move_item_down" => Action::MoveSelectionDown,
        "move_item_up" => Action::MoveSelectionUp,
        "clear_queue" => Action::ClearQueue,
        "shuffle" => Action::ShuffleQueue,
        "pane_left" => Action::PaneLeft,
        "pane_right" => Action::PaneRight,
        "toggle_output" => Action::ToggleOutput,
        "save_playlist" => Action::SavePlaylist,
        "rename_playlist" => Action::RenamePlaylist,
        "delete_playlist" => Action::DeletePlaylist,
        "reload_lyrics" => Action::ReloadLyrics,
        "lyrics_earlier" => Action::LyricsEarlier,
        "lyrics_later" => Action::LyricsLater,
        "edit_tag" => Action::EditTag,
        "save_tags" => Action::SaveTags,
        "retry_scrobbles" => Action::RetryScrobbles,
        "browser_up" => Action::BrowserUp,
        other => return Err(format!("unknown action {other:?}")),
    })
}

fn parse_screen_name(value: &str) -> Result<Screen, String> {
    Ok(match value.trim().to_ascii_lowercase().as_str() {
        "queue" | "1" => Screen::Queue,
        "library" | "2" => Screen::Library,
        "playlists" | "3" => Screen::Playlists,
        "outputs" | "4" => Screen::Outputs,
        "stats" | "server" | "5" => Screen::Stats,
        "lyrics" | "6" => Screen::Lyrics,
        "visualizer" | "7" => Screen::Visualizer,
        "tags" | "tag-editor" | "tag editor" | "8" => Screen::TagEditor,
        "history" | "9" => Screen::History,
        "browser" | "files" | "0" => Screen::Browser,
        other => return Err(format!("unknown screen {other:?}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn action(code: KeyCode, screen: Screen) -> Option<Action> {
        Keymap::default_bindings().action_for(key(code), screen)
    }

    #[test]
    fn global_transport_keys() {
        assert_eq!(
            action(KeyCode::Char('p'), Screen::Queue),
            Some(Action::TogglePlay)
        );
        assert_eq!(
            action(KeyCode::Char('>'), Screen::Queue),
            Some(Action::NextTrack)
        );
        assert_eq!(
            action(KeyCode::Char('r'), Screen::Queue),
            Some(Action::ToggleRepeat)
        );
        assert_eq!(
            action(KeyCode::Char('1'), Screen::Queue),
            Some(Action::GotoScreen(0))
        );
        assert_eq!(
            action(KeyCode::Char('9'), Screen::Queue),
            Some(Action::GotoScreen(8))
        );
    }

    #[test]
    fn screen_specific_keys_do_not_leak() {
        assert_eq!(
            action(KeyCode::Char('d'), Screen::Queue),
            Some(Action::RemoveSelected)
        );
        assert_eq!(action(KeyCode::Char('d'), Screen::Library), None);
        assert_eq!(
            action(KeyCode::Char('h'), Screen::Library),
            Some(Action::PaneLeft)
        );
        assert_eq!(action(KeyCode::Char('h'), Screen::Queue), None);
        assert_eq!(
            action(KeyCode::Char('x'), Screen::Playlists),
            Some(Action::DeletePlaylist)
        );
        assert_eq!(
            action(KeyCode::Char('x'), Screen::Queue),
            Some(Action::ToggleSingle)
        );
    }

    #[test]
    fn enter_is_context_sensitive() {
        assert_eq!(
            action(KeyCode::Enter, Screen::Outputs),
            Some(Action::ToggleOutput)
        );
        assert_eq!(
            action(KeyCode::Enter, Screen::TagEditor),
            Some(Action::EditTag)
        );
        assert_eq!(
            action(KeyCode::Enter, Screen::Queue),
            Some(Action::PlaySelected)
        );
    }

    #[test]
    fn shifted_keys_match_their_uppercase_characters() {
        let keymap = Keymap::default_bindings();
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(keymap.action_for(event, Screen::Queue), Some(Action::Last));
    }

    #[test]
    fn config_overrides_and_unbinds() {
        let mut bindings = HashMap::new();
        bindings.insert("d".to_string(), "play_selected".to_string());
        bindings.insert("S".to_string(), "none".to_string());
        let mut config = HashMap::new();
        config.insert("queue".to_string(), bindings);
        let keymap = Keymap::from_config(&config);

        assert_eq!(
            keymap.action_for(key(KeyCode::Char('d')), Screen::Queue),
            Some(Action::PlaySelected)
        );
        assert_eq!(
            keymap.action_for(key(KeyCode::Char('d')), Screen::Library),
            None
        );
        assert_eq!(
            keymap.action_for(key(KeyCode::Char('S')), Screen::Queue),
            None
        );
    }

    #[test]
    fn parses_key_specs() {
        assert_eq!(
            parse_chord("ctrl+c").unwrap(),
            KeyChord {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL
            }
        );
        assert_eq!(
            parse_chord("shift+tab").unwrap(),
            KeyChord {
                code: KeyCode::BackTab,
                modifiers: KeyModifiers::NONE
            }
        );
        assert_eq!(
            parse_chord("+").unwrap(),
            KeyChord {
                code: KeyCode::Char('+'),
                modifiers: KeyModifiers::NONE
            }
        );
        assert!(parse_chord("nonsense").is_err());
    }

    #[test]
    fn colon_opens_the_palette() {
        assert_eq!(
            action(KeyCode::Char(':'), Screen::Queue),
            Some(Action::OpenPalette)
        );
    }

    #[test]
    fn browser_keys() {
        assert_eq!(
            action(KeyCode::Char('0'), Screen::Queue),
            Some(Action::GotoScreen(9))
        );
        assert_eq!(
            action(KeyCode::Backspace, Screen::Browser),
            Some(Action::BrowserUp)
        );
        assert_eq!(action(KeyCode::Backspace, Screen::Queue), None);
    }

    #[test]
    fn parses_action_specs() {
        assert_eq!(
            parse_action("screen:history").unwrap(),
            Action::GotoScreen(8)
        );
        assert_eq!(parse_action("toggle_play").unwrap(), Action::TogglePlay);
        assert_eq!(
            parse_action("retry_scrobbles").unwrap(),
            Action::RetryScrobbles
        );
        assert!(parse_action("dance").is_err());
    }
}
