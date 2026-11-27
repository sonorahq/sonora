use gpui::prelude::*;
use gpui::{AnyElement, App, Div, ElementId, Entity, Pixels, Window, div, px};

use state::{AppSettings, FilterValue, Playback};
use ui::ActiveTheme as _;
use ui::{
    Button, Filter, FilterChange, Listing, Scrollbar, Table, TableSource, TableState, Viewport,
    scrolled,
};

use crate::chrome::Chrome;
use crate::shared::cells;
use crate::shared::tracks::{self, TrackSource};

/// The width of the border around a framed table, which starts its rows this much lower than
/// the box the page measures the scroll from.
const TABLE_BORDER: Pixels = px(1.);

/// Aligns a page heading and its controls.
pub(crate) fn row(leading: impl IntoElement, trailing: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_x_4()
        .justify_between()
        .child(leading)
        .child(trailing)
        .into_any_element()
}

/// Creates a small directional button used by paged decks and horizontal shelves.
pub(crate) fn arrow_button(id: impl Into<ElementId>, previous: bool) -> Button {
    Button::new(id)
        .small()
        .outline()
        .icon(match previous {
            true => "icons/chevron-left.svg",
            false => "icons/chevron-right.svg",
        })
        .tooltip(match previous {
            true => "common-previous",
            false => "common-next",
        })
}

pub(crate) fn store(
    settings: &Entity<AppSettings>,
    table: &dyn Listing,
    layout_key: &str,
    sort_key: &str,
    cx: &mut App,
) {
    let layout = table.layout(cx);
    let sorting = table.sorting(cx);
    let axes = table.filters(cx);

    settings.update(cx, |settings, cx| {
        settings.set_table(layout_key, layout, cx);
        settings.set_sorting(sort_key, sorting, cx);
        // Axes not currently offered keep their saved values while metadata loads.
        let mut stored = settings.filters(sort_key).unwrap_or_default();
        for axis in axes {
            let (key, value) = match axis {
                Filter::Range(axis) => (
                    axis.key,
                    (!axis.whole()).then_some(FilterValue::Range(axis.value.0, axis.value.1)),
                ),
                Filter::Flag(axis) => (axis.key, axis.on.then_some(FilterValue::Flag(true))),
                Filter::Choice(axis) => {
                    let picked: Vec<_> =
                        axis.picked().map(|option| option.id.to_string()).collect();
                    (
                        axis.key,
                        (!picked.is_empty()).then_some(FilterValue::Choice(picked)),
                    )
                }
            };
            match value {
                Some(value) => stored.insert(key.to_owned(), value),
                None => stored.remove(key),
            };
        }
        settings.set_filters(sort_key, stored, cx);
    });
}

/// Applies a filter edit. Clears its saved value first so removing an axis cannot retain stale picks.
pub(crate) fn filter(
    settings: &Entity<AppSettings>,
    table: &dyn Listing,
    key: &str,
    change: FilterChange,
    cx: &mut App,
) {
    if matches!(change, FilterChange::Reset | FilterChange::Choice(..)) {
        settings.update(cx, |settings, cx| {
            let mut filters = settings.filters(key).unwrap_or_default();
            match &change {
                FilterChange::Choice(key, ..) => {
                    filters.remove(*key);
                }
                _ => filters.retain(|_, value| matches!(value, FilterValue::Text(_))),
            }
            settings.set_filters(key, filters, cx);
        });
    }
    table.filter(change, cx);
}

/// Applies the stored axes a table has not narrowed itself yet. Ranges clamp to the current
/// bounds, so a span saved before its rows arrived still lands, while one the library outgrew
/// stays whole instead of filtering everything out. Stored keys the table no longer names are
/// ignored.
pub(crate) fn restore(
    settings: &Entity<AppSettings>,
    table: &dyn Listing,
    key: &str,
    cx: &mut App,
) {
    let Some(stored) = settings.read(cx).filters(key) else {
        return;
    };
    for axis in table.filters(cx) {
        match axis {
            Filter::Range(axis) => {
                let Some(FilterValue::Range(low, high)) = stored.get(axis.key) else {
                    continue;
                };
                let value = (
                    low.clamp(axis.bounds.0, axis.bounds.1),
                    high.clamp(axis.bounds.0, axis.bounds.1),
                );
                if value != axis.bounds && value != axis.value {
                    table.filter(FilterChange::Range(axis.key, Some(value)), cx);
                }
            }
            Filter::Flag(axis) => {
                let Some(FilterValue::Flag(on)) = stored.get(axis.key) else {
                    continue;
                };
                if *on != axis.on {
                    table.filter(FilterChange::Flag(axis.key, *on), cx);
                }
            }
            Filter::Choice(axis) => {
                let Some(FilterValue::Choice(picked)) = stored.get(axis.key) else {
                    continue;
                };
                // Unknown choices remain selected and clearable while metadata loads.
                for id in picked {
                    if !axis.picked().any(|option| option.id == id.as_str()) {
                        table.filter(FilterChange::Choice(axis.key, id.clone().into(), true), cx);
                    }
                }
                for option in axis
                    .picked()
                    .filter(|option| !picked.iter().any(|id| option.id == id.as_str()))
                {
                    table.filter(FilterChange::Choice(axis.key, option.id.clone(), false), cx);
                }
            }
        }
    }
}

