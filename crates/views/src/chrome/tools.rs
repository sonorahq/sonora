use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, div};
use ui::{
    ActiveTheme as _, Button, Filter, FilterChange, MenuItem, Mode, Picker, Popovers,
    RangeScrubber, RangeState, Sort, SortAxis, Text, Toggle, eyebrow,
};

const COLUMNS: &str = "columns";
const FILTERS: &str = "filters";
const SORTS: &str = "sorts";

#[derive(Default)]
pub(crate) struct Sliders(RefCell<Vec<(&'static str, RangeState)>>);

impl Sliders {
    fn state(&self, key: &'static str) -> RangeState {
        let mut cache = self.0.borrow_mut();
        if let Some((_, state)) = cache.iter().find(|(known, _)| *known == key) {
            return state.clone();
        }

        let state = RangeState::new(key);
        cache.push((key, state.clone()));
        state
    }
}

pub(crate) fn columns(
    group: &Popovers,
    toggles: Vec<Toggle>,
    switch: impl Fn(&'static str, &mut App) + 'static,
) -> AnyElement {
    let switch = Rc::new(switch);

    Picker::icon(COLUMNS, group, "icons/columns-3.svg")
        .tooltip("tool-columns")
        .sticky()
        .items(toggles.into_iter().map(move |toggle| {
            let key = toggle.key;
            let switch = switch.clone();

            MenuItem::new(key, toggle.label)
                .selected(toggle.visible)
                .on_click(move |_, _, cx| switch(key, cx))
        }))
        .into_any_element()
}

pub(crate) fn sorts(
    group: &Popovers,
    axes: Vec<SortAxis>,
    rank: impl Fn(&'static str, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = *cx.theme();
    let sorted = axes.iter().any(|axis| axis.order.is_some());
    let rank = Rc::new(rank);

    Picker::icon(SORTS, group, "icons/arrow-up-down.svg")
        .tooltip("tool-sort")
        .sticky()
        .tint(match sorted {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .items(axes.into_iter().map(move |axis| {
            let key = axis.key;
            let rank = rank.clone();
            let arrow = axis.order.map(|order| match order {
                Sort::Ascending => "icons/chevron-up.svg",
                Sort::Descending => "icons/chevron-down.svg",
            });

            MenuItem::new(key, axis.label)
                .selected(axis.order.is_some())
                .when_some(arrow, MenuItem::icon)
                .on_click(move |_, _, cx| rank(key, cx))
        }))
        .into_any_element()
}

pub(crate) fn filters(
    group: &Popovers,
    sliders: &Sliders,
    axes: Vec<Filter>,
    filter_fn: impl Fn(FilterChange, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = *cx.theme();
    let narrowed = axes.iter().any(|axis| axis.narrowed());
    let filter_fn = Rc::new(filter_fn);

    let items = axes.iter().map(|axis| match axis {
        Filter::Range(axis) => {
            let key = axis.key;
            let unit = axis.unit;
            let copy = axis.clone();
            let filter_fn = filter_fn.clone();
            let state = sliders.state(key);

            MenuItem::new(key, axis.label.clone()).content(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .py_1()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(eyebrow(axis.label.clone(), cx))
                            .child(
                                div()
                                    .text_size(theme.text(Text::Small))
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{} - {}",
                                        unit.say(axis.value.0),
                                        unit.say(axis.value.1)
                                    )),
                            ),
                    )
                    .child(
                        RangeScrubber::new(&state, axis.share())
                            .stops(axis.stops())
                            .colors(theme.progress_bar, theme.muted, theme.foreground)
                            .on_change(move |share: &(f32, f32), _, cx| {
                                filter_fn(FilterChange::Range(key, copy.at(*share)), cx);
                            }),
                    ),
            )
        }
        Filter::Flag(axis) => {
            let key = axis.key;
            let on = axis.on;
            let filter_fn = filter_fn.clone();

            MenuItem::new(key, axis.label.clone())
                .selected(on)
                .on_click(move |_, _, cx| filter_fn(FilterChange::Flag(key, !on), cx))
        }
    });

    let reset = filter_fn.clone();

    Picker::icon(FILTERS, group, "icons/funnel.svg")
        .tooltip("tool-filters")
        .sticky()
        .width(Picker::WIDE)
        .tint(match narrowed {
            true => theme.primary,
            false => theme.muted_foreground,
        })
        .items(items)
        .item(MenuItem::separator("filters-end"))
        .item(
            MenuItem::new("filters-reset", i18n::t!("filter-reset"))
                .on_click(move |_, _, cx| reset(FilterChange::Reset, cx)),
        )
        .into_any_element()
}

pub(crate) fn views(
    group: &Popovers,
    mode: Mode,
    shift: impl Fn(Mode, &mut App) + 'static,
) -> AnyElement {
    let next = match mode {
        Mode::List => Mode::Grid,
        Mode::Grid => Mode::List,
    };
    let group = group.clone();

    Button::new("view-toggle")
        .icon(next.icon())
        .tooltip(next.key())
        .small()
        .ghost()
        .on_click(move |_, _, cx| {
            group.close();
            shift(next, cx);
        })
        .into_any_element()
}
