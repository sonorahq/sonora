use gpui::App;
use i18n::t;
use state::{Library, Shelf};
use ui::{Filter, FilterChange, FlagAxis};

const LOCAL_ONLY: &str = "filter-local-only";
const STREAMING_ONLY: &str = "filter-streaming-only";
/// The glyph that marks a local file on a library page that mixes the two shelves.
pub(crate) const LOCAL_MARK: &str = "icons/file-music.svg";

/// Two lists read as one, every row of `head` before the rows of `tail`. The library pages list
/// the streaming shelf and the local one together through it, without copying either.
pub(crate) struct Joined<'a, T> {
    head: &'a [T],
    tail: &'a [T],
}

impl<T> Clone for Joined<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Joined<'_, T> {}

impl<'a, T> Joined<'a, T> {
    pub(crate) fn new(head: &'a [T], tail: &'a [T]) -> Self {
        Self { head, tail }
    }

    pub(crate) fn len(&self) -> usize {
        self.head.len() + self.tail.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.head.is_empty() && self.tail.is_empty()
    }

    pub(crate) fn get(&self, row: usize) -> Option<&'a T> {
        match self.head.get(row) {
            Some(item) => Some(item),
            None => self.tail.get(row - self.head.len()),
        }
    }

    pub(crate) fn first(&self) -> Option<&'a T> {
        self.get(0)
    }

    pub(crate) fn iter(
        &self,
    ) -> std::iter::Chain<std::slice::Iter<'a, T>, std::slice::Iter<'a, T>> {
        self.head.iter().chain(self.tail.iter())
    }

    /// Whether both lists hold something, the only time a page mixes the two.
    pub(crate) fn mixed(&self) -> bool {
        !self.head.is_empty() && !self.tail.is_empty()
    }

    pub(crate) fn to_vec(self) -> Vec<T>
    where
        T: Clone,
    {
        self.iter().cloned().collect()
    }
}

impl<'a, T> From<&'a [T]> for Joined<'a, T> {
    fn from(rows: &'a [T]) -> Self {
        Self::new(rows, &[])
    }
}

impl<'a, T> IntoIterator for Joined<'a, T> {
    type Item = &'a T;
    type IntoIter = std::iter::Chain<std::slice::Iter<'a, T>, std::slice::Iter<'a, T>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// One part of the library across both shelves, the streaming rows first.
pub(crate) fn joined<'a, T: 'a>(
    library: &'a Library,
    pick: impl Fn(&'a state::LibraryState) -> &'a [T],
) -> Joined<'a, T> {
    Joined::new(
        pick(library.state(Shelf::Streaming)),
        pick(library.state(Shelf::Local)),
    )
}

/// The shelf a library page is narrowed to by its Local only or Streaming only filter. The two
/// rule each other out, so turning one on turns the other off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reach(Option<Shelf>);

impl Reach {
    /// Whether a row with this id stays on the page.
    pub(crate) fn keeps(self, id: &str) -> bool {
        self.0.is_none_or(|shelf| Shelf::of(id) == shelf)
    }

    pub(crate) fn active(self) -> bool {
        self.0.is_some()
    }

    /// Whether a page that `mixed` both shelves badges its local rows. It does unless the Local
    /// only filter is on, when every row left is local and the badge would tell nothing apart.
    pub(crate) fn marks_local(self, mixed: bool) -> bool {
        mixed && self.0 != Some(Shelf::Local)
    }

    /// The two flags, offered while the page mixes both shelves and kept while one is on, so a
    /// filter that emptied the page can still be turned off.
    pub(crate) fn axes(self, mixed: bool) -> Vec<Filter> {
        if !mixed && !self.active() {
            return Vec::new();
        }
        vec![
            Filter::Flag(FlagAxis {
                key: LOCAL_ONLY,
                label: t!("filter-local-only"),
                on: self.0 == Some(Shelf::Local),
            }),
            Filter::Flag(FlagAxis {
                key: STREAMING_ONLY,
                label: t!("filter-streaming-only"),
                on: self.0 == Some(Shelf::Streaming),
            }),
        ]
    }

    /// Applies a change aimed at one of the two flags, and says whether it was one. A reset is
    /// left to the caller, which clears its other filters along with this one.
    pub(crate) fn filter(&mut self, change: &FilterChange) -> bool {
        let (shelf, on) = match change {
            FilterChange::Flag(LOCAL_ONLY, on) => (Shelf::Local, *on),
            FilterChange::Flag(STREAMING_ONLY, on) => (Shelf::Streaming, *on),
            _ => return false,
        };
        match on {
            true => self.0 = Some(shelf),
            false if self.0 == Some(shelf) => self.0 = None,
            false => {}
        }
        true
    }
}

/// Whether either shelf still has `part` on its way.
pub(crate) fn waiting(library: &Library, part: state::LibraryPart, cx: &App) -> bool {
    library.waiting(Shelf::Streaming, part, cx) || library.waiting(Shelf::Local, part, cx)
}
