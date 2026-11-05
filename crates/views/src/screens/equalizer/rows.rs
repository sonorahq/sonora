use gpui::accesskit::Role;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Entity, Focusable as _, Pixels, SharedString,
    Stateful, Window, div,
};
use i18n::t;
use music::equalizer::{
    Curve, FREQUENCY_RANGE, Filter, GAIN_RANGE, Invalid, Kind, MAX_FILTERS, OCTAVES_RANGE,
    PREAMP_RANGE, Preamp, Q_RANGE, Width, apo,
};
use state::{Outcome, Toast, Toasts};
use ui::{ActiveTheme as _, Button, Input, MenuItem, Picker, Switch, Text, vacant};

use super::{EqualizerView, FILTERS_ICON, heading};

/// One kind picker per filter row, since a popover is known by a static name.
const KINDS: [&str; MAX_FILTERS] = [
    "equalizer-kind-0",
    "equalizer-kind-1",
    "equalizer-kind-2",
    "equalizer-kind-3",
    "equalizer-kind-4",
    "equalizer-kind-5",
    "equalizer-kind-6",
    "equalizer-kind-7",
    "equalizer-kind-8",
    "equalizer-kind-9",
    "equalizer-kind-10",
    "equalizer-kind-11",
    "equalizer-kind-12",
    "equalizer-kind-13",
    "equalizer-kind-14",
    "equalizer-kind-15",
    "equalizer-kind-16",
    "equalizer-kind-17",
    "equalizer-kind-18",
    "equalizer-kind-19",
];
/// How wide each part of a filter's row is, in cover sizes, so the parts line up from row to
/// row: the kind picker, then the frequency, gain and width fields with their units inside.
const KIND: f32 = 0.86;
const FREQUENCY: f32 = 0.72;
const GAIN: f32 = 0.62;
const WIDTH: f32 = 0.62;
/// The new filter Add puts in: flat, so adding one changes nothing until it is shaped.
pub(super) const NEW_FILTER: (Kind, f64, f64, f64) = (Kind::Peak, 1_000., 0., 1.);

/// The number fields of one filter row. Each keeps its number against its end with its unit
/// inside, so the figures of every row line up.
pub(super) struct Fields {
    frequency: Entity<Input>,
    gain: Entity<Input>,
    width: Entity<Input>,
}

/// A number field, which is what a commit reads, validates and applies.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Field {
    Preamp,
    Filter(usize, Value),
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Value {
    Frequency,
    Gain,
    Width,
}

impl Fields {
    pub(super) fn new(cx: &mut Context<EqualizerView>) -> Self {
        let mut number = |unit: &'static str| {
            cx.new(|cx| Input::new(unit, cx).compact().align_end().suffix(unit))
        };
        Self {
            frequency: number("equalizer-hertz"),
            gain: number("equalizer-decibels"),
            width: number("equalizer-q"),
        }
    }
}

impl EqualizerView {
    pub(super) fn fields(&self) -> impl Iterator<Item = (Field, &Entity<Input>)> {
        let rows = self.rows.iter().enumerate().flat_map(|(index, row)| {
            [
                (Field::Filter(index, Value::Frequency), &row.frequency),
                (Field::Filter(index, Value::Gain), &row.gain),
                (Field::Filter(index, Value::Width), &row.width),
            ]
        });
        std::iter::once((Field::Preamp, &self.preamp)).chain(rows)
    }

    fn input(&self, field: Field) -> &Entity<Input> {
        match field {
            Field::Preamp => &self.preamp,
            Field::Filter(index, Value::Frequency) => &self.rows[index].frequency,
            Field::Filter(index, Value::Gain) => &self.rows[index].gain,
            Field::Filter(index, Value::Width) => &self.rows[index].width,
        }
    }

