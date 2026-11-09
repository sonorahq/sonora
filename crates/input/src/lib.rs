//! The app's keyboard shortcuts. Text editing, list and menu navigation are Rune's own (its
//! fields, focus groups and modals); what is left here are the shortcuts that act on the app.

use rune::{Chord, KeyEvent, Tx};

/// Something a shortcut does, whatever has focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Quit,
    Refresh,
    TogglePlayback,
    SongPrevious,
    SongNext,
    NavigateBack,
    NavigateForward,
    OpenFilter,
    OpenSearch,
    OpenSettings,
    ToggleFullscreen,
    ToggleWindowFullscreen,
    CloseWindow,
    MinimizeWindow,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    Undo,
    Redo,
    Dismiss,
}

/// Where a shortcut applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Whatever has focus.
    Anywhere,
    /// Only while no text field has the keyboard, so typing a space or an `f` types it.
    AwayFromText,
}

/// One shortcut: its chord, what it does and where.
pub struct Binding {
    pub chord: Chord,
    pub action: Action,
    pub reach: Reach,
}

/// Every shortcut, platform ones included. `mod` is ⌘ on macOS and Ctrl elsewhere.
pub fn bindings() -> Vec<Binding> {
    use Action::*;
    use Reach::*;

    let mut table = vec![
        ("alt+left", NavigateBack, Anywhere),
        ("alt+right", NavigateForward, Anywhere),
        ("mod+q", Quit, Anywhere),
        ("mod+r", Refresh, Anywhere),
        ("mod+f", OpenFilter, Anywhere),
        ("mod+shift+f", OpenSearch, Anywhere),
        ("mod+,", OpenSettings, Anywhere),
        ("mod+=", IncreaseFontSize, Anywhere),
        ("mod+plus", IncreaseFontSize, Anywhere),
        ("mod+-", DecreaseFontSize, Anywhere),
        ("mod+0", ResetFontSize, Anywhere),
        ("space", TogglePlayback, AwayFromText),
        ("ctrl+left", SongPrevious, AwayFromText),
        ("ctrl+right", SongNext, AwayFromText),
        ("f", ToggleFullscreen, AwayFromText),
        ("f11", ToggleWindowFullscreen, Anywhere),
        ("mod+z", Undo, AwayFromText),
        ("mod+shift+z", Redo, AwayFromText),
        ("escape", Dismiss, Anywhere),
    ];
    if cfg!(target_os = "macos") {
        table.extend([
            ("mod+w", CloseWindow, Anywhere),
            ("mod+m", MinimizeWindow, Anywhere),
            ("ctrl+mod+f", ToggleWindowFullscreen, Anywhere),
            ("mod+[", NavigateBack, Anywhere),
            ("mod+]", NavigateForward, Anywhere),
        ]);
    } else {
        table.push(("ctrl+y", Redo, AwayFromText));
    }
    table
        .into_iter()
        .filter_map(|(chord, action, reach)| {
            Some(Binding {
                chord: Chord::parse(chord).ok()?,
                action,
                reach,
            })
        })
        .collect()
}

/// The action a key press asks for, if any. `typing` says whether a text field has the keyboard.
pub fn action(bindings: &[Binding], event: &KeyEvent, tx: &Tx, typing: bool) -> Option<Action> {
    bindings
        .iter()
        .filter(|binding| !typing || binding.reach == Reach::Anywhere)
        .find(|binding| binding.chord.matches_tx(event, tx))
        .map(|binding| binding.action)
}
