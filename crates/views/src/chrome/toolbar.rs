//! The shown page's part of the title bar: its tools (sort, filter, view, a create button) and,
//! for a page that can be filtered by typing, a search field that a magnifier opens and an ✕
//! closes.
//!
//! A screen offers tools by answering for its destination in [`page_tools`]: a function of the
//! screen's own (`fn tools(ui: &Ui, layer: &mut Vec<El>) -> Tools`) that builds them with
//! [`super::tools`]. A searchable screen reads what was typed with
//! `ui.chrome.toolbar.query(&ui.nav.current())`, which is empty while its field is shut or the
//! field belongs to another page.

use router::Destination;
use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx, row};
use ui::{Button, Input};

use crate::Ui;

/// The widest the open search field gets.
const WIDEST: f32 = 280.;

/// The search field's state. One field serves every page; it belongs to the page it was opened
/// on and reads as empty anywhere else.
pub struct State {
    open: bool,
    /// The page the open field filters, as [`page`] names it.
    page: Option<String>,
    pub edit: TextEdit,
}

impl Default for State {
    fn default() -> Self {
        Self {
            open: false,
            page: None,
            edit: TextEdit::new(""),
        }
    }
}

impl State {
    /// What the field filters `at` by: its text while it is open on that page, else nothing.
    pub fn query(&self, at: &Destination) -> &str {
        match self.shows(at) {
            true => self.edit.text.as_str(),
            false => "",
        }
    }

    /// Whether the field is open on the page `at`.
    pub fn shows(&self, at: &Destination) -> bool {
        self.open && self.page.as_deref() == Some(page(at).as_str())
    }
}

/// What a page puts in the title bar: its tools in order, and the Fluent key of its search
/// field's hint when it can be filtered by typing.
#[derive(Default)]
pub(crate) struct Tools {
    pub items: Vec<El>,
    pub search: Option<&'static str>,
}

/// The shown page's tools, then its search toggle (and the field while open). Empty for a page
/// that offers neither.
pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let at = ui.nav.current();
    let tools = page_tools(ui, &at, layer);
    let open = ui.chrome.toolbar.shows(&at);

    let mut bar = row()
        .key("toolbar")
        .role("toolbar")
        .min_w(0.)
        .center_y()
        .justify_end()
        .gap(theme.font_size * 0.25)
        .no_window_drag()
        .children(tools.items);
    let Some(hint) = tools.search else {
        return bar;
    };
    bar = bar.label(i18n::t!("common-search"));
    if open {
        let field = Input::new(
            "toolbar-search",
            &ui.chrome.toolbar.edit,
            i18n::lookup(hint, None),
            |ui: &mut Ui| &mut ui.chrome.toolbar.edit,
        )
        .icon("icons/search.svg")
        .compact()
        .autofocus()
        .on_key(
            |ui: &mut Ui, event: &KeyEvent, tx: &mut Tx| match event.key {
                Key::Escape => {
                    close(ui, tx);
                    true
                }
                _ => false,
            },
        )
        .build(theme);
        bar = bar.child(
            row()
                .key("toolbar-search-slot")
                .w(WIDEST)
                .min_w(0.)
                .shrink()
                .transition(rune::Transition::opacity().combined(rune::Transition::scale(0.96)))
                .child(field.grow(1.)),
        );
    }
    bar.child(
        Button::new("search-toggle")
            .icon(match open {
                true => "icons/x.svg",
                false => "icons/search.svg",
            })
            .tooltip(match open {
                true => i18n::t!("common-dismiss"),
                false => i18n::t!("common-search"),
            })
            .small()
            .ghost()
            .expanded(open)
            .on_press_tx(move |ui: &mut Ui, tx: &mut Tx| match open {
                true => close(ui, tx),
                false => focus(ui),
            })
            .build(theme),
    )
}

/// Opens the shown page's search field and gives it the keyboard, for a page that has one: the
/// magnifier and the filter shortcut both land here.
pub(crate) fn focus(ui: &mut Ui) {
    let at = ui.nav.current();
    if ui.chrome.toolbar.shows(&at) {
        ui.chrome.toolbar.edit.focused = true;
        return;
    }
    let state = &mut ui.chrome.toolbar;
    state.open = true;
    state.page = Some(page(&at));
    state.edit = TextEdit::new("");
    state.edit.focused = true;
}

/// Empties and shuts the search field and takes the keyboard off it. A field no longer drawn
/// would otherwise keep focus and swallow the shortcuts.
pub(crate) fn close(ui: &mut Ui, tx: &mut Tx) {
    let state = &mut ui.chrome.toolbar;
    let focused = state.edit.focused;
    state.open = false;
    state.page = None;
    state.edit = TextEdit::new("");
    if focused {
        tx.blur();
    }
}

/// The tools `at` offers. A screen that has tools answers for its destinations here; until it
/// does, its page shows only its search toggle, or nothing.
fn page_tools(ui: &Ui, at: &Destination, layer: &mut Vec<El>) -> Tools {
    let _ = (ui, &layer);
    match at {
        Destination::Library(_) | Destination::Local(_) => {
            crate::screens::library::tools(ui, layer)
        }
        Destination::History => Tools {
            search: Some("filter-history"),
            ..Tools::default()
        },
        Destination::Album(_) | Destination::Playlist(_) => {
            crate::screens::detail::tools(ui, layer)
        }
        Destination::Artist(_) => crate::screens::artist::tools(ui),
        Destination::Genre(_) => crate::screens::genre::tools(ui),
        Destination::Home
        | Destination::Search
        | Destination::Song(_)
        | Destination::User(_)
        | Destination::Settings(_)
        | Destination::Equalizer
        | Destination::Fullscreen => Tools::default(),
    }
}

/// The page a search field belongs to: a section's tabs share one, every item page has its own.
fn page(at: &Destination) -> String {
    match at {
        Destination::Library(_) => "library".to_owned(),
        Destination::Local(_) => "local".to_owned(),
        other => format!("{other:?}"),
    }
}
