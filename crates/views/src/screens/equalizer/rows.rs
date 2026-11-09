//! The filter list of advanced mode: a row per filter with its switch, kind, frequency, gain and
//! width, and the number fields behind them, which commit on Enter or when they lose focus.

use i18n::{SharedString, t};
use music::equalizer::{
    Curve, FREQUENCY_RANGE, Filter, GAIN_RANGE, Invalid, Kind, MAX_FILTERS, OCTAVES_RANGE,
    PREAMP_RANGE, Preamp, Q_RANGE, Width, apo,
};
use rune::{Cursor, El, Key, KeyEvent, TextEdit, Tx, col, el, row, text};
use state::{Outcome, Toast, Toasts};
use ui::{Button, Input, MenuItem, PRESS_SCALE_CARD, Picker, Switch, Text, Theme, Weight, font};

use super::{FILTERS_ICON, State, apply, heading};
use crate::shared::page::rems;
use crate::{Ui, act};

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
#[derive(Default)]
pub(super) struct Fields {
    frequency: TextEdit,
    gain: TextEdit,
    width: TextEdit,
}

/// A number field, which is what a commit reads, validates and applies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Field {
    Preamp,
    Filter(usize, Value),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Value {
    Frequency,
    Gain,
    Width,
}

impl Fields {
    pub(super) fn rows() -> Vec<Self> {
        (0..MAX_FILTERS).map(|_| Self::default()).collect()
    }
}

/// Every number field of the page, the preamp's first.
fn fields() -> impl Iterator<Item = Field> {
    let rows = (0..MAX_FILTERS).flat_map(|index| {
        [Value::Frequency, Value::Gain, Value::Width].map(|value| Field::Filter(index, value))
    });
    std::iter::once(Field::Preamp).chain(rows)
}

/// The text `field` holds in the page's state.
pub(super) fn edit_of(state: &mut State, field: Field) -> &mut TextEdit {
    match field {
        Field::Preamp => &mut state.preamp,
        Field::Filter(index, Value::Frequency) => &mut state.rows[index].frequency,
        Field::Filter(index, Value::Gain) => &mut state.rows[index].gain,
        Field::Filter(index, Value::Width) => &mut state.rows[index].width,
    }
}

fn edit_ref(state: &State, field: Field) -> &TextEdit {
    match field {
        Field::Preamp => &state.preamp,
        Field::Filter(index, Value::Frequency) => &state.rows[index].frequency,
        Field::Filter(index, Value::Gain) => &state.rows[index].gain,
        Field::Filter(index, Value::Width) => &state.rows[index].width,
    }
}

/// Fills every field whose text differs from the playing curve once it changed, the field being
/// typed in included, since what it held belongs to the curve that was replaced. Returns whether
/// the curve changed since the fields were last filled.
pub(super) fn show_curve(ui: &mut Ui) -> bool {
    let curve = ui.store.equalizer.curve();
    let state = &mut ui.screens.equalizer;
    if state.shown.as_ref() == Some(curve) {
        return false;
    }
    let curve = curve.clone();
    for field in fields() {
        let text = shown(&curve, field);
        let edit = edit_of(state, field);
        if edit.text != text {
            edit.replace_text(text);
        }
    }
    state.shown = Some(curve);
    true
}

/// Commits the field that lost focus since the last frame, which is what leaving a field does.
/// Returns whether one did.
pub(super) fn blurred(ui: &mut Ui) -> bool {
    let state = &ui.screens.equalizer;
    let now = fields().find(|field| edit_ref(state, *field).focused);
    let was = state.focused;
    if was == now {
        return false;
    }
    ui.screens.equalizer.focused = now;
    match was {
        Some(field) => {
            commit(ui, field);
            ui.settle();
            true
        }
        None => false,
    }
}

