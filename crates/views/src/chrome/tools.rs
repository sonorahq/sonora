//! The page tools the title bar carries for the shown page: the sort picker, the filter picker
//! and the list/grid switch. A screen builds the ones it offers from its own state and hands
//! them to [`super::toolbar::Tools`]; each opens as one popover of the chrome's group, so only
//! one menu of the bar is open at a time.

use std::rc::Rc;

use rune::{El, Tx, col, row};
use ui::{
    Button, Filter, FilterChange, MenuItem, Mode, Picker, RangeScrubber, Sort, SortAxis, Text,
    Weight, eyebrow,
};

use crate::Ui;

/// The popover keys of the two pickers in the chrome's popover group.
const FILTERS: &str = "filters";
const SORTS: &str = "sorts";
/// The menu's padding and a row's side padding, which a range row's scrubber fits inside.
const MENU_PAD: f32 = 0.5;

/// What a sort pick asks for: cycle the order of the column keyed so.
type Rank = Rc<dyn Fn(&mut Ui, &'static str)>;
/// What a filter change asks for.
type Sift = Rc<dyn Fn(&mut Ui, FilterChange)>;

/// The sort picker: one row per column the list can be sorted by, the sorted one checked with
/// an arrow for its order. The trigger is lit while any sort is in effect. `rank` runs inside
/// [`crate::act`], so it may change the store.
pub(crate) fn sorts(
    ui: &Ui,
    axes: Vec<SortAxis>,
    rank: impl Fn(&mut Ui, &'static str) + 'static,
    layer: &mut Vec<El>,
) -> El {
    let theme = &ui.theme;
    let sorted = axes.iter().any(|axis| axis.order.is_some());
    let rank: Rank = Rc::new(rank);

    Picker::icon(
        SORTS,
        &ui.chrome.popovers,
        |ui: &mut Ui| &mut ui.chrome.popovers,
        "icons/arrow-up-down.svg",
    )
    .tooltip(i18n::t!("tool-sort"))
    .sticky()
    .tint(match sorted {
        true => theme.primary,
        false => theme.muted_foreground,
    })
    .items(axes.into_iter().map(move |axis| {
        let key = axis.key;
        let rank = rank.clone();
        let item = MenuItem::new(key, axis.label.to_string())
            .selected(axis.order.is_some())
            .on_press(crate::act(move |ui: &mut Ui| rank(ui, key)));
        match axis.order {
            Some(Sort::Ascending) => item.icon("icons/chevron-up.svg"),
            Some(Sort::Descending) => item.icon("icons/chevron-down.svg"),
            None => item,
        }
    }))
    .build(theme, layer)
}

/// The filter picker: a two-handled scrubber for each range axis, a toggle for each flag and a
/// reset under them. The trigger is lit while anything is narrowed.
pub(crate) fn filters(
    ui: &Ui,
    axes: Vec<Filter>,
    sift: impl Fn(&mut Ui, FilterChange) + 'static,
    layer: &mut Vec<El>,
) -> El {
    let theme = &ui.theme;
    let narrowed = axes.iter().any(Filter::narrowed);
    let sift: Sift = Rc::new(sift);
    let width = Picker::<Ui>::WIDE - theme.font_size * MENU_PAD * 2. - theme.metrics.pad * 2.;

    let items = axes.into_iter().map(|axis| match axis {
        Filter::Range(axis) => {
            let key = axis.key;
            let unit = axis.unit;
            let (low, high) = (unit.say(axis.value.0), unit.say(axis.value.1));
            let label = axis.label.to_string();
            let sift = sift.clone();
            let copy = axis.clone();
            let heading = row()
                .center_y()
                .justify_between()
                .gap(theme.metrics.pad)
                .child(eyebrow(&label, theme))
                .child(
                    ui::spoken(format!("{key}-range"), format!("{low} - {high}"), theme)
                        .font(ui::font(theme, Text::Small, Weight::NORMAL))
                        .color(theme.muted_foreground.color()),
                );
            let scrubber = RangeScrubber::<Ui>::new(format!("{key}-scrubber"), axis.share(), width)
                .name(label)
                .values(low.to_string(), high.to_string())
                .stops(axis.stops())
                .colors(theme.progress_bar, theme.muted, theme.foreground)
                .on_change(move |ui: &mut Ui, share: (f32, f32), _: &mut Tx| {
                    sift(ui, FilterChange::Range(key, copy.at(share)));
                    ui.settle();
                })
                .build(theme);
            MenuItem::content(
                key,
                col()
                    .gap(theme.font_size * 0.25)
                    .py(theme.font_size * 0.25)
                    .child(heading)
                    .child(scrubber),
            )
        }
        Filter::Flag(axis) => {
            let key = axis.key;
            let on = axis.on;
            let sift = sift.clone();
            MenuItem::new(key, axis.label.to_string())
                .selected(on)
                .on_press(crate::act(move |ui: &mut Ui| {
                    sift(ui, FilterChange::Flag(key, !on))
                }))
        }
    });
    let reset = sift.clone();

    Picker::icon(
        FILTERS,
        &ui.chrome.popovers,
        |ui: &mut Ui| &mut ui.chrome.popovers,
        "icons/funnel.svg",
    )
    .tooltip(i18n::t!("tool-filters"))
    .sticky()
    .width(Picker::<Ui>::WIDE)
    .tint(match narrowed {
        true => theme.primary,
        false => theme.muted_foreground,
    })
    .items(items)
    .item(MenuItem::separator("filters-end"))
    .item(
        MenuItem::new("filters-reset", i18n::t!("filter-reset")).on_press(crate::act(
            move |ui: &mut Ui| reset(ui, FilterChange::Reset),
        )),
    )
    .build(theme, layer)
}

/// The switch between a list and a grid, showing the mode it switches to. Switching closes any
/// open menu of the bar first.
pub(crate) fn views(ui: &Ui, mode: Mode, shift: impl Fn(&mut Ui, Mode) + 'static) -> El {
    let next = match mode {
        Mode::List => Mode::Grid,
        Mode::Grid => Mode::List,
    };

    Button::new("view-toggle")
        .icon(next.icon())
        .tooltip(i18n::lookup(next.key(), None))
        .small()
        .ghost()
        .on_press(crate::act(move |ui: &mut Ui| {
            ui.chrome.popovers.close();
            shift(ui, next);
        }))
        .build(&ui.theme)
}
