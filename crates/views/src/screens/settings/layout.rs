//! Where settings put things inside the workspace's content area. The header with the search
//! field and the page tabs floats over the top of the area, and one content column is centred
//! under it. The header measures itself and every page starts below what it measured, so the
//! header, the pages and the equalizer all work the places out here from the same numbers.

use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{App, Div, Entity, Global, Pixels, SharedString, Window, div, px};
use router::{Destination, SettingsPage};
use ui::{ActiveTheme as _, Button, Room, Scrollbar, Scroller, effects, heading};

use crate::chrome::Chrome;

/// How wide the content column grows at most.
const COLUMN: Pixels = px(640.);
/// How far past the header the rows keep dissolving, so the handoff under its blur has no hard
/// edge where sharp content comes out of the haze.
const FADE_TAIL: Pixels = px(48.);

/// The places of the settings parts in the content area.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    /// How far the content column and the header keep from the area's sides.
    pub(crate) margin: Pixels,
    /// How wide the content column is.
    pub(crate) width: Pixels,
    /// How tall the header measured last, which is where a page's title starts.
    pub(crate) top: Pixels,
    /// Whether the header has measured itself yet. Until it has, `top` is a guess.
    pub(crate) measured: bool,
}

/// The header's height as it last measured itself.
#[derive(Default)]
struct HeaderHeight(Option<Pixels>);

impl Global for HeaderHeight {}

impl Layout {
    /// The places for the content area as the workspace last published it. The column is as
    /// wide as the area leaves after the margins, up to `COLUMN`.
    pub(crate) fn of(window: &Window, cx: &App) -> Self {
        let metrics = cx.theme().metrics;
        let margin = match Chrome::room(window, cx) {
            Room::Tight | Room::Snug => metrics.pad * 2.,
            Room::Roomy | Room::Wide | Room::Vast => metrics.inset,
        };
        let content = Chrome::content(window, cx);
        let header = cx.try_global::<HeaderHeight>().and_then(|height| height.0);

        Self {
            margin,
            width: (content - margin * 2.).clamp(Pixels::ZERO, COLUMN),
            top: header.unwrap_or(Pixels::ZERO),
            measured: header.is_some(),
        }
    }

    /// The ladder step of the content column, which the pages lay their rows and cards out by.
    pub(crate) fn room(&self) -> Room {
        Room::of(self.width)
    }

    /// Takes the header's measured height. Returns true when it changed, so the caller knows to
    /// redraw the pages that start below it.
    pub(crate) fn set_header(height: Pixels, cx: &mut App) -> bool {
        let next = Some(height);
        if cx.try_global::<HeaderHeight>().map(|header| header.0) == Some(next) {
            return false;
        }
        cx.set_global(HeaderHeight(next));
        true
    }

    /// How far below the top of the scrolling page content stands clear of the fade, which is
    /// where a revealed setting or section is put.
    pub(crate) fn clear(&self) -> Pixels {
        self.top + FADE_TAIL
    }

    /// Uses the original settings treatment across the full header and its tail. The scrolling
    /// region starts at the content area's top so the fade stays aligned with the header blur.
    pub(crate) fn fade<E: Styled>(&self, element: E) -> E {
        match effects() {
            true => element.fade_edges(self.clear(), Pixels::ZERO),
            false => element,
        }
    }
}

/// The top of a page: the back link to the tab a drill-in opened from, and the title, which is
/// `named` when given and otherwise the name of the tab's first page or the drill-in's own.
fn title(destination: &Destination, named: Option<SharedString>, cx: &App) -> Div {
    let theme = cx.theme();
    let (parent, page) = match destination {
        Destination::Equalizer => (Some(SettingsPage::Sound), "settings-equalizer"),
        Destination::Settings(page) => match page.parent() {
            Some(parent) => (Some(parent), page.key()),
            None => (None, page.tab().lead().key()),
        },
        _ => (None, "nav-settings"),
    };

    div()
        .flex()
        .flex_col()
        .items_start()
        .children(parent.map(|parent| {
            Button::new("settings-back")
                .ghost()
                .small()
                .icon("icons/chevron-left.svg")
                .label(i18n::lookup(parent.tab().key(), None))
                .tint(theme.muted_foreground)
                .ml(-theme.metrics.pad)
                .on_click(move |_, _, cx| router::up(Destination::Settings(parent), cx))
        }))
        .child(headline(
            named.unwrap_or_else(|| i18n::lookup(page, None)),
            cx,
        ))
}

/// The title a later page of a tab opens on, as large as the tab's own.
pub(crate) fn section_title(page: SettingsPage, cx: &App) -> Div {
    headline(i18n::lookup(page.key(), None), cx)
}

/// A page's large title, standing as tall as a field, and the top heading screen readers find on
/// the page.
fn headline(label: SharedString, cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .h(cx.theme().metrics.field)
        .child(
            heading(label.clone(), cx)
                .id(SharedString::from(format!("settings-title-{label}")))
                .role(Role::Heading)
                .aria_level(1)
                .aria_label(label),
        )
}

/// Lays a settings page out in its column, centred in the content area: the title, then `body`,
/// scrolling together from just under the floating header, into which they dissolve.
/// `destination` names the page for its title unless `named` gives one. The page sits out the
/// frames before the header has measured itself, so it never jumps.
pub(crate) fn frame(
    destination: &Destination,
    named: Option<SharedString>,
    id: &'static str,
    scrollbar: &Entity<Scrollbar>,
    body: impl IntoElement,
    window: &Window,
    cx: &mut App,
) -> Div {
    let layout = Layout::of(window, cx);
    let theme = *cx.theme();
    scrollbar.update(cx, |bar, cx| {
        bar.set_track_top(layout.top, cx);
    });
    div()
        .relative()
        .size_full()
        .when(!layout.measured, |page| page.invisible())
        .child(
            layout
                .fade(Scroller::new(id, scrollbar).bottom_inset(crate::chrome::Chrome::bottom(cx)))
                .flex()
                .flex_col()
                .items_center()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .w(layout.width)
                        .pt(layout.top)
                        .pb(theme.metrics.inset * 2.)
                        .child(title(destination, named, cx).mb(theme.metrics.inset))
                        .child(body),
                ),
        )
}