/// Applies what `field` holds when it differs from the curve, through the model's checks. A value
/// the model refuses is put back and the reason shown.
pub(super) fn commit(ui: &mut Ui, field: Field) {
    let text = edit_ref(&ui.screens.equalizer, field)
        .text
        .trim()
        .to_owned();
    let curve = ui.store.equalizer.curve().clone();
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
            // A comma is read the way an import reads it, so `12,500` is 12.5 kHz, while a dot
            // is a decimal point, so `15.625` stays 15.625 Hz.
            (Field::Filter(_, Value::Frequency), true) => {
                apo::hertz_read(&dotted).unwrap_or(number)
            }
            _ => number,
        })
        .map(|number| edit(&curve, field, number));
    match edited {
        Some(Ok(curve)) => apply(ui, curve),
        refused => {
            if let Some(Err(invalid)) = refused {
                Toasts::post(
                    Toast::new(Outcome::Warning, "equalizer-value-refused")
                        .detail(invalid_text(invalid)),
                    &mut ui.store.cx,
                );
            }
            edit_of(&mut ui.screens.equalizer, field).replace_text(current);
        }
    }
}

/// Puts `field` back as the curve has it and leaves it, which is what Escape does.
fn revert(ui: &mut Ui, field: Field, tx: &mut Tx) {
    let text = shown(ui.store.equalizer.curve(), field);
    edit_of(&mut ui.screens.equalizer, field).replace_text(text);
    // the field is left as the curve has it, so the blur that follows commits nothing
    ui.screens.equalizer.focused = None;
    tx.blur();
}

/// A number field: its number against its end and its unit inside, committing on Enter and
/// putting itself back on Escape.
pub(super) fn number(ui: &Ui, key: String, field: Field, unit: &str) -> El {
    let theme = &ui.theme;
    let unit = i18n::lookup(unit, None).into_owned();
    Input::new(
        key,
        edit_ref(&ui.screens.equalizer, field),
        unit.clone(),
        move |ui: &mut Ui| edit_of(&mut ui.screens.equalizer, field),
    )
    .compact()
    .align_end()
    .suffix(unit)
    .on_submit(move |ui: &mut Ui, _: &mut Tx| {
        commit(ui, field);
        ui.settle();
    })
    .on_key(
        move |ui: &mut Ui, event: &KeyEvent, tx: &mut Tx| match event.key {
            Key::Escape => {
                revert(ui, field, tx);
                true
            }
            _ => false,
        },
    )
    .build(theme)
}

/// The playing curve with filter `index` changed by `change`, or unchanged when the filter is
/// gone.
fn change(ui: &mut Ui, index: usize, change: impl FnOnce(Filter) -> Filter) {
    let mut curve = ui.store.equalizer.curve().clone();
    let Some(filter) = curve.filters_mut().get_mut(index) else {
        return;
    };
    *filter = change(*filter);
    apply(ui, curve);
}

