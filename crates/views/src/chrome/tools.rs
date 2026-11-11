use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, Entity, EntityId, Hsla, SharedString, div, px, svg};
use ui::{
    ActiveTheme as _, Button, Choice, ChoiceAxis, Filter, FilterChange, FlagAxis, Menu, MenuItem,
    MenuSearch, Mode, Picker, Popovers, RangeAxis, RangeScrubber, RangeState, Scrollbar, Sort,
    SortAxis, SubmenuState, Text, Tipped as _, Unit, eyebrow,
};

const FILTERS: &str = "filters";
const SORTS: &str = "sorts";

/// The shared background for active search and filter controls.
pub(crate) fn active_filter_background(cx: &App) -> Hsla {
    cx.theme().primary.opacity(0.2)
}

/// Creates a localized flag axis from its Fluent key.
pub(crate) fn flag(key: &'static str, on: bool) -> Filter {
    Filter::Flag(FlagAxis {
        key,
        label: i18n::lookup(key, None),
        on,
    })
}

/// How wide a choice filter's submenu is, and how tall before it scrolls.
const CHOICE_WIDTH: f32 = 220.;
const CHOICE_HEIGHT: f32 = 320.;

/// What one choice filter's submenu keeps between frames: whether it is open, what is typed in
/// its search, and how far its list is scrolled.
#[derive(Clone)]
struct ChoiceMenu {
    state: SubmenuState,
    search: MenuSearch,
    scrollbar: Entity<Scrollbar>,
}

#[derive(Default)]
pub(crate) struct Sliders {
    ranges: RefCell<Vec<(&'static str, RangeState)>>,
    choices: Vec<(&'static str, ChoiceMenu)>,
}

impl Sliders {
    /// Gives the filters the submenus for the choice axes `keys` name. A screen whose table
    /// offers a choice axis must build its sliders this way, or the axis stays out of the menu.
    pub(crate) fn with_choices(
        keys: impl IntoIterator<Item = &'static str>,
        watching: EntityId,
        cx: &mut App,
    ) -> Self {
        let choices = keys
            .into_iter()
            .map(|key| {
                let menu = ChoiceMenu {
                    state: SubmenuState::instant(),
                    search: MenuSearch::new("menu-search-filter", cx),
                    scrollbar: cx.new(|_| Scrollbar::inset().watching(watching)),
                };
                (key, menu)
            })
            .collect();
        Self {
            ranges: RefCell::default(),
            choices,
        }
    }

    fn choice_menu(&self, key: &'static str) -> Option<ChoiceMenu> {
        self.choices
            .iter()
            .find(|(known, _)| *known == key)
            .map(|(_, menu)| menu.clone())
    }

    fn state(&self, key: &'static str) -> RangeState {
        let mut cache = self.ranges.borrow_mut();
        if let Some((_, state)) = cache.iter().find(|(known, _)| *known == key) {
            return state.clone();
        }

        let state = RangeState::new(key);
        cache.push((key, state.clone()));
        state
    }
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
        .large()
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
    let narrowed = axes.iter().any(Filter::narrowed);
    let text = |words: SharedString| {
        div()
            .min_w_0()
            .truncate()
            .text_size(theme.text(Text::Small))
            .child(words)
            .into_any_element()
    };
    let active = axes.iter().filter(|axis| axis.narrowed()).map(|axis| {
        let (key, label, content, icon_only) = match axis {
            Filter::Range(axis) => (
                axis.key,
                axis.label.clone(),
                text(compact_range(axis)),
                false,
            ),
            Filter::Choice(axis) => {
                let picked: Vec<_> = axis.picked().map(|option| option.label.as_ref()).collect();
                let words = format!("{}: {}", axis.label, picked.join(", "));
                (axis.key, axis.label.clone(), text(words.into()), false)
            }
            Filter::Flag(axis) => {
                let icon = filter_icon(axis.key);
                (
                    axis.key,
                    axis.label.clone(),
                    match icon {
                        Some(icon) => svg()
                            .path(icons::path(icon))
                            .size(theme.text(Text::Label))
                            .flex_none()
                            .text_color(theme.primary)
                            .into_any_element(),
                        None => div().child(axis.label.clone()).into_any_element(),
                    },
                    icon.is_some(),
                )
            }
        };

        div()
            .id(key)
            .flex()
            .items_center()
            .h(theme.metrics.control_small)
            .when(icon_only, |this| {
                this.w(theme.metrics.control_small).justify_center()
            })
            .when(!icon_only, |this| {
                this.px_2().max_w(theme.metrics.control * 4.)
            })
            .rounded_full()
            .bg(active_filter_background(cx))
            .tip_label(label, ui::Perch::Below)
            .child(content)
            .into_any_element()
    });
    let active = div().flex().items_center().gap_1().children(active);
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
                                ui::spoken(
                                    gpui::SharedString::from(format!("{key}-range")),
                                    format!(
                                        "{} - {}",
                                        unit.say(axis.value.0),
                                        unit.say(axis.value.1)
                                    ),
                                )
                                .text_size(theme.text(Text::Small))
                                .text_color(theme.muted_foreground),
                            ),
                    )
                    .child(
                        RangeScrubber::new(&state, axis.share())
                            .name(axis.label.clone())
                            .values(unit.say(axis.value.0), unit.say(axis.value.1))
                            .stops(axis.stops())
                            .colors(theme.progress_bar, theme.muted, theme.foreground)
                            .on_change(move |share: &(f32, f32), _, cx| {
                                filter_fn(FilterChange::Range(key, copy.narrowing(*share)), cx);
                            }),
                    ),
            )
        }
        Filter::Flag(axis) => {
            let key = axis.key;
            let on = axis.on;
            let filter_fn = filter_fn.clone();

            MenuItem::new(key, axis.label.clone())
                .when_some(filter_icon(key), MenuItem::icon)
                .selected(on)
                .on_click(move |_, _, cx| filter_fn(FilterChange::Flag(key, !on), cx))
        }
        Filter::Choice(axis) => match sliders.choice_menu(axis.key) {
            Some(menu) => choice_item(axis, menu, &filter_fn, cx),
            None => MenuItem::new(axis.key, axis.label.clone()).disabled(),
        },
    });

    let reset = filter_fn.clone();
    let open = group.shows(FILTERS);
    let trigger = Button::new("filters-picker")
        .icon("icons/funnel.svg")
        .border_1()
        .border_color(match (open, narrowed) {
            (true, _) => theme.primary,
            (false, true) => theme.border,
            (false, false) => theme.border.opacity(0.),
        })
        .when_else(
            narrowed,
            |button| button.outline().rounded_full().pr_0().leading(active),
            Button::ghost,
        );
    Picker::icon(FILTERS, group, "icons/funnel.svg")
        .button(trigger)
        .tooltip("tool-filters")
        .sticky()
        .width(Picker::WIDE)
        .tint(theme.foreground)
        .items(items)
        .item(MenuItem::separator("filters-end"))
        .item(
            MenuItem::new("filters-reset", i18n::t!("filter-reset"))
                .on_click(move |_, _, cx| reset(FilterChange::Reset, cx)),
        )
        .into_any_element()
}

