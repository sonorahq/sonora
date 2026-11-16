use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{App, Div, Entity, Pixels, ScrollHandle, Window, div, px};

use state::{AppSettings, FilterValue, Playback};
use ui::ActiveTheme as _;
use ui::{
    Filter, FilterChange, Listing, Table, TableSource, TableState, Viewport, quantize, scrolled,
};

use crate::chrome::Chrome;
use crate::shared::cells;
use crate::shared::tracks::{self, TrackSource};

/// The width of the border around a framed table, which starts its rows this much lower than
/// the box the page measures the scroll from.
const TABLE_BORDER: Pixels = px(1.);

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
        let filters = snapshot(axes, settings.filters(sort_key).unwrap_or_default());
        settings.set_filters(sort_key, filters, cx);
    });
}

/// Merges the current axes into the stored ones: a narrowed axis upserts its key, a whole one
/// drops it, and an axis the table does not name right now leaves storage alone. That last
/// rule is what keeps a saved year span alive while its rows are still loading, and resetting
/// a table clears its entry on the next store.
pub(crate) fn snapshot(
    axes: Vec<Filter>,
    into: HashMap<String, FilterValue>,
) -> HashMap<String, FilterValue> {
    let mut stored = into;
    for axis in axes {
        match axis {
            Filter::Range(axis) if axis.whole() => {
                stored.remove(axis.key);
            }
            Filter::Range(axis) => {
                stored.insert(
                    axis.key.to_owned(),
                    FilterValue::Range(axis.value.0, axis.value.1),
                );
            }
            Filter::Flag(axis) if axis.on => {
                stored.insert(axis.key.to_owned(), FilterValue::Flag(true));
            }
            Filter::Flag(axis) => {
                stored.remove(axis.key);
            }
        }
    }
    stored
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
                if value != axis.bounds {
                    table.filter(FilterChange::Range(axis.key, value), cx);
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
        .child(table.when(framed, |table| {
            table
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
        }))
}

/// Moves a viewport measured from a framed table's outer edge onto its rows, past the border.
pub(crate) fn past_border(viewport: Viewport) -> Viewport {
    Viewport {
        top: (viewport.top - TABLE_BORDER).max(Pixels::ZERO),
        ..viewport
    }
}

pub(crate) fn viewport(scroll: &ScrollHandle, inset: Pixels, window: &Window) -> Viewport {
    quantize(scroll, window);
    let hero = scroll
        .bounds_for_item(0)
        .map(|bounds| bounds.size.height)
        .unwrap_or_default();
    let visible = scroll.bounds().size.height;

    Viewport::measured(scrolled(scroll) - inset - hero, visible, window)
}
