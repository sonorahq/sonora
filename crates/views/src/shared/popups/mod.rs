//! The dialogs and searchable menus several screens share: the account choice a sign-in asks
//! for, the manual cookie sign-in, and the search field over a long picker.

mod accounts;
mod cookie;

use ui::{Menu, MenuSearch};

pub(crate) use accounts::account_picker;
pub(crate) use cookie::{CookiePrompt, cookie_hint};

/// How tall a searchable picker grows before it scrolls.
const SEARCH_HEIGHT: f32 = 320.;

/// The search field over a long picker, such as the languages or the typefaces, and the hint it
/// carries. Rune's menu search walks the rows with the arrow keys, takes focus as the menu
/// opens and picks with Return; this only keeps it with the hint and the menu's size.
#[derive(Clone, Debug)]
pub(crate) struct SearchPopup {
    pub(crate) search: MenuSearch,
    hint: &'static str,
}

impl SearchPopup {
    /// A search whose field shows the Fluent message `hint` while empty.
    pub(crate) fn new(hint: &'static str) -> Self {
        Self {
            search: MenuSearch::new(),
            hint,
        }
    }

    /// What was typed, trimmed and lowercased for matching.
    pub(crate) fn query(&self) -> String {
        self.search.query()
    }

    /// Empties the field, for a picker about to open afresh.
    pub(crate) fn reset(&mut self) {
        self.search.reset();
    }

    /// How tall the picker grows before it scrolls.
    pub(crate) fn height(&self) -> f32 {
        SEARCH_HEIGHT
    }

    /// The menu `key`, `width` wide, with this search over its rows. `access` finds this popup's
    /// `MenuSearch` in the window's state.
    pub(crate) fn menu<S: 'static>(
        &self,
        key: impl Into<String>,
        width: f32,
        access: impl for<'a> Fn(&'a mut S) -> &'a mut MenuSearch + 'static,
    ) -> Menu<S> {
        Menu::new(key)
            .width(width)
            .max_height(SEARCH_HEIGHT)
            .search(&self.search, i18n::lookup(self.hint, None), access)
    }
}

/// Whether an entry with `id` and `label` answers `query`, already lowercased. An empty query
/// answers to everything.
pub(crate) fn matches_query(id: &str, label: &str, query: &str) -> bool {
    query.is_empty() || label.to_lowercase().contains(query) || id.to_lowercase().contains(query)
}
