use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Div, DragMoveEvent, ElementId, Empty, Hsla, MouseButton, MouseDownEvent, Pixels,
    Render, SharedString, Stateful, StyleRefinement, Window, canvas, div, px, relative,
};

use i18n::t;

use crate::Sort;
use crate::theme::ActiveTheme as _;
use crate::time::clock;

const TRACK: f32 = 0.5;
const THUMB: f32 = 1.5;
const HIT: f32 = 2.;
/// How far one screen reader step moves a handle of a range without stops, as a share of its
/// length.
const STEP: f32 = 0.05;

type ChangeFn = Box<dyn Fn(&(f32, f32), &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Clock,
    Plain,
}

impl Unit {
    pub fn say(self, value: f32) -> SharedString {
        match self {
            Unit::Clock => clock(std::time::Duration::from_secs_f32(value.max(0.))),
            Unit::Plain => SharedString::from(format!("{}", value.round() as i64)),
        }
    }
}

#[derive(Clone)]
pub enum Filter {
    Range(RangeAxis),
    Flag(FlagAxis),
}

impl Filter {
    pub fn narrowed(&self) -> bool {
        match self {
            Self::Range(axis) => !axis.whole(),
            Self::Flag(axis) => axis.on,
        }
    }
}

pub enum FilterChange {
    Range(&'static str, (f32, f32)),
    Flag(&'static str, bool),
    Reset,
}

#[derive(Clone)]
pub struct RangeAxis {
    pub key: &'static str,
    pub label: SharedString,
    pub bounds: (f32, f32),
    pub value: (f32, f32),
    pub unit: Unit,
    pub values: Option<Vec<f32>>,
}

impl RangeAxis {
    pub fn span(&self) -> f32 {
        (self.bounds.1 - self.bounds.0).max(f32::EPSILON)
    }

    fn steps(&self) -> Option<&[f32]> {
        match self.values.as_deref() {
            Some(values) if values.len() > 1 => Some(values),
            _ => None,
        }
    }

    fn seat(&self, value: f32) -> f32 {
        let Some(steps) = self.steps() else {
            return ((value - self.bounds.0) / self.span()).clamp(0., 1.);
        };
        let last = (steps.len() - 1) as f32;
        steps
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - value).abs().total_cmp(&(*b - value).abs()))
            .map(|(index, _)| index as f32 / last)
            .unwrap_or(0.)
    }

    pub fn share(&self) -> (f32, f32) {
        (self.seat(self.value.0), self.seat(self.value.1))
    }

    pub fn at(&self, share: (f32, f32)) -> (f32, f32) {
        let Some(steps) = self.steps() else {
            return (
                self.bounds.0 + share.0 * self.span(),
                self.bounds.0 + share.1 * self.span(),
            );
        };
        let last = steps.len() - 1;
        let pick = |share: f32| {
            let index = (share.clamp(0., 1.) * last as f32).round() as usize;
            steps[index.min(last)]
        };
        (pick(share.0), pick(share.1))
    }

    pub fn stops(&self) -> Vec<f32> {
        let Some(steps) = self.steps() else {
            return Vec::new();
        };
        let last = (steps.len() - 1) as f32;
        (0..steps.len()).map(|index| index as f32 / last).collect()
    }

    pub fn clamped(mut self) -> Self {
        self.value = (
            self.value.0.clamp(self.bounds.0, self.bounds.1),
            self.value.1.clamp(self.bounds.0, self.bounds.1),
        );
        self
    }

    pub fn whole(&self) -> bool {
        let (low, high) = self.share();
        low <= f32::EPSILON && high >= 1. - f32::EPSILON
    }
}

#[derive(Clone)]
pub struct FlagAxis {
    pub key: &'static str,
    pub label: SharedString,
    pub on: bool,
}