    /// Fills every field whose text differs from the playing curve once it changed, the field
    /// being typed in included, since what it held belongs to the curve that was replaced.
    pub(super) fn show_curve(&mut self, cx: &mut Context<Self>) {
        let curve = self.equalizer.read(cx).curve();
        if self.shown.as_ref() == Some(curve) {
            return;
        }
        let curve = curve.clone();
        let texts: Vec<_> = self
            .fields()
            .map(|(field, input)| (input.clone(), shown(&curve, field)))
            .collect();
        for (input, text) in texts {
            if input.read(cx).text() != text {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
        }
        let units: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, fields)| {
                let unit = match curve.filters().get(index).map(Filter::width) {
                    Some(Width::Octaves(_)) => "equalizer-octaves",
                    _ => "equalizer-q",
                };
                (fields.width.clone(), unit)
            })
            .collect();
        for (input, unit) in units {
            input.update(cx, |input, cx| input.set_suffix(Some(unit.into()), cx));
        }
        self.shown = Some(curve);
    }

    /// Applies what `field` holds when it differs from the curve, through the model's checks.
    /// A value the model refuses is put back and the reason shown.
    pub(super) fn commit(&mut self, field: Field, cx: &mut Context<Self>) {
        let input = self.input(field).clone();
        let text = input.read(cx).text().trim().to_owned();
        let curve = self.equalizer.read(cx).curve().clone();
        let current = shown(&curve, field);
        if text == current {
            return;
        }
        let dotted = text.replace(',', ".");
        let edited = dotted
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(|number| match (field, text.contains(',')) {
                // A comma is read the way an import reads it, so `12,500` is 12.5 kHz, while a
                // dot is a decimal point, so `15.625` stays 15.625 Hz.
                (Field::Filter(_, Value::Frequency), true) => {
                    apo::hertz_read(&dotted).unwrap_or(number)
                }
                _ => number,
            })
            .map(|number| edit(&curve, field, number));
        match edited {
            Some(Ok(curve)) => self.apply(curve, cx),
            refused => {
                if let Some(Err(invalid)) = refused {
                    Toasts::post(
                        Toast::new(Outcome::Warning, "equalizer-value-refused")
                            .detail(invalid_text(invalid)),
                        cx,
                    );
                }
                input.update(cx, |input, cx| input.set_text(current, cx));
            }
        }
    }

    /// Commits the field that has focus, which is what Enter does.
    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .fields()
            .find(|(_, input)| input.read(cx).focus_handle(cx).is_focused(window))
            .map(|(field, _)| field);
        if let Some(field) = focused {
            self.commit(field, cx);
        }
    }

    /// Puts the focused field back as it was and leaves it, which is what Escape does.
    pub(super) fn revert(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let curve = self.equalizer.read(cx).curve().clone();
        let texts: Vec<_> = self
            .fields()
            .filter(|(_, input)| input.read(cx).focus_handle(cx).is_focused(window))
            .map(|(field, input)| (input.clone(), shown(&curve, field)))
            .collect();
        for (input, text) in texts {
            input.update(cx, |input, cx| input.set_text(text, cx));
        }
        window.blur(cx);
    }

    /// The playing curve with filter `index` changed by `change`, or unchanged when the filter
    /// is gone.
    fn change(
        &mut self,
        index: usize,
        change: impl FnOnce(Filter) -> Filter,
        cx: &mut Context<Self>,
    ) {
        let mut curve = self.equalizer.read(cx).curve().clone();
        let Some(filter) = curve.filters_mut().get_mut(index) else {
            return;
        };
        *filter = change(*filter);
        self.apply(curve, cx);
    }

    /// One filter on one line: its switch, its kind, its frequency, gain and width, and a button
    /// that removes it. The row lights the filter's node on the graph while the pointer is over
    /// it, a press on it selects that node, and it stands marked while its node is selected.
    /// Where the line has too little room, the three values move to a line of their own. `count`
    /// is how many filters the list holds, which screen readers announce with the row.
    fn filter_row(
        &self,
        index: usize,
        filter: Filter,
        selected: bool,
        count: usize,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = *cx.theme();
        let cover = theme.metrics.cover;
        let fields = &self.rows[index];
        let enabled = Switch::new(("equalizer-filter-on", index), filter.enabled())
            .name(t!("equalizer-filter", number = index + 1))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.change(index, |filter| filter.with_enabled(!filter.enabled()), cx)
            }));
        let kind = Picker::new(KINDS[index], &self.popovers, kind_name(filter.kind()))
            .name(t!("equalizer-filter-kind", number = index + 1))
            .width(Picker::NARROW)
            .left()
            .items(Kind::ALL.map(|kind| {
                MenuItem::new(kind.id(), kind_name(kind))
                    .selected(kind == filter.kind())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(index, |filter| filter.with_kind(kind), cx)
                    }))
            }));
        let number = index + 1;
        let gain = match filter.kind().has_gain() {
            true => fields.gain.clone().into_any_element(),
            false => figure_cell(
                ("equalizer-no-gain", index),
                t!("equalizer-no-gain"),
                t!("equalizer-no-gain-spoken"),
                cover * GAIN,
                cx,
            ),
        };
        let remove = Button::new(("equalizer-remove", index))
            .small()
            .ghost()
            .icon("icons/x.svg")
            .tooltip("equalizer-remove-filter")
            .on_click(cx.listener(move |this, _, _, cx| {
                this.graph
                    .update(cx, |graph, cx| graph.remove_filter(index, cx))
            }));
        let frequency = field_box(
            ("equalizer-frequency-field", index),
            t!("equalizer-filter-frequency", number = number),
        )
        .child(fields.frequency.clone());
        let gain = field_box(
            ("equalizer-gain-field", index),
            t!("equalizer-filter-gain", number = number),
        )
        .child(gain);
        let width = field_box(
            ("equalizer-width-field", index),
            t!("equalizer-filter-width", number = number),
        )
        .child(fields.width.clone());

        let row = div()
            .id(("equalizer-filter", index))
            .role(Role::ListItem)
            .aria_label(t!("equalizer-filter", number = number))
            .aria_selected(selected)
            .aria_position_in_set(number)
            .aria_size_of_set(count)
            .flex()
            .items_center()
            .gap_2()
            .mx_neg_3()
            .px_3()
            .rounded(theme.radius)
            .when(selected, |row| row.bg(theme.secondary_active))
            .when(!selected, |row| {
                row.hover(|style| style.bg(theme.secondary_hover))
            })
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                let hovered = *hovered;
                this.graph
                    .update(cx, |graph, cx| graph.light(index, hovered, cx));
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.graph
                    .update(cx, |graph, cx| graph.select_index(index, cx));
            }));
        match wide {
            true => row
                .h(theme.metrics.list_row)
                .child(enabled)
                .child(kind.w(cover * KIND))
                .child(div().flex_1())
                .child(frequency.flex_none().w(cover * FREQUENCY))
                .child(gain.flex_none().w(cover * GAIN))
                .child(width.flex_none().w(cover * WIDTH))
                .child(remove),
            // two lines of one width: the switch, the kind and the remove button over the three
            // values, which share what the window leaves them
            false => row.py_2().child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(enabled)
                            .child(kind.flex_1().min_w_0())
                            .child(remove),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(frequency.flex_1().min_w_0())
                            .child(gain.flex_1().min_w_0())
                            .child(width.flex_1().min_w_0()),
                    ),
            ),
        }
        .into_any_element()
    }

    /// The button that appends a flat filter, which does nothing once the curve is full.
    fn add_button(&self, full: bool, cx: &mut Context<Self>) -> Button {
        Button::new("equalizer-add")
            .small()
            .outline()
            .icon("icons/plus.svg")
            .label(t!("equalizer-add-filter"))
            .disabled(full)
            .on_click(cx.listener(|this, _, _, cx| {
                let (kind, frequency, gain, q) = NEW_FILTER;
                let filter = Filter::new(kind, frequency, gain, Width::Q(q), true)
                    .expect("the new filter is in range");
                this.graph
                    .update(cx, |graph, cx| graph.add_filter(filter, cx))
            }))
    }

    /// The filters' group: a row for each filter under a heading that carries the add button,
    /// or an empty state while there are none. `wide` puts each filter on one line.
    pub(super) fn filters(&self, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let filters = self.equalizer.read(cx).curve().filters().to_vec();
        let full = filters.len() >= MAX_FILTERS;
        let selected = self.graph.read(cx).selected_index();
        let count = filters.len();
        let rows: Vec<_> = filters
            .into_iter()
            .enumerate()
            .map(|(index, filter)| {
                self.filter_row(index, filter, selected == Some(index), count, wide, cx)
            })
            .collect();
        let add = self.add_button(full, cx).into_any_element();
        let empty = t!("equalizer-no-filters");

        div()
            .flex()
            .flex_col()
            .child(heading("equalizer-filters", FILTERS_ICON, Some(add), cx))
            .map(|group| match rows.is_empty() {
                true => group.child(
                    vacant(empty.clone(), cx)
                        .id("equalizer-no-filters")
                        .role(Role::Label)
                        .aria_value(empty)
                        .rounded(theme.radius)
                        .border_1()
                        .border_dashed()
                        .border_color(theme.border),
                ),
                false => group.child(
                    div()
                        .id("equalizer-filter-list")
                        .role(Role::List)
                        .aria_label(t!("equalizer-filters"))
                        .flex()
                        .flex_col()
                        .when(!wide, |list| list.gap_1())
                        .children(rows),
                ),
            })
            .into_any_element()
    }
}

