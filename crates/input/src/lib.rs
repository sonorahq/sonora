use std::collections::BTreeMap;

use gpui::{KeyBinding, actions};
use ui::{
    Activate, Backspace, BackspaceToStart, BackspaceWord, Copy, Cut, Delete, DeleteToEnd,
    DeleteWord, Deselect, Dismiss, End, FORM_CONTEXT, Home, INPUT_CONTEXT, Left, MENU_CONTEXT,
    NextField, Paste, PreviousField, Remove, Right, SelectAll, SelectEnd, SelectHome, SelectLeft,
    SelectNext, SelectPrevious, SelectRight, SelectWordLeft, SelectWordRight, ShowCharacterPalette,
    Space, Submit, TABLE_CONTEXT, WordLeft, WordRight,
};

mod shortcuts;

pub use shortcuts::{
    Section, Shortcut, assign, clear, keystroke_of, reset, shortcut, shortcuts, shown,
};

actions!(
    sonora,
    [
        Quit,
        SignOut,
        Refresh,
        TogglePlayback,
        SongPrevious,
        SongNext,
        VolumeUp,
        VolumeDown,
        SeekBack,
        SeekForward,
        ToggleShuffle,
        ToggleRepeat,
        NavigateBack,
        NavigateForward,
        OpenFilter,
        OpenSearch,
        OpenSettings,
        ToggleFullscreen,
        ToggleQueue,
        ToggleLyrics,
        TogglePowerbar,
        PowerbarDetails,
        ShowMembers,
        CloseWindow,
        MinimizeWindow,
        ZoomWindow,
        ToggleWindowFullscreen,
        Hide,
        HideOthers,
        ShowAll,
        IncreaseFontSize,
        DecreaseFontSize,
        ResetFontSize,
        Undo,
        Redo
    ]
);

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const SEARCH_CONTEXT: &str = "Search";
pub const POWERBAR_CONTEXT: &str = "Powerbar";

/// Every key binding, in precedence order: GPUI prefers the deepest matching context and,
/// within one, the binding registered last, so the macOS set at the end overrides the shared one.
/// The shortcuts come before the shared set so that a key the page takes for a list, such as
/// Up and Down, keeps the list's binding when both match. Once the list lets the key go, GPUI
/// tries the next binding, which is how the arrow keys reach the volume and seek shortcuts.
/// `overrides` holds the keys the user chose for shortcuts under Settings, Keybinds, by
/// shortcut id. A shortcut missing from it keeps its defaults, and an empty list unbinds it.
pub fn bindings(overrides: &BTreeMap<String, Vec<String>>) -> Vec<KeyBinding> {
    let mut bindings = shortcuts::bindings(overrides);
    bindings.extend(shared());
    if cfg!(target_os = "macos") {
        bindings.extend(macos());
    }
    bindings
}