#[derive(Clone)]
pub struct SortAxis {
    pub key: &'static str,
    pub label: SharedString,
    pub order: Option<Sort>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Handle {
    Low,
    High,
}

#[derive(Clone)]
struct Seize(SharedString);

impl Render for Seize {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone)]
pub struct RangeState {
    id: SharedString,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    active: Rc<Cell<Handle>>,
}

impl RangeState {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            bounds: Rc::new(Cell::new(Bounds::default())),
            active: Rc::new(Cell::new(Handle::Low)),
        }
    }

    fn share_at(&self, x: Pixels, pad: Pixels) -> f32 {
        let bounds = self.bounds.get();
        let pin = px((pad / px(1.) * THUMB).round());
        let travel = bounds.size.width - pin;
        if travel <= px(0.) {
            return 0.;
        }
        ((x - bounds.origin.x - pin / 2.) / travel).clamp(0., 1.)
    }
}

#[derive(IntoElement)]
pub struct RangeScrubber {
    base: Stateful<Div>,
    id: SharedString,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    active: Rc<Cell<Handle>>,
    value: (f32, f32),
    stops: Rc<[f32]>,
    filled: Hsla,
    empty: Hsla,
    thumb: Hsla,
    /// What a screen reader calls the pair of handles, read as focus enters them.
    name: Option<SharedString>,
    /// How a screen reader reads where the low and the high handle stand, in place of bare
    /// fractions.
    values: Option<(SharedString, SharedString)>,
    on_change: Option<ChangeFn>,
}

impl RangeScrubber {
    pub fn new(state: &RangeState, value: (f32, f32)) -> Self {
        Self {
            base: div().id(ElementId::Name(state.id.clone())),
            id: state.id.clone(),
            bounds: state.bounds.clone(),
            active: state.active.clone(),
            value: (value.0.clamp(0., 1.), value.1.clamp(0., 1.)),
            stops: Rc::from(Vec::new()),
            filled: gpui::white(),
            empty: gpui::black(),
            thumb: gpui::white(),
            name: None,
            values: None,
            on_change: None,
        }
    }

    pub fn stops(mut self, stops: Vec<f32>) -> Self {
        self.stops = Rc::from(stops);
        self
    }

    pub fn colors(mut self, filled: Hsla, empty: Hsla, thumb: Hsla) -> Self {
        self.filled = filled;
        self.empty = empty;
        self.thumb = thumb;
        self
    }

    /// Names the pair of handles for screen readers after what they narrow, such as the year.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Where the low and the high handle stand as a screen reader should read them, such as
    /// two years, in place of the bare fractions.
    pub fn values(mut self, low: impl Into<SharedString>, high: impl Into<SharedString>) -> Self {
        self.values = Some((low.into(), high.into()));
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(&(f32, f32), &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Box::new(handler));
        self
    }
}

impl Styled for RangeScrubber {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for RangeScrubber {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let pad = cx.theme().metrics.pad;
        let line = px((pad / px(1.) * TRACK).round());
        let pin = px((pad / px(1.) * THUMB).round());
        let reach = px((pad / px(1.) * HIT).round());

        let Self {
            mut base,
            id,
            bounds,
            active,
            value,
            stops,
            filled,
            empty,
            thumb,
            name,
            values,
            on_change,
        } = self;
        let overrides = std::mem::take(base.style());

        let state = Rc::new(RangeState {
            id: id.clone(),
            bounds: bounds.clone(),
            active: active.clone(),
        });
        let on_change = on_change.map(Rc::new);

        let seize = {
            let state = state.clone();
            let on_change = on_change.clone();
            let stops = stops.clone();
            move |event: &MouseDownEvent, window: &mut Window, cx: &mut App| {
                let share = snap(state.share_at(event.position.x, pad), &stops);
                let near = match (share - value.0).abs() <= (share - value.1).abs() {
                    true => Handle::Low,
                    false => Handle::High,
                };
                state.active.set(near);
                if let Some(handler) = on_change.as_ref() {
                    handler(&clamped_against_other(value, near, share), window, cx);
                }
            }
        };

