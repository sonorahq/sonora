//! The filter and sort model a list's filter popover draws, and the two-handled range scrubber it
//! narrows a range with.

use std::rc::Rc;

use i18n::{SharedString, t};
use rune::{Cursor, Drag, El, Phase, Tx, el, row};

use crate::color::Hsla;
use crate::motion::Springs;
use crate::table::Sort;
use crate::theme::Theme;
use crate::time::clock;

/// The rail's thickness, the handles' size and the height that takes the pointer, as shares of
/// the theme's padding.
const TRACK: f32 = 0.5;
const THUMB: f32 = 1.5;
const HIT: f32 = 2.;
/// How far one screen reader or arrow-key step moves a handle of a range without stops, as a
/// share of its length.
const STEP: f32 = 0.05;
/// Which handle a drag holds, kept in the drag's scratch value.
const LOW: f32 = 0.;
const HIGH: f32 = 1.;

type ChangeFn<S> = Rc<dyn Fn(&mut S, (f32, f32), &mut Tx)>;

/// How a range's values read.
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

/// One axis a list can be narrowed along.
#[derive(Clone)]
pub enum Filter {
    Range(RangeAxis),
    Flag(FlagAxis),
}

impl Filter {
    /// Whether the axis narrows the list at all.
    pub fn narrowed(&self) -> bool {
        match self {
            Self::Range(axis) => !axis.whole(),
            Self::Flag(axis) => axis.on,
        }
    }
}

/// A change the filter popover asks the list for.
pub enum FilterChange {
    Range(&'static str, (f32, f32)),
    Flag(&'static str, bool),
    Reset,
}

/// A numeric axis, such as year or length, narrowed to a span of its bounds. With `values` the
/// handles step between the values present rather than sliding freely.
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

    /// Where `value` sits along the scrubber, as a share of it.
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

    /// The selection as shares of the scrubber.
    pub fn share(&self) -> (f32, f32) {
        (self.seat(self.value.0), self.seat(self.value.1))
    }

    /// The values two shares of the scrubber stand for.
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

    /// The shares the handles snap to: evenly spaced, one per value present, whatever the gaps
    /// between the values.
    pub fn stops(&self) -> Vec<f32> {
        let Some(steps) = self.steps() else {
            return Vec::new();
        };
        let last = (steps.len() - 1) as f32;
        (0..steps.len()).map(|index| index as f32 / last).collect()
    }

    /// The selection pulled inside the bounds.
    pub fn clamped(mut self) -> Self {
        self.value = (
            self.value.0.clamp(self.bounds.0, self.bounds.1),
            self.value.1.clamp(self.bounds.0, self.bounds.1),
        );
        self
    }

    /// Whether the selection spans the whole axis, narrowing nothing.
    pub fn whole(&self) -> bool {
        let (low, high) = self.share();
        low <= f32::EPSILON && high >= 1. - f32::EPSILON
    }
}

/// An on-or-off axis, such as liked songs only.
#[derive(Clone)]
pub struct FlagAxis {
    pub key: &'static str,
    pub label: SharedString,
    pub on: bool,
}

/// A column a list can be sorted by, with the order it is sorted in when it is the sort.
#[derive(Clone)]
pub struct SortAxis {
    pub key: &'static str,
    pub label: SharedString,
    pub order: Option<Sort>,
}

/// A two-handled scrubber over `0..=1` that narrows a range. Dragging anywhere on it takes the
/// nearer handle; each handle is a slider of its own for the keyboard and screen readers.
pub struct RangeScrubber<S: 'static> {
    key: String,
    value: (f32, f32),
    stops: Rc<[f32]>,
    colors: Option<(Hsla, Hsla, Hsla)>,
    name: Option<String>,
    values: Option<(String, String)>,
    on_change: Option<ChangeFn<S>>,
}