/// One filter on one line: its switch, its kind, its frequency, gain and width, and a button
/// that removes it. The row lights the filter's node on the graph while the pointer is over it,
/// a press on it selects that node, and it stands marked while its node is selected. Where the
/// line has too little room, the three values move to a line of their own. `count` is how many
/// filters the list holds, which screen readers announce with the row.
fn filter_row(
    ui: &Ui,
    layer: &mut Vec<El>,
    index: usize,
    filter: Filter,
    selected: bool,
    count: usize,
    wide: bool,
) -> El {
    let theme = &ui.theme;
    let state = &ui.screens.equalizer;
    let cover = theme.metrics.cover;
    let number = index + 1;
    let on = filter.enabled();
    let enabled = Switch::new(format!("equalizer-filter-on-{index}"), on)
        .name(t!("equalizer-filter", number = number))
        .on_press(act(move |ui: &mut Ui| {
            change(ui, index, |filter| filter.with_enabled(!on))
        }))
        .build(theme);
    let kinds = Kind::ALL.map(|kind| {
        MenuItem::new(kind.id(), kind_name(kind))
            .selected(kind == filter.kind())
            .on_press(act(move |ui: &mut Ui| {
                change(ui, index, |filter| filter.with_kind(kind))
            }))
    });
    let kind = Picker::new(
        format!("equalizer-kind-{index}"),
        &state.popovers,
        |ui: &mut Ui| &mut ui.screens.equalizer.popovers,
        kind_name(filter.kind()),
    )
    .name(t!("equalizer-filter-kind", number = number))
    .width(Picker::<Ui>::NARROW)
    .left()
    .items(kinds)
    .build(theme, layer);
    let gain = match filter.kind().has_gain() {
        true => self::number(
            ui,
            format!("equalizer-gain-{index}"),
            Field::Filter(index, Value::Gain),
            "equalizer-decibels",
        ),
        false => figure_cell(
            theme,
            format!("equalizer-no-gain-{index}"),
            t!("equalizer-no-gain").into_owned(),
            t!("equalizer-no-gain-spoken").into_owned(),
            cover * GAIN,
        ),
    };
    let width_unit = match filter.width() {
        Width::Octaves(_) => "equalizer-octaves",
        Width::Q(_) => "equalizer-q",
    };
    let remove = Button::new(format!("equalizer-remove-{index}"))
        .small()
        .ghost()
        .icon("icons/x.svg")
        .tooltip(t!("equalizer-remove-filter"))
        .on_press(act(move |ui: &mut Ui| {
            super::graph::remove_filter(ui, index)
        }))
        .build(theme);
    let frequency = field_box(
        format!("equalizer-frequency-field-{index}"),
        t!("equalizer-filter-frequency", number = number).into_owned(),
        self::number(
            ui,
            format!("equalizer-frequency-{index}"),
            Field::Filter(index, Value::Frequency),
            "equalizer-hertz",
        ),
    );
    let gain = field_box(
        format!("equalizer-gain-field-{index}"),
        t!("equalizer-filter-gain", number = number).into_owned(),
        gain,
    );
    let width = field_box(
        format!("equalizer-width-field-{index}"),
        t!("equalizer-filter-width", number = number).into_owned(),
        self::number(
            ui,
            format!("equalizer-width-{index}"),
            Field::Filter(index, Value::Width),
            width_unit,
        ),
    );

    let id = state.graph.id(index);
    let reach = rems(theme, 0.75);
    let mut line = row()
        .key(format!("equalizer-filter-{}", id.unwrap_or(index as u64)))
        .role("listitem")
        .label(t!("equalizer-filter", number = number))
        .position_in_set(number, count)
        .center_y()
        .gap(rems(theme, 0.5))
        .mx(-reach)
        .px(reach)
        .radius(theme.radius)
        .cursor(Cursor::Pointer)
        .on_hover_change(move |ui: &mut Ui, hovered: bool, _: &mut Tx| {
            super::graph::light(ui, index, hovered)
        })
        .on_press(move |ui: &mut Ui| super::graph::select_index(ui, index))
        .transition(rune::Transition::opacity());
    if selected {
        line = line
            .fill(theme.secondary_active.color())
            .description(t!("equalizer-filter-selected"));
    } else {
        line = line
            .hover(theme.secondary_hover.color())
            .press_scale(PRESS_SCALE_CARD);
    }
    match wide {
        true => line
            .h(theme.metrics.list_row)
            .child(enabled)
            .child(kind.w(cover * KIND))
            .child(el().grow(1.))
            .child(frequency.w(cover * FREQUENCY))
            .child(gain.w(cover * GAIN))
            .child(width.w(cover * WIDTH))
            .child(remove),
        // two lines of one width: the switch, the kind and the remove button over the three
        // values, which share what the window leaves them
        false => line.py(rems(theme, 0.5)).child(
            col()
                .key("lines")
                .grow(1.)
                .min_w(0.)
                .gap(rems(theme, 0.5))
                .child(
                    row()
                        .key("top")
                        .center_y()
                        .gap(rems(theme, 0.5))
                        .child(enabled)
                        .child(kind.grow(1.).shrink().min_w(0.))
                        .child(remove),
                )
                .child(
                    row()
                        .key("values")
                        .center_y()
                        .gap(rems(theme, 0.5))
                        .child(frequency.grow(1.).w(0.).min_w(0.))
                        .child(gain.grow(1.).w(0.).min_w(0.))
                        .child(width.grow(1.).w(0.).min_w(0.)),
                ),
        ),
    }
}