        let dragged = {
            let state = state.clone();
            let on_change = on_change.clone();
            let mine = id.clone();
            let stops = stops.clone();
            move |event: &DragMoveEvent<Seize>, window: &mut Window, cx: &mut App| {
                if event.drag(cx).0 != mine {
                    return;
                }
                let share = snap(state.share_at(event.event.position.x, pad), &stops);
                if let Some(handler) = on_change.as_ref() {
                    handler(
                        &clamped_against_other(value, state.active.get(), share),
                        window,
                        cx,
                    );
                }
            }
        };

        let (low_value, high_value) = match values {
            Some((low, high)) => (Some(low), Some(high)),
            None => (None, None),
        };
        // Each handle is a slider of its own, which a screen reader steps from stop to stop.
        let knob = |which: Handle, spoken: Option<SharedString>| {
            let (share, side, name) = match which {
                Handle::Low => (value.0, "low", t!("range-from")),
                Handle::High => (value.1, "high", t!("range-to")),
            };
            handle(share, line, pin, thumb)
                .id((ElementId::Name(id.clone()), side))
                .role(gpui::accesskit::Role::Slider)
                .aria_label(name)
                .aria_numeric_value(f64::from(share))
                .aria_min_numeric_value(0.)
                .aria_max_numeric_value(1.)
                .when_some(spoken, |this, spoken| this.aria_value(spoken))
                .when_some(on_change.clone(), |this, on_change| {
                    let (rise, fall) = (on_change.clone(), on_change);
                    let (above, below) = (stops.clone(), stops.clone());
                    this.on_a11y_action(gpui::accesskit::Action::Increment, move |_, window, cx| {
                        let share = stepped(share, &above, true);
                        rise(&clamped_against_other(value, which, share), window, cx)
                    })
                    .on_a11y_action(
                        gpui::accesskit::Action::Decrement,
                        move |_, window, cx| {
                            let share = stepped(share, &below, false);
                            fall(&clamped_against_other(value, which, share), window, cx)
                        },
                    )
                })
        };

        let mut scrubber = base
            .role(gpui::accesskit::Role::Group)
            .when_some(name, |this, name| this.aria_label(name))
            .flex()
            .items_center()
            .w_full()
            .h(reach)
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, seize)
            .on_drag(Seize(id.clone()), |seize, _, _, cx| {
                cx.new(|_| seize.clone())
            })
            .on_drag_move(dragged)
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(line)
                    .rounded_full()
                    .bg(empty)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(pin / 2.)
                            .right(pin / 2.)
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .bottom_0()
                                    .left(relative(value.0))
                                    .right(relative(1. - value.1.max(value.0)))
                                    .bg(filled),
                            )
                            .child(knob(Handle::Low, low_value))
                            .child(knob(Handle::High, high_value)),
                    )
                    .child(
                        canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                    ),
            );
        scrubber.style().refine(&overrides);
        scrubber
    }
}

/// A range handle centred on `share` of the rail it sits in.
fn handle(share: f32, line: Pixels, pin: Pixels, thumb: Hsla) -> Div {
    div()
        .absolute()
        .top((line - pin) / 2.)
        .left(relative(share))
        .ml(Pixels::ZERO - pin / 2.)
        .size(pin)
        .rounded_full()
        .bg(thumb)
}

fn snap(share: f32, stops: &[f32]) -> f32 {
    stops
        .iter()
        .copied()
        .min_by(|a, b| (a - share).abs().total_cmp(&(b - share).abs()))
        .unwrap_or(share)
}

/// Where a handle at `share` lands after one screen reader step up or down: the next stop that
/// way when the range has stops, or `STEP` further along it when it has none.
fn stepped(share: f32, stops: &[f32], up: bool) -> f32 {
    if stops.is_empty() {
        let moved = match up {
            true => share + STEP,
            false => share - STEP,
        };
        return moved.clamp(0., 1.);
    }
    let next = match up {
        true => stops
            .iter()
            .copied()
            .find(|stop| *stop > share + f32::EPSILON),
        false => stops
            .iter()
            .rev()
            .copied()
            .find(|stop| *stop < share - f32::EPSILON),
    };
    next.unwrap_or(share)
}

