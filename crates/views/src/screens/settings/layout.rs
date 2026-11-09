//! Where settings put things inside the workspace's content area. The header with the search
//! field and the page tabs floats over the top of the area, and one content column is centred
//! under it. The header measures itself and every page starts below what it measured, so the
//! header, the pages and the equalizer all work the places out here from the same numbers.

use router::{Destination, SettingsPage};
use rune::{Cx, El, ScrollSpec, Transition, col, el, row, text};
use ui::{Button, Room, Text, Theme, VeilEdge, Weight, effects, font, veil};

use super::header;
use crate::{Ui, act};

/// How wide the content column grows at most.
pub(crate) const COLUMN: f32 = 640.;
/// How far past the header the rows keep dissolving, so the handoff under its blur has no hard
/// edge where sharp content comes out of the haze.
const FADE_TAIL: f32 = 48.;
/// How far the rows are blurred where they pass under the header.
const BLUR: f32 = 1.;
/// The key the header's column measures under.
pub(crate) const HEADER: &str = "settings-header-column";
/// The key the content column measures under.
const MEASURE: &str = "settings-column";

/// The places of the settings parts in the content area.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    /// How far the content column and the header keep from the area's sides.
    pub(crate) margin: f32,
    /// How wide the content column is.
    pub(crate) width: f32,
    /// How tall the header measured last, which is where a page's title starts.
    pub(crate) top: f32,
}

impl Layout {
    /// The places for the content area, from the widths and the header the last frame laid out.
    pub(crate) fn of(ui: &Ui, cx: &Cx<Ui>) -> Self {
        let theme = &ui.theme;
        let margin = match crate::content_room(cx) {
            Room::Tight | Room::Snug => theme.metrics.pad * 2.,
            Room::Roomy | Room::Wide | Room::Vast => theme.metrics.inset,
        };
        let content = cx
            .layout_with("content", |rect| rect[2].round())
            .unwrap_or(COLUMN + margin * 2.);
        let top = cx
            .layout_with(HEADER, |rect| rect[3].round())
            .filter(|height| *height > 0.)
            .unwrap_or_else(|| guess(theme));
        Self {
            margin,
            width: (content - margin * 2.).clamp(0., COLUMN),
            top,
        }
    }

    /// The ladder step of the content column, which the pages lay their rows and cards out by.
    pub(crate) fn room(&self) -> Room {
        Room::of(self.width)
    }

    /// How far below the top of the scrolling page content stands clear of the fade, which is
    /// where a revealed setting or section is put.
    pub(crate) fn clear(&self) -> f32 {
        self.top + FADE_TAIL
    }
}

/// The width of the content column, re-running the view only when it changes, for the cards
/// that size what they draw by it.
pub(crate) fn column_width(cx: &Cx<Ui>) -> Option<f32> {
    cx.layout_with(MEASURE, |rect| rect[2].round())
}

/// A settings page in its column, centred in the content area under the floating header: the
/// title, then `body`, scrolling together from just under the header, into which they dissolve.
/// `destination` names the page for its title unless `named` gives one. `scroll` keys the
/// scrolling page, so a new key starts a fresh page at the top that fades in over the old one.
/// The equalizer page lays itself out through this too.
pub(crate) fn frame(
    ui: &Ui,
    cx: &mut Cx<Ui>,
    layer: &mut Vec<El>,
    destination: &Destination,
    named: Option<String>,
    scroll: String,
    body: El,
) -> El {
    let theme = &ui.theme;
    let layout = Layout::of(ui, cx);
    let column = col()
        .key(MEASURE)
        .w_full()
        .max_w(COLUMN)
        .min_w(0.)
        .pt(layout.top)
        .pb(theme.metrics.inset * 2.)
        .child(title(theme, destination, named).mb(theme.metrics.inset))
        .child(body);
    let mut page = col()
        .key(scroll)
        .pin(Some(0.), Some(0.), Some(0.), Some(0.))
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_margin(layout.clear())
        .transition(Transition::opacity())
        .child(
            col()
                .key("settings-centre")
                .w_full()
                .items_center()
                .px(layout.margin)
                .child(column),
        );
    if effects() {
        page = page.clip_fade(0., layout.clear(), 0., 0.);
    }
    el().key("settings-page")
        .w_full()
        .h_full()
        .child(page)
        .child(floating(ui, cx, layer, &layout))
}

/// The header pinned over the top of the area, with the haze under it that blurs the rows
/// passing beneath. A see-through page has content of its own passing under the header and
/// reads worse without the haze, so only the flat fallback is dropped there.
fn floating(ui: &Ui, cx: &mut Cx<Ui>, layer: &mut Vec<El>, layout: &Layout) -> El {
    let theme = &ui.theme;
    let mut band = el()
        .key("settings-header")
        .pin(Some(0.), Some(0.), Some(0.), None)
        .h(layout.top);
    if effects() || !theme.transparent {
        band = band.child(veil(
            VeilEdge::Top,
            layout.top,
            BLUR,
            theme.background,
            None,
        ));
    }
    band.child(
        row()
            .key("settings-header-row")
            .pin(Some(0.), Some(0.), Some(0.), None)
            .justify_center()
            .px(layout.margin)
            .child(header::view(ui, cx, layer, layout)),
    )
}

/// The top of a page: the back link to the tab a drill-in opened from, and the title, which is
/// `named` when given and otherwise the name of the tab's first page or the drill-in's own.
fn title(theme: &Theme, destination: &Destination, named: Option<String>) -> El {
    let (parent, page) = match destination {
        Destination::Equalizer => (Some(SettingsPage::Sound), "settings-equalizer"),
        Destination::Settings(page) => match page.parent() {
            Some(parent) => (Some(parent), page.key()),
            None => (None, page.tab().lead().key()),
        },
        _ => (None, "nav-settings"),
    };
    let label = named.unwrap_or_else(|| i18n::lookup(page, None).into_owned());
    col()
        .key("settings-title")
        .items_start()
        .children(parent.map(|parent| {
            Button::new("settings-back")
                .ghost()
                .small()
                .icon("icons/chevron-left.svg")
                .label(i18n::lookup(parent.tab().key(), None))
                .tint(theme.muted_foreground)
                .on_press(act(move |ui: &mut Ui| {
                    ui.up(Destination::Settings(parent));
                }))
                .build(theme)
                .ml(-theme.metrics.pad)
        }))
        .child(headline(theme, label))
}

/// The title a later page of a tab opens on, as large as the tab's own.
pub(crate) fn section_title(theme: &Theme, page: SettingsPage) -> El {
    headline(theme, i18n::lookup(page.key(), None).into_owned())
        .key(format!("settings-section-{}", page.id()))
}

/// A page's large title, standing as tall as a field, and the top heading screen readers find
/// on the page.
fn headline(theme: &Theme, label: String) -> El {
    row()
        .key("headline")
        .center_y()
        .h(theme.metrics.field)
        .child(
            text(label.clone())
                .key(format!("settings-title-{label}"))
                .role("heading")
                .level(1)
                .label(label)
                .font(font(theme, Text::Title, Weight::SEMIBOLD))
                .color(theme.foreground.color()),
        )
}

/// What the header measures before it has laid out once: its padding, the field, the gap and
/// the tab bar.
fn guess(theme: &Theme) -> f32 {
    let bar = theme.metrics.control_small + theme.font_size * 0.5 + 2.;
    theme.metrics.inset * 2. + theme.metrics.field + header::GAP + bar
}