/// The button that appends a flat filter, which does nothing once the curve is full.
fn add_button(theme: &Theme, full: bool) -> El {
    Button::new("equalizer-add")
        .small()
        .outline()
        .icon("icons/plus.svg")
        .label(t!("equalizer-add-filter"))
        .disabled(full)
        .on_press(act(|ui: &mut Ui| {
            let (kind, frequency, gain, q) = NEW_FILTER;
            let filter = Filter::new(kind, frequency, gain, Width::Q(q), true)
                .expect("the new filter is in range");
            super::graph::add_filter(ui, filter);
        }))
        .build(theme)
}

/// The filters' group: a row for each filter under a heading that carries the add button, or an
/// empty state while there are none. `wide` puts each filter on one line.
pub(super) fn filters(ui: &Ui, layer: &mut Vec<El>, wide: bool) -> El {
    let theme = &ui.theme;
    let filters = ui.store.equalizer.curve().filters().to_vec();
    let full = filters.len() >= MAX_FILTERS;
    let selected = ui.screens.equalizer.graph.selected_index();
    let count = filters.len();
    let rows: Vec<El> = filters
        .into_iter()
        .enumerate()
        .map(|(index, filter)| {
            filter_row(
                ui,
                layer,
                index,
                filter,
                selected == Some(index),
                count,
                wide,
            )
        })
        .collect();
    let group = col().key("equalizer-filters-group").w_full().child(heading(
        theme,
        "equalizer-filters",
        FILTERS_ICON,
        Some(add_button(theme, full)),
    ));
    match rows.is_empty() {
        true => {
            let empty = t!("equalizer-no-filters").into_owned();
            group.child(
                ui::spoken_vacant("equalizer-no-filters", empty, theme)
                    .radius(theme.radius)
                    .border(1., theme.border.color())
                    .border_dash(DASH[0], DASH[1])
                    .transition(rune::Transition::opacity()),
            )
        }
        false => {
            let mut list = col()
                .key("equalizer-filter-list")
                .role("list")
                .label(t!("equalizer-filters"))
                .w_full()
                .children(rows);
            if !wide {
                list = list.gap(rems(theme, 0.25));
            }
            group.child(list)
        }
    }
}

/// The dash and gap of the empty filter list's border.
const DASH: [f32; 2] = [4., 3.];

/// What `field` shows for `curve`: the value rounded to what a person types, or nothing for a
/// row past the last filter. A field that still reads this was not edited.
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

/// Why the model turned a value down, in the interface's language.
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

fn kind_name(kind: Kind) -> String {
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
    i18n::lookup(key, None).into_owned()
}

/// `figure` muted against the end of a box as wide and tall as a number field, standing where a
/// field would when there is nothing to type, so nothing beside it moves. Screen readers read
/// `spoken` in its place, since a figure such as a dash says nothing aloud.
pub(super) fn figure_cell(
    theme: &Theme,
    key: String,
    figure: String,
    spoken: String,
    width: f32,
) -> El {
    row()
        .key(key)
        .role("text")
        .label(spoken)
        .center_y()
        .justify_end()
        .w(width)
        .h(theme.metrics.control_small)
        .px(rems(theme, 0.75))
        .child(
            text(figure)
                .font(font(theme, Text::Label, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .a11y_hidden(),
        )
}

/// The box around one of a filter's number fields, named for screen readers after the filter and
/// the value, since the field itself only names its unit.
fn field_box(key: String, name: String, field: El) -> El {
    row()
        .key(key)
        .role("group")
        .label(name)
        .min_w(0.)
        .child(field.grow(1.))
}