/// What `field` shows for `curve`: the value rounded to what a person types, or nothing for
/// a row past the last filter. A field that still reads this was not edited.
fn shown(curve: &Curve, field: Field) -> String {
    let (index, value) = match field {
        Field::Preamp => {
            return match curve.preamp() {
                Preamp::Manual(db) => figure(db, 2),
                Preamp::Auto => String::new(),
            };
        }
        Field::Filter(index, value) => (index, value),
    };
    let Some(filter) = curve.filters().get(index) else {
        return String::new();
    };
    match value {
        Value::Frequency => figure(filter.frequency(), 2),
        Value::Gain => figure(filter.gain(), 2),
        Value::Width => figure(filter.width().value(), 3),
    }
}

/// `curve` with `field` set to `number`, or why the model refuses it.
fn edit(curve: &Curve, field: Field, number: f64) -> Result<Curve, Invalid> {
    let (index, value) = match field {
        Field::Preamp => return Curve::new(Preamp::Manual(number), curve.filters().to_vec()),
        Field::Filter(index, value) => (index, value),
    };
    let mut curve = curve.clone();
    let Some(filter) = curve.filters_mut().get_mut(index) else {
        return Ok(curve);
    };
    let (mut frequency, mut gain, mut width) = (filter.frequency(), filter.gain(), filter.width());
    match value {
        Value::Frequency => frequency = number,
        Value::Gain => gain = number,
        Value::Width => {
            width = match width {
                Width::Q(_) => Width::Q(number),
                Width::Octaves(_) => Width::Octaves(number),
            }
        }
    }
    *filter = Filter::new(filter.kind(), frequency, gain, width, filter.enabled())?;
    Ok(curve)
}

