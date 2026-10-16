use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Focusable as _, MouseDownEvent, SharedString, Window, div,
};
use i18n::t;
use music::equalizer::{
    Curve, FREQUENCY_RANGE, Filter, GAIN_RANGE, Invalid, Kind, MAX_FILTERS, OCTAVES_RANGE,
    PREAMP_RANGE, Preamp, Q_RANGE, Width, apo,
};
use state::{Outcome, Toast, Toasts};
use ui::{ActiveTheme as _, Button, Input, MenuItem, Picker, Switch, Text, faint, vacant};

use super::{EqualizerView, FIELD};

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
/// How wide the kind picker's column is, in cover sizes, so the fields line up in every row.
const KIND: f32 = 0.75;
/// The new filter Add puts in: flat, so adding one changes nothing until it is shaped.
pub(super) const NEW_FILTER: (Kind, f64, f64, f64) = (Kind::Peak, 1_000., 0., 1.);

/// The number fields of one filter row.
pub(super) struct Row {
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

impl Row {
    pub(super) fn new(cx: &mut Context<EqualizerView>) -> Self {
        let mut number = || cx.new(|cx| Input::new("", cx).compact());
        Self {
            frequency: number(),
            gain: number(),
            width: number(),
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
        let curve = self.playback.read(cx).equalizer_curve();
        if self.shown.as_ref() == Some(curve) {
            return;
        }
        let curve = curve.clone();
        self.graph.follow(&curve);
        let texts: Vec<_> = self
            .fields()
            .map(|(field, input)| (input.clone(), shown(&curve, field)))
            .collect();
        for (input, text) in texts {
            if input.read(cx).text() != text {
                input.update(cx, |input, cx| input.set_text(text, cx));
            }
        }
        self.shown = Some(curve);
    }

    /// Applies what `field` holds when it differs from the curve, through the model's checks.
    /// A value the model refuses is put back and the reason shown.
    pub(super) fn commit(&mut self, field: Field, cx: &mut Context<Self>) {
        let input = self.input(field).clone();
        let text = input.read(cx).text().trim().to_owned();
        let curve = self.playback.read(cx).equalizer_curve().clone();
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
        let curve = self.playback.read(cx).equalizer_curve().clone();
        let texts: Vec<_> = self
            .fields()
            .filter(|(_, input)| input.read(cx).focus_handle(cx).is_focused(window))
            .map(|(field, input)| (input.clone(), shown(&curve, field)))
            .collect();
        for (input, text) in texts {
            input.update(cx, |input, cx| input.set_text(text, cx));
        }
        window.blur();
    }

    /// The playing curve with filter `index` changed by `change`, or unchanged when the filter
    /// is gone.
    fn change(
        &mut self,
        index: usize,
        change: impl FnOnce(Filter) -> Filter,
        cx: &mut Context<Self>,
    ) {
        let mut curve = self.playback.read(cx).equalizer_curve().clone();
        let Some(filter) = curve.filters_mut().get_mut(index) else {
            return;
        };
        *filter = change(*filter);
        self.apply(curve, cx);
    }

    /// A number field and its unit after it.
    pub(super) fn number(&self, input: &Entity<Input>, unit: SharedString, cx: &App) -> AnyElement {
        let theme = cx.theme();
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().w(theme.metrics.cover * FIELD).child(input.clone()))
            .child(
                div()
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground)
                    .child(unit),
            )
            .into_any_element()
    }

    fn filter_row(
        &self,
        index: usize,
        filter: Filter,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = self.graph.id(index);
        let selected = id.is_some() && id == self.graph.selected();
        let theme = *cx.theme();
        let row = &self.rows[index];
        let enabled = Switch::new(("equalizer-filter-on", index), filter.enabled()).on_click(
            cx.listener(move |this, _, _, cx| {
                this.change(index, |filter| filter.with_enabled(!filter.enabled()), cx)
            }),
        );
        let kind = Picker::new(KINDS[index], &self.popovers, kind_name(filter.kind()))
            .width(Picker::NARROW)
            .items(Kind::ALL.map(|kind| {
                MenuItem::new(kind.id(), kind_name(kind))
                    .selected(kind == filter.kind())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change(index, |filter| filter.with_kind(kind), cx)
                    }))
            }));
        let gain = match filter.kind().has_gain() {
            true => self.number(&row.gain, t!("equalizer-decibels"), cx),
            false => div()
                .w(theme.metrics.cover * FIELD)
                .text_color(theme.muted_foreground)
                .child(t!("equalizer-no-gain"))
                .into_any_element(),
        };
        let width_unit = match filter.width() {
            Width::Q(_) => t!("equalizer-q"),
            Width::Octaves(_) => t!("equalizer-octaves"),
        };
        let remove = Button::new(("equalizer-remove", index))
            .small()
            .ghost()
            .icon("icons/x.svg")
            .tooltip("equalizer-remove-filter")
            .on_click(cx.listener(move |this, _, _, cx| this.remove_filter(index, cx)));
        let values = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_3()
            .child(self.number(&row.frequency, t!("equalizer-hertz"), cx))
            .child(gain)
            .child(self.number(&row.width, width_unit, cx));
        let controls = div()
            .flex()
            .items_center()
            .gap_3()
            .child(enabled)
            .child(div().w(theme.metrics.cover * KIND).child(kind));

        match wide {
            true => div()
                .flex()
                .items_center()
                .gap_3()
                .child(controls)
                .child(values)
                .child(div().flex_1())
                .child(remove),
            false => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(controls.child(div().flex_1()).child(remove))
                .child(values),
        }
        .py_1()
        .px_2()
        .rounded(theme.radius)
        .when(selected, |row| row.bg(theme.table_active))
        // In the capture phase, since the fields and the kind picker keep their mouse-down.
        .when_some(id, |row, id| {
            row.capture_any_mouse_down(cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                if this.graph.select(id) {
                    cx.notify();
                }
            }))
        })
        .into_any_element()
    }

    pub(super) fn filters(&self, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let filters = self.playback.read(cx).equalizer_curve().filters().to_vec();
        let full = filters.len() >= MAX_FILTERS;
        let empty = filters.is_empty();
        let rows: Vec<_> = filters
            .into_iter()
            .enumerate()
            .map(|(index, filter)| self.filter_row(index, filter, wide, cx))
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(faint(cx).child(t!("equalizer-filters")))
            .children(rows)
            .when(empty, |this| {
                this.child(vacant(t!("equalizer-no-filters"), cx))
            })
            .child(
                div().flex().pt_2().child(
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
                            this.add_filter(filter, cx)
                        })),
                ),
            )
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