/// A menu row for a choice axis, opening a searchable list of its options that stays open as
/// each is picked.
fn choice_item(
    axis: &ChoiceAxis,
    menu: ChoiceMenu,
    filter_fn: &Rc<impl Fn(FilterChange, &mut App) + 'static>,
    cx: &App,
) -> MenuItem {
    let key = axis.key;
    let query = menu.search.query(cx);
    let found: Vec<&Choice> = axis
        .options
        .iter()
        .filter(|option| option.label.to_lowercase().contains(&query))
        .collect();
    let unmatched = found.is_empty();
    let rows = found.into_iter().map(|option| {
        let id = option.id.clone();
        let on = option.on;
        let filter_fn = filter_fn.clone();
        MenuItem::new(
            SharedString::from(format!("{key}-{id}")),
            option.label.clone(),
        )
        .checked(on)
        .stays_open()
        .on_click(move |_, _, cx| filter_fn(FilterChange::Choice(key, id.clone(), !on), cx))
    });
    let submenu = Menu::new(SharedString::from(format!("{key}-menu")))
        .w(px(CHOICE_WIDTH))
        .max_h(px(CHOICE_HEIGHT))
        .scrollbar(menu.scrollbar)
        .search(menu.search)
        .when(unmatched, |menu| {
            menu.item(
                MenuItem::new(
                    SharedString::from(format!("{key}-none")),
                    i18n::t!("search-no-matches"),
                )
                .disabled(),
            )
        })
        .items(rows);
    MenuItem::new(key, axis.label.clone())
        .selected(axis.picked().next().is_some())
        .submenu(submenu, menu.state)
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
        .ghost()
        .on_click(move |_, _, cx| {
            group.close();
            shift(next, cx);
        })
        .into_any_element()
}

/// Uses the same bounds as the range control, omitting the unchanged end from its summary.
fn compact_range(axis: &RangeAxis) -> SharedString {
    let low = axis.unit.say(axis.value.0);
    let high = axis.unit.say(axis.value.1);
    let (from, to) = axis.share();
    let value = match (from > f32::EPSILON, to < 1. - f32::EPSILON) {
        (false, true) => format!("<{high}"),
        (true, false) => format!(">{low}"),
        _ => format!("{low}–{high}"),
    };
    match axis.unit {
        Unit::Clock => value.into(),
        Unit::Plain => format!("{}: {value}", axis.label).into(),
    }
}

/// Shared compact symbols for filter flags, with the localized name retained as a tooltip.
fn filter_icon(key: &str) -> Option<&'static str> {
    match key {
        "filter-hide-explicit" => Some("icons/explicit-off.svg"),
        "filter-playable" => Some("icons/circle-check.svg"),
        "filter-favorites" => Some("icons/heart-filled.svg"),
        "filter-collaborations" => Some("icons/users-round-collaboration.svg"),
        "filter-downloaded" => Some("icons/download.svg"),
        "filter-owned" => Some("icons/user-round.svg"),
        _ => None,
    }
}