/// `value` with at most `decimals` places and no trailing zeros.
pub(super) fn figure(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    let text = match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.'),
        false => &text,
    };
    match text {
        "-0" => "0".to_owned(),
        text => text.to_owned(),
    }
}

pub(super) fn invalid_text(invalid: Invalid) -> SharedString {
    let range = |range: std::ops::RangeInclusive<f64>| (*range.start(), *range.end());
    match invalid {
        Invalid::Frequency(_) => {
            let (low, high) = range(FREQUENCY_RANGE);
            t!("equalizer-invalid-frequency", low = low, high = high)
        }
        Invalid::Gain(_) => {
            let (low, high) = range(GAIN_RANGE);
            t!("equalizer-invalid-gain", low = low, high = high)
        }
        Invalid::Q(_) => {
            let (low, high) = range(Q_RANGE);
            t!("equalizer-invalid-q", low = low, high = high)
        }
        Invalid::Octaves(_) => {
            let (low, high) = range(OCTAVES_RANGE);
            t!("equalizer-invalid-octaves", low = low, high = high)
        }
        Invalid::ShelfOctaves => t!("equalizer-invalid-shelf-octaves"),
        Invalid::Preamp(_) => {
            let (low, high) = range(PREAMP_RANGE);
            t!("equalizer-invalid-preamp", low = low, high = high)
        }
        Invalid::TooMany(_) => t!("equalizer-filter-limit", max = MAX_FILTERS),
    }
}

fn kind_name(kind: Kind) -> SharedString {
    let key = match kind {
        Kind::Peak => "equalizer-kind-peak",
        Kind::LowShelf => "equalizer-kind-low-shelf",
        Kind::HighShelf => "equalizer-kind-high-shelf",
        Kind::LowPass => "equalizer-kind-low-pass",
        Kind::HighPass => "equalizer-kind-high-pass",
        Kind::Notch => "equalizer-kind-notch",
        Kind::BandPass => "equalizer-kind-band-pass",
        Kind::AllPass => "equalizer-kind-all-pass",
    };
    i18n::lookup(key, None)
}

/// `figure` muted against the end of a box as wide and tall as a number field, standing where a
/// field would when there is nothing to type, so nothing beside it moves. Screen readers read
/// `spoken` in its place, since a figure such as a dash says nothing aloud.
pub(super) fn figure_cell(
    id: impl Into<ElementId>,
    figure: SharedString,
    spoken: SharedString,
    width: Pixels,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .role(Role::Label)
        .aria_value(spoken)
        .flex()
        .flex_none()
        .items_center()
        .justify_end()
        .w(width)
        .h(theme.metrics.control_small)
        .px_3()
        .text_size(theme.text(Text::Label))
        .text_color(theme.muted_foreground)
        .child(figure)
        .into_any_element()
}

/// The box around one of a filter's number fields, named for screen readers after the filter and
/// the value, since the field itself only names its unit.
fn field_box(id: (&'static str, usize), name: SharedString) -> Stateful<Div> {
    div().id(id).role(Role::Group).aria_label(name)
}