impl<S: 'static> RangeScrubber<S> {
    /// `value` is the low and high handle as shares of the scrubber.
    pub fn new(key: impl Into<String>, value: (f32, f32)) -> Self {
        let low = value.0.clamp(0., 1.);
        Self {
            key: key.into(),
            value: (low, value.1.clamp(0., 1.).max(low)),
            stops: Rc::from(Vec::new()),
            colors: None,
            name: None,
            values: None,
            on_change: None,
        }
    }

    /// The shares the handles snap to, from [`RangeAxis::stops`].
    pub fn stops(mut self, stops: Vec<f32>) -> Self {
        self.stops = Rc::from(stops);
        self
    }

    /// The selected span, the rail outside it and the handles. Defaults to the primary, the
    /// secondary and the foreground.
    pub fn colors(mut self, filled: Hsla, empty: Hsla, thumb: Hsla) -> Self {
        self.colors = Some((filled, empty, thumb));
        self
    }

    /// Names the pair of handles for screen readers after what they narrow, such as the year.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Where the low and the high handle stand as a screen reader should read them, such as two
    /// years, in place of the bare fractions.
    pub fn values(mut self, low: impl Into<String>, high: impl Into<String>) -> Self {
        self.values = Some((low.into(), high.into()));
        self
    }

    /// The new low and high shares, whenever a handle moves.
    pub fn on_change(mut self, f: impl Fn(&mut S, (f32, f32), &mut Tx) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let Self {
            key,
            value,
            stops,
            colors,
            name,
            values,
            on_change,
        } = self;
        let pad = theme.metrics.pad;
        let line = (pad * TRACK).round();
        let pin = (pad * THUMB).round();
        let reach = (pad * HIT).round();
        let (filled, empty, thumb) =
            colors.unwrap_or((theme.primary, theme.secondary, theme.foreground));
        let (low, high) = value;
        let (low_value, high_value) = match values {
            Some((low, high)) => (Some(low), Some(high)),
            None => (None, None),
        };

        let knob = |which: f32, spoken: Option<String>| {
            let (share, side, label) = match which == LOW {
                true => (low, "low", t!("range-from")),
                false => (high, "high", t!("range-to")),
            };
            let mut knob = el()
                .key(format!("{key}-{side}"))
                .role("slider")
                .label(label.to_string())
                .focusable(true)
                .range_value(f64::from(share), 0., 1.)
                .size(pin, pin)
                .radius(pin / 2.)
                .fill(thumb.color())
                .motion(Springs::QUICK);
            if let Some(spoken) = spoken {
                knob = knob.a11y_value(spoken);
            }
            if let Some(on_change) = on_change.clone() {
                let stops = stops.clone();
                knob = knob.on_step(move |s: &mut S, delta: i32, tx: &mut Tx| {
                    let share = stepped(share, &stops, delta > 0);
                    on_change(s, toward((low, high), which, share), tx);
                });
            }
            knob
        };

        // The rail lays the handles out by flex shares, so a value set from outside slides them
        // on the layout spring. Between the handles it is filled.
        let rail = row()
            .w_full()
            .h(pin)
            .center_y()
            .child(segment(low, line, empty))
            .child(knob(LOW, low_value))
            .child(segment(high - low, line, filled))
            .child(knob(HIGH, high_value))
            .child(segment(1. - high, line, empty));

        let mut scrubber = row()
            .key(key.clone())
            .role("group")
            .w_full()
            .h(reach)
            .center_y()
            .cursor(Cursor::Pointer)
            .child(rail);
        if let Some(name) = name {
            scrubber = scrubber.label(name);
        }
        let Some(on_change) = on_change else {
            return scrubber;
        };
        scrubber.on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
            let travel = (drag.w - pin * 2.).max(f32::EPSILON);
            let low_at = |x: f32| ((x - pin / 2.) / travel).clamp(0., 1.);
            let high_at = |x: f32| ((x - pin * 1.5) / travel).clamp(0., 1.);
            if drag.phase == Phase::Down {
                // Handles that meet are told apart by the side the press lands on.
                let near_low = match low == high {
                    true => low_at(drag.x) < low,
                    false => (low_at(drag.x) - low).abs() <= (high_at(drag.x) - high).abs(),
                };
                drag.data = match near_low {
                    true => LOW,
                    false => HIGH,
                };
            }
            if drag.cancelled() {
                on_change(s, (low, high), tx);
                return;
            }
            let share = match drag.data == LOW {
                true => snap(low_at(drag.x), &stops),
                false => snap(high_at(drag.x), &stops),
            };
            on_change(s, toward((low, high), drag.data, share), tx);
        })
    }
}

/// A stretch of the rail `share` of its free length long, `line` thick.
fn segment(share: f32, line: f32, color: Hsla) -> El {
    el().grow(share.max(0.))
        .w(0.)
        .h(line)
        .fill(color.color())
        .motion(Springs::QUICK)
}

fn snap(share: f32, stops: &[f32]) -> f32 {
    stops
        .iter()
        .copied()
        .min_by(|a, b| (a - share).abs().total_cmp(&(b - share).abs()))
        .unwrap_or(share)
}

/// Where a handle at `share` lands after one step up or down: the next stop that way when the
/// range has stops, or `STEP` further along it when it has none.
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

/// The selection with `handle` moved to `share`, never past the other handle.
fn toward(value: (f32, f32), handle: f32, share: f32) -> (f32, f32) {
    match handle == LOW {
        true => (share.min(value.1), value.1),
        false => (value.0, share.max(value.0)),
    }
}