fn clamped_against_other(value: (f32, f32), handle: Handle, share: f32) -> (f32, f32) {
    match handle {
        Handle::Low => (share.min(value.1), value.1),
        Handle::High => (value.0, share.max(value.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::{RangeAxis, Unit, snap};

    fn axis(bounds: (f32, f32), value: (f32, f32)) -> RangeAxis {
        RangeAxis {
            key: "test",
            label: "Test".into(),
            bounds,
            value,
            unit: Unit::Plain,
            values: None,
        }
    }

    fn sparse() -> RangeAxis {
        RangeAxis {
            values: Some(vec![1967., 1977., 1989., 2020.]),
            ..axis((1967., 2020.), (1967., 2020.))
        }
    }

    #[test]
    fn stops_are_evenly_spaced_whatever_the_gaps() {
        let stops = sparse().stops();

        assert_eq!(stops.len(), 4);
        for (index, stop) in stops.iter().enumerate() {
            assert!((stop - index as f32 / 3.).abs() < 1e-6, "stop {index}");
        }
    }

    #[test]
    fn every_present_value_owns_an_equal_slice() {
        let axis = sparse();
        let stops = axis.stops();

        for (index, value) in [1967., 1977., 1989., 2020.].iter().enumerate() {
            let seat = snap(index as f32 / 3., &stops);
            assert_eq!(axis.at((seat, 1.)).0.round(), *value);
        }
    }

    #[test]
    fn a_position_between_stops_lands_on_the_nearer_value() {
        let axis = sparse();
        let stops = axis.stops();

        assert_eq!(axis.at((snap(0.6, &stops), 1.)).0.round(), 1989.);
        assert_eq!(axis.at((snap(0.2, &stops), 1.)).0.round(), 1977.);
    }

    #[test]
    fn a_selection_round_trips_through_its_seat() {
        let axis = RangeAxis {
            values: Some(vec![1967., 1977., 1989., 2020.]),
            ..axis((1967., 2020.), (1977., 1989.))
        };

        assert_eq!(axis.at(axis.share()), (1977., 1989.));
    }

    #[test]
    fn snapping_is_a_no_op_without_values() {
        let stops = axis((0., 100.), (0., 100.)).stops();

        assert!(stops.is_empty());
        assert!((snap(0.37, &stops) - 0.37).abs() < 1e-6);
    }

    #[test]
    fn shares_round_trip_through_values() {
        let axis = axis((60., 300.), (120., 240.));
        let (low, high) = axis.at(axis.share());

        assert!((low - 120.).abs() < 0.01);
        assert!((high - 240.).abs() < 0.01);
    }

    #[test]
    fn a_full_span_reads_as_untouched() {
        assert!(axis((0., 100.), (0., 100.)).whole());
        assert!(!axis((0., 100.), (10., 100.)).whole());
        assert!(!axis((0., 100.), (0., 90.)).whole());
    }

    #[test]
    fn a_collapsed_axis_never_divides_by_zero() {
        let axis = axis((5., 5.), (5., 5.));

        assert!(axis.share().0.is_finite());
        assert!(axis.share().1.is_finite());
    }

    #[test]
    fn clamping_pulls_a_stale_selection_into_view() {
        let axis = axis((2010., 2020.), (1990., 2000.)).clamped();

        assert_eq!(axis.value, (2010., 2010.));
    }

    #[test]
    fn clamping_leaves_a_selection_inside_the_bounds_alone() {
        let axis = axis((1967., 2026.), (1990., 2000.)).clamped();

        assert_eq!(axis.value, (1990., 2000.));
    }

    #[test]
    fn clock_and_plain_format_differently() {
        assert_eq!(Unit::Clock.say(125.), "2:05");
        assert_eq!(Unit::Plain.say(2005.), "2005");
    }
}