/// The fixed bindings every platform gets: quitting, equalizer undo, lists, menus, search, forms
/// and text editing. Both `cmd-` and `ctrl-` are registered for each shortcut.
fn shared() -> Vec<KeyBinding> {
    let editing = Some(INPUT_CONTEXT);
    let away_from_text = format!("{WORKSPACE_CONTEXT} && !{INPUT_CONTEXT}");
    let table = Some(TABLE_CONTEXT);
    let form = Some(FORM_CONTEXT);
    let menu = Some(MENU_CONTEXT);
    let search = Some(SEARCH_CONTEXT);
    let browsing = format!(
        "{WORKSPACE_CONTEXT} && !{INPUT_CONTEXT} && !{MENU_CONTEXT} && !{FORM_CONTEXT} && !{TABLE_CONTEXT} && !{SEARCH_CONTEXT}"
    );
    let results = format!("{SEARCH_CONTEXT} && !{INPUT_CONTEXT}");
    let typing = format!("{SEARCH_CONTEXT} > {INPUT_CONTEXT}");

    vec![
        KeyBinding::new("down", SelectNext, table),
        KeyBinding::new("up", SelectPrevious, table),
        KeyBinding::new("shift-down", SelectNext, table),
        KeyBinding::new("shift-up", SelectPrevious, table),
        KeyBinding::new("enter", Activate, table),
        KeyBinding::new("delete", Remove, table),
        KeyBinding::new("escape", Deselect, table),
        KeyBinding::new("down", SelectNext, Some(&browsing)),
        KeyBinding::new("up", SelectPrevious, Some(&browsing)),
        KeyBinding::new("shift-down", SelectNext, Some(&browsing)),
        KeyBinding::new("shift-up", SelectPrevious, Some(&browsing)),
        KeyBinding::new("enter", Activate, Some(&browsing)),
        KeyBinding::new("delete", Remove, Some(&browsing)),
        KeyBinding::new("escape", Deselect, Some(&browsing)),
        KeyBinding::new("down", SelectNext, search),
        KeyBinding::new("up", SelectPrevious, search),
        KeyBinding::new("left", SelectLeft, Some(&results)),
        KeyBinding::new("right", SelectRight, Some(&results)),
        KeyBinding::new("enter", Activate, Some(&results)),
        KeyBinding::new("escape", Deselect, Some(&results)),
        KeyBinding::new("enter", Submit, Some(&typing)),
        KeyBinding::new("down", SelectNext, menu),
        KeyBinding::new("up", SelectPrevious, menu),
        KeyBinding::new("tab", SelectNext, menu),
        KeyBinding::new("shift-tab", SelectPrevious, menu),
        KeyBinding::new("enter", Submit, menu),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("down", SelectNext, Some(POWERBAR_CONTEXT)),
        KeyBinding::new("up", SelectPrevious, Some(POWERBAR_CONTEXT)),
        KeyBinding::new("enter", Submit, Some(POWERBAR_CONTEXT)),
        KeyBinding::new("ctrl-enter", PowerbarDetails, Some(POWERBAR_CONTEXT)),
        KeyBinding::new("cmd-enter", PowerbarDetails, Some(POWERBAR_CONTEXT)),
        KeyBinding::new("cmd-z", Undo, Some(&away_from_text)),
        KeyBinding::new("ctrl-z", Undo, Some(&away_from_text)),
        KeyBinding::new("shift-cmd-z", Redo, Some(&away_from_text)),
        KeyBinding::new("shift-ctrl-z", Redo, Some(&away_from_text)),
        KeyBinding::new("ctrl-y", Redo, Some(&away_from_text)),
        KeyBinding::new("escape", Dismiss, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("backspace", Backspace, editing),
        KeyBinding::new("ctrl-backspace", BackspaceWord, editing),
        KeyBinding::new("delete", Delete, editing),
        KeyBinding::new("ctrl-delete", DeleteWord, editing),
        KeyBinding::new("left", Left, editing),
        KeyBinding::new("right", Right, editing),
        KeyBinding::new("ctrl-left", WordLeft, editing),
        KeyBinding::new("ctrl-right", WordRight, editing),
        KeyBinding::new("shift-left", SelectLeft, editing),
        KeyBinding::new("shift-right", SelectRight, editing),
        KeyBinding::new("shift-ctrl-left", SelectWordLeft, editing),
        KeyBinding::new("shift-ctrl-right", SelectWordRight, editing),
        KeyBinding::new("home", Home, editing),
        KeyBinding::new("end", End, editing),
        KeyBinding::new("cmd-left", Home, editing),
        KeyBinding::new("cmd-right", End, editing),
        KeyBinding::new("shift-home", SelectHome, editing),
        KeyBinding::new("shift-end", SelectEnd, editing),
        KeyBinding::new("shift-cmd-left", SelectHome, editing),
        KeyBinding::new("shift-cmd-right", SelectEnd, editing),
        KeyBinding::new("cmd-a", SelectAll, editing),
        KeyBinding::new("ctrl-a", SelectAll, editing),
        KeyBinding::new("cmd-v", Paste, editing),
        KeyBinding::new("ctrl-v", Paste, editing),
        KeyBinding::new("cmd-c", Copy, editing),
        KeyBinding::new("ctrl-c", Copy, editing),
        KeyBinding::new("cmd-x", Cut, editing),
        KeyBinding::new("ctrl-x", Cut, editing),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, editing),
        KeyBinding::new("space", Space, editing),
        KeyBinding::new("escape", Dismiss, editing),
        KeyBinding::new("enter", Submit, form),
        KeyBinding::new("escape", Dismiss, form),
        KeyBinding::new("tab", NextField, form),
        KeyBinding::new("shift-tab", PreviousField, form),
    ]
}

/// What a Mac user expects on top of the shared set: the standard window and application
/// shortcuts, and the Cocoa text-field conventions, `alt` for words, `cmd` for the line and the
/// Emacs control keys.
fn macos() -> Vec<KeyBinding> {
    let editing = Some(INPUT_CONTEXT);

    vec![
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("alt-left", WordLeft, editing),
        KeyBinding::new("alt-right", WordRight, editing),
        KeyBinding::new("shift-alt-left", SelectWordLeft, editing),
        KeyBinding::new("shift-alt-right", SelectWordRight, editing),
        KeyBinding::new("alt-backspace", BackspaceWord, editing),
        KeyBinding::new("alt-delete", DeleteWord, editing),
        KeyBinding::new("cmd-backspace", BackspaceToStart, editing),
        KeyBinding::new("cmd-delete", DeleteToEnd, editing),
        KeyBinding::new("cmd-up", Home, editing),
        KeyBinding::new("cmd-down", End, editing),
        KeyBinding::new("shift-cmd-up", SelectHome, editing),
        KeyBinding::new("shift-cmd-down", SelectEnd, editing),
        KeyBinding::new("ctrl-a", Home, editing),
        KeyBinding::new("ctrl-e", End, editing),
        KeyBinding::new("ctrl-b", Left, editing),
        KeyBinding::new("ctrl-f", Right, editing),
        KeyBinding::new("ctrl-h", Backspace, editing),
        KeyBinding::new("ctrl-d", Delete, editing),
        KeyBinding::new("ctrl-k", DeleteToEnd, editing),
    ]
}
