use gpui::prelude::*;
use gpui::{Context, ElementId, Hsla};
use router::NavEntry;
use state::{AppSettings, Tool};
use ui::Button;

/// A navigation row as the sidebars draw it: a ghost button with its label at the start, in
/// `tint`, that takes `accent` while hovered or pressed and lets it fade once the pointer
/// leaves. The caller adds the icon and lights the row it considers current with
/// `.lit(true, accent)`.
pub(crate) fn nav_row(
    id: impl Into<ElementId>,
    key: &'static str,
    tint: Hsla,
    accent: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .label(i18n::lookup(key, None))
        .tint(tint)
        .gap_2p5()
        .justify_start()
        .lit(false, accent)
        .active(move |style| style.bg(accent))
}

/// A navigation row reduced to its icon, named by a tooltip beside it.
pub(crate) fn nav_icon(
    id: impl Into<ElementId>,
    key: &'static str,
    icon: &'static str,
    tint: Hsla,
    accent: Hsla,
) -> Button {
    Button::new(id)
        .ghost()
        .icon(icon)
        .tooltip_trailing(key)
        .tint(tint)
        // Its fill hugs the icon rather than spanning the rail.
        .self_center()
        .lit(false, accent)
        .active(move |style| style.bg(accent))
}

/// Whether the sidebar shows `entry`. The pinned section follows its own switch, and every other
/// row is a tool of the sidebar's arrangement.
pub(crate) fn entry_shown(entry: NavEntry, settings: &AppSettings) -> bool {
    match Tool::from_id(entry.id()) {
        Some(tool) => settings.panels().in_sidebar(tool),
        None => settings.nav_shown(entry.id()),
    }
}

/// Shows or hides `entry` in the sidebar, the counterpart of `entry_shown`. A row shown here
/// leaves whichever other bar held it.
pub(crate) fn set_entry_shown(
    entry: NavEntry,
    shown: bool,
    settings: &mut AppSettings,
    cx: &mut Context<AppSettings>,
) {
    let Some(tool) = Tool::from_id(entry.id()) else {
        settings.set_nav_shown(entry.id(), shown, cx);
        return;
    };
    let mut panels = settings.panels();
    panels.set_in_sidebar(tool, shown);
    settings.set_panels(panels, cx);
}
