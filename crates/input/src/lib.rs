use gpui::{KeyBinding, actions};
use ui::{
    Activate, Backspace, BackspaceWord, Copy, Cut, Delete, DeleteWord, Deselect, Dismiss, End,
    FORM_CONTEXT, Home, INPUT_CONTEXT, Left, MENU_CONTEXT, Paste, Remove, Right, SelectAll,
    SelectEnd, SelectHome, SelectLeft, SelectNext, SelectPrevious, SelectRight, SelectWordLeft,
    SelectWordRight, ShowCharacterPalette, Space, Submit, TABLE_CONTEXT, WordLeft, WordRight,
};

actions!(
    sonora,
    [
        Quit,
        SignOut,
        RefreshLibrary,
        TogglePlayback,
        SongPrevious,
        SongNext,
        SeekBack,
        SeekForward,
        NavigateBack,
        NavigateForward,
        OpenFilter,
        OpenSearch,
        OpenSettings,
        ToggleFullscreen,
        ToggleQueue,
        ToggleLyrics
    ]
);

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const SEARCH_CONTEXT: &str = "Search";

pub fn bindings() -> Vec<KeyBinding> {
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
        KeyBinding::new("left", SeekBack, None),
        KeyBinding::new("right", SeekForward, None),
        KeyBinding::new("down", SelectNext, search),
        KeyBinding::new("up", SelectPrevious, search),
        KeyBinding::new("left", SelectLeft, Some(&results)),
        KeyBinding::new("right", SelectRight, Some(&results)),
        KeyBinding::new("enter", Activate, Some(&results)),
        KeyBinding::new("escape", Deselect, Some(&results)),
        KeyBinding::new("down", SelectNext, menu),
        KeyBinding::new("up", SelectPrevious, menu),
        KeyBinding::new("tab", SelectNext, menu),
        KeyBinding::new("shift-tab", SelectPrevious, menu),
        KeyBinding::new("enter", Submit, menu),
        KeyBinding::new("alt-left", NavigateBack, None),
        KeyBinding::new("alt-right", NavigateForward, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("ctrl-q", Quit, None),
        KeyBinding::new("cmd-r", RefreshLibrary, None),
        KeyBinding::new("ctrl-r", RefreshLibrary, None),
        KeyBinding::new("cmd-f", OpenFilter, None),
        KeyBinding::new("ctrl-f", OpenFilter, None),
        KeyBinding::new("shift-cmd-f", OpenSearch, None),
        KeyBinding::new("shift-ctrl-f", OpenSearch, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("space", TogglePlayback, Some(&away_from_text)),
        KeyBinding::new("ctrl-left", SongPrevious, Some(&away_from_text)),
        KeyBinding::new("ctrl-right", SongNext, Some(&away_from_text)),
        KeyBinding::new("f", ToggleFullscreen, Some(&away_from_text)),
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
    ]
}