pub(crate) fn reserved(inset: Pixels) -> Pixels {
    inset * 2. + px(2.)
}

pub(crate) fn play(
    table: &Entity<TableState<TrackSource>>,
    playback: &Entity<Playback>,
    display: usize,
    cx: &mut App,
) {
    let queued = tracks::ordered(table, cx);
    // A row that cannot play while the network is gone stays inert, with its tooltip saying why.
    let unreachable = queued
        .get(display)
        .and_then(|track| playback.read(cx).unreachable_reason(track, cx));
    if unreachable.is_some() {
        return;
    }
    let from = tracks::whence(table, cx);
    playback.update(cx, |playback, cx| playback.start(queued, display, from, cx));
}

pub(crate) fn play_or_toggle(
    table: &Entity<TableState<TrackSource>>,
    playback: &Entity<Playback>,
    display: usize,
    cx: &mut App,
) {
    let queued = tracks::ordered(table, cx);
    let Some(track) = queued.get(display) else {
        return;
    };
    let current = playback.read(cx).track();
    let same = current.and_then(|track| track.id.as_deref()) == track.id.as_deref();
    match same {
        true => playback.update(cx, |playback, cx| playback.toggle_play(cx)),
        false => play(table, playback, display, cx),
    }
}

pub(crate) fn resize(
    table: &dyn Listing,
    width: &mut Pixels,
    inset: Pixels,
    window: &Window,
    cx: &mut App,
) {
    let next = cells::content_width(window, reserved(inset), cx);
    if (next - *width).abs() < px(0.5) {
        return;
    }
    *width = next;
    table.set_width(next, cx);
}

/// Whether a table that fills the page stands framed instead, inset with rounded corners and a
/// border like the artist page's. It does once the content area outgrows `metrics.page` and the
/// page stands centred.
pub(crate) fn framed(window: &Window, cx: &App) -> bool {
    Chrome::margin(window, cx) > Pixels::ZERO
}

/// The width of a table that fills the page. Once the table is `framed` it leaves room for the
/// frame's inset and border.
pub(crate) fn bleed_width(window: &Window, cx: &App) -> Pixels {
    let reserved = match framed(window, cx) {
        true => reserved(cx.theme().metrics.inset),
        false => Pixels::ZERO,
    };
    cells::content_width(window, reserved, cx)
}

/// Wraps a table that fills the page and draws the frame around it when `framed` is true. The
/// table's width has to come from `bleed_width` so its rows match the frame.
pub(crate) fn bleed<S: TableSource>(table: Table<S>, framed: bool, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .when(framed, |this| this.px(theme.metrics.inset))
        .child(match framed {
            true => table
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border),
            // Unframed, the head sticks to the top of the page, so it rounds with the page.
            false => table.stuck_corners(crate::chrome::inner_corners(cx)),
        })
}

/// How far below the top of a page holding a full-page table its scrollbar starts, which is
/// under the table's head that sticks to the top of the page as it scrolls.
pub(crate) fn below_head(window: &Window, cx: &App) -> Pixels {
    ui::snapped(cx.theme().metrics.header, window).max(Chrome::sheet_corners(cx).top_right)
}

/// Moves a viewport measured from a framed table's outer edge onto its rows, past the border.
pub(crate) fn past_border(viewport: Viewport) -> Viewport {
    Viewport {
        top: (viewport.top - TABLE_BORDER).max(Pixels::ZERO),
        ..viewport
    }
}

/// The viewport of a table below the page hero, with the band the player bar covers marked.
pub(crate) fn viewport(
    scrollbar: &Entity<Scrollbar>,
    inset: Pixels,
    window: &Window,
    cx: &App,
) -> Viewport {
    let scrollbar = scrollbar.read(cx);
    scrollbar.quantize(window);
    let scroll = scrollbar.scroll();
    let hero = scroll
        .bounds_for_item(0)
        .map(|bounds| bounds.size.height)
        .unwrap_or_default();
    let visible = scroll.bounds().size.height;

    Viewport::measured(scrolled(scroll) - inset - hero, visible, window).covered(Chrome::bottom(cx))
}
