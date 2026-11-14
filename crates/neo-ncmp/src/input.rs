use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::Screen;

/// A user intention, independent of the key that produced it.
///
/// Keeping this as a data type is what will make configurable keybindings and
/// a command palette a small change later instead of a rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleHelp,
    OpenSearch,
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
}

/// Maps a key press to an action in the current screen context.
///
/// Overlays (help, confirmation) are handled by the application before this
/// function is called.
pub fn action_for(key: KeyEvent, screen: Screen) -> Option<Action> {
    if key.kind != KeyEventKind::Press {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    match key.code {
        KeyCode::Tab => return Some(Action::NextScreen),
        KeyCode::BackTab => return Some(Action::PreviousScreen),
        KeyCode::Esc => return None,
        KeyCode::Home => return Some(Action::First),
        KeyCode::End => return Some(Action::Last),
        KeyCode::PageDown => return Some(Action::PageDown),
        KeyCode::PageUp => return Some(Action::PageUp),
        KeyCode::Up => return Some(Action::MoveUp),
        KeyCode::Down => return Some(Action::MoveDown),
        KeyCode::Left if screen == Screen::Library => return Some(Action::PaneLeft),
        KeyCode::Right if screen == Screen::Library => return Some(Action::PaneRight),
        KeyCode::Enter => {
            return Some(if screen == Screen::Outputs {
                Action::ToggleOutput
            } else {
                Action::PlaySelected
            });
        }
        _ => {}
    }

    if ctrl {
        return match key.code {
            KeyCode::Char('d') => Some(Action::PageDown),
            KeyCode::Char('u') => Some(Action::PageUp),
            _ => None,
        };
    }

    let KeyCode::Char(character) = key.code else {
        return None;
    };

    let action = match character {
        'q' => Action::Quit,
        '?' => Action::ToggleHelp,
        '/' => Action::OpenSearch,
        'R' => Action::Refresh,

        '1' => Action::GotoScreen(0),
        '2' => Action::GotoScreen(1),
        '3' => Action::GotoScreen(2),
        '4' => Action::GotoScreen(3),

        'j' => Action::MoveDown,
        'k' => Action::MoveUp,
        'g' => Action::First,
        'G' => Action::Last,

        'p' | ' ' => Action::TogglePlay,
        's' => Action::Stop,
        '>' | '.' | 'n' => Action::NextTrack,
        '<' | ',' | 'N' => Action::PreviousTrack,
        '+' | '=' => Action::VolumeUp,
        '-' => Action::VolumeDown,
        'f' => Action::SeekForward,
        'b' => Action::SeekBackward,
        'r' => Action::ToggleRepeat,
        'z' => Action::ToggleRandom,
        'x' => Action::ToggleSingle,
        'c' => Action::ToggleConsume,

        'a' if screen == Screen::Library => Action::EnqueueSelected,
        'A' if screen == Screen::Library => Action::ReplaceAndPlay,
        'h' if screen == Screen::Library => Action::PaneLeft,
        'l' if screen == Screen::Library => Action::PaneRight,
        'd' if screen == Screen::Queue => Action::RemoveSelected,
        'J' if screen == Screen::Queue => Action::MoveSelectionDown,
        'K' if screen == Screen::Queue => Action::MoveSelectionUp,
        'C' if screen == Screen::Queue => Action::ClearQueue,
        'S' if screen == Screen::Queue => Action::ShuffleQueue,
        _ => return None,
    };
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn global_transport_keys() {
        assert_eq!(
            action_for(key(KeyCode::Char('p')), Screen::Queue),
            Some(Action::TogglePlay)
        );
        assert_eq!(
            action_for(key(KeyCode::Char('>')), Screen::Queue),
            Some(Action::NextTrack)
        );
        assert_eq!(
            action_for(key(KeyCode::Char('r')), Screen::Queue),
            Some(Action::ToggleRepeat)
        );
        assert_eq!(
            action_for(key(KeyCode::Char('1')), Screen::Queue),
            Some(Action::GotoScreen(0))
        );
    }

    #[test]
    fn screen_specific_keys_do_not_leak() {
        assert_eq!(
            action_for(key(KeyCode::Char('d')), Screen::Queue),
            Some(Action::RemoveSelected)
        );
        assert_eq!(action_for(key(KeyCode::Char('d')), Screen::Library), None);
        assert_eq!(
            action_for(key(KeyCode::Char('h')), Screen::Library),
            Some(Action::PaneLeft)
        );
        assert_eq!(action_for(key(KeyCode::Char('h')), Screen::Queue), None);
    }

    #[test]
    fn enter_toggles_outputs() {
        assert_eq!(
            action_for(key(KeyCode::Enter), Screen::Outputs),
            Some(Action::ToggleOutput)
        );
        assert_eq!(
            action_for(key(KeyCode::Enter), Screen::Queue),
            Some(Action::PlaySelected)
        );
    }

    #[test]
    fn slash_opens_search() {
        assert_eq!(
            action_for(key(KeyCode::Char('/')), Screen::Queue),
            Some(Action::OpenSearch)
        );
    }

    #[test]
    fn control_c_quits() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(action_for(ctrl_c, Screen::Library), Some(Action::Quit));
    }
}
