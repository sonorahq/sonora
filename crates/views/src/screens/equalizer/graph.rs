use gpui::prelude::*;
use gpui::{AnyElement, Context};
use i18n::t;
use music::equalizer::{
    Curve, FREQUENCY_RANGE, Filter, GAIN_RANGE, Invalid, Kind, OCTAVES_RANGE, Q_RANGE, Response,
    Width,
};
use ui::{
    ActiveTheme as _, GraphEdit, ResponseAxes, ResponseGraph, ResponseGraphState, ResponseNode,
    ResponsePoint, Room,
};

use super::EqualizerView;
use super::rows::{NEW_FILTER, figure};

/// The graph's frequency axis in hertz, the band people hear.
const GRAPH_FREQUENCIES: (f64, f64) = (20., 20_000.);
/// How many points the response is sampled at across the band, each enabled filter's centre
/// coming on top, so a narrow notch reaches the floor.
const POINTS: usize = 2048;
/// The rate the response is drawn at while nothing plays.
const IDLE_RATE: u32 = 48_000;
/// How much one wheel step over a node narrows or widens it, as a power of two.
const WIDTH_STEP: f64 = 1. / 12.;
/// How many significant figures a dragged frequency keeps, so the field reads 1230 rather than
/// 1234.5678.
const FREQUENCY_FIGURES: i32 = 3;
/// The levels at the bottom and the top of the plot that the spectrum is drawn between, in dBFS.
const SPECTRUM_SCALE: (f32, f32) = (-90., 0.);
/// How tall the graph is in cover sizes, with the room of a wide window, and with less.
const GRAPH_TALL: f32 = 2.8;
const GRAPH_SHORT: f32 = 2.;

/// What the graph keeps between renders: an id per filter of the playing curve, the selected
/// filter, and the response as last sampled. Filters that came from anywhere but this screen
/// get new ids, so a selection or a drag never carries into another profile's filters.
pub(super) struct Graph {
    state: ResponseGraphState,
    /// One id per filter of `filters`, in its order.
    ids: Vec<u64>,
    next_id: u64,
    selected: Option<u64>,
    /// The filters the ids were last brought in step with.
    filters: Vec<Filter>,
    /// The filters this screen put in place last, until the playing curve shows them.
    own: Option<Vec<Filter>>,
    plotted: Option<Plotted>,
}

/// The response the graph draws, and what it was sampled for.
struct Plotted {
    curve: Curve,
    rate: u32,
    response: Response,
    points: Vec<ResponsePoint>,
}

impl Graph {
    pub(super) fn new() -> Self {
        Self {
            state: ResponseGraphState::new("equalizer-graph"),
            ids: Vec::new(),
            next_id: 0,
            selected: None,
            filters: Vec::new(),
            own: None,
            plotted: None,
        }
    }

    /// Brings the ids in step with `curve`, which plays now. Filters this screen did not put in
    /// place get new ids and lose the selection.
    pub(super) fn follow(&mut self, curve: &Curve) {
        let own = self.own.take();
        if curve.filters() == self.filters {
            return;
        }
        self.filters = curve.filters().to_vec();
        if own.as_deref() == Some(curve.filters()) && self.ids.len() == self.filters.len() {
            return;
        }
        let first = self.next_id;
        self.next_id += self.filters.len() as u64;
        self.ids = (first..self.next_id).collect();
        self.selected = None;
    }

    /// Notes `curve` as this screen's own edit, to be applied next.
    pub(super) fn edited(&mut self, curve: &Curve) {
        self.own = Some(curve.filters().to_vec());
    }

    pub(super) fn id(&self, index: usize) -> Option<u64> {
        self.ids.get(index).copied()
    }

    pub(super) fn selected(&self) -> Option<u64> {
        self.selected
    }

    /// Selects filter `id`. Returns whether that changed anything.
    pub(super) fn select(&mut self, id: u64) -> bool {
        let changed = self.selected != Some(id);
        self.selected = Some(id);
        changed
    }

    fn index(&self, id: u64) -> Option<usize> {
        self.ids.iter().position(|known| *known == id)
    }

    /// `curve` with `filter` after the others, given a new id and selected, unless it is full.
    fn add(&mut self, curve: &Curve, filter: Filter) -> Option<Curve> {
        let mut filters = curve.filters().to_vec();
        filters.push(filter);
        let curve = Curve::new(curve.preamp(), filters).ok()?;
        let id = self.next_id;
        self.next_id += 1;
        self.ids.push(id);
        self.selected = Some(id);
        self.edited(&curve);
        Some(curve)
    }

    /// `curve` without filter `index`, its id going with it, and the selection when it was that
    /// filter.
    fn remove(&mut self, curve: &Curve, index: usize) -> Option<Curve> {
        if index >= curve.filters().len() || index >= self.ids.len() {
            return None;
        }
        let mut filters = curve.filters().to_vec();
        filters.remove(index);
        let curve = Curve::new(curve.preamp(), filters).ok()?;
        let id = self.ids.remove(index);
        if self.selected == Some(id) {
            self.selected = None;
        }
        self.edited(&curve);
        Some(curve)
    }

    /// `curve` with filter `id` made into what `change` makes of it, when the model accepts it.
    fn change(
        &mut self,
        curve: &Curve,
        id: u64,
        change: impl FnOnce(Filter) -> Result<Filter, Invalid>,
    ) -> Option<Curve> {
        let index = self.index(id)?;
        let mut curve = curve.clone();
        let filter = curve.filters_mut().get_mut(index)?;
        *filter = change(*filter).ok()?;
        self.edited(&curve);
        Some(curve)
    }

    /// The response of `curve` at `rate`, sampled again only when either changed.
    fn plotted(&mut self, curve: &Curve, rate: u32) -> &Plotted {
        let stale = self
            .plotted
            .as_ref()
            .is_none_or(|plotted| plotted.rate != rate || &plotted.curve != curve);
        if stale {
            let response = curve.response(rate);
            let points = sample(curve, &response, rate);
            self.plotted = Some(Plotted {
                curve: curve.clone(),
                rate,
                response,
                points,
            });
        }
        self.plotted.as_ref().expect("sampled above")
    }
}

impl EqualizerView {
    /// The live response of the filters, a node per filter on it, and the preamp as a level
    /// across it. A node sits on its own filter's curve at its frequency.
    pub(super) fn graph(&mut self, room: Room, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let playback = self.playback.read(cx);
        let on = playback.equalizer();
        let curve = playback.equalizer_curve().clone();
        let rate = playback.equalizer_rate().unwrap_or(IDLE_RATE);
        let preamp = playback.equalizer_preamp_db();
        let spectrum: Vec<_> = self
            .signal
            .read(cx)
            .levels()
            .unwrap_or_default()
            .iter()
            .map(|level| ResponsePoint {
                frequency_hz: level.frequency,
                gain_db: level.dbfs,
            })
            .collect();
        let (ids, selected) = (self.graph.ids.clone(), self.graph.selected);
        let plotted = self.graph.plotted(&curve, rate);
        let nodes: Vec<_> = curve
            .filters()
            .iter()
            .zip(ids)
            .map(|(filter, id)| {
                let gain = match filter.kind().has_gain() {
                    true => filter.gain(),
                    false => plotted.response.db(filter.frequency()),
                };
                ResponseNode {
                    id,
                    position: ResponsePoint {
                        frequency_hz: filter.frequency() as f32,
                        gain_db: plotted_db(gain * lift(filter.kind())),
                    },
                    readout_db: plotted_db(gain),
                    gain_adjustable: filter.kind().has_gain(),
                    selected: selected == Some(id),
                    enabled: on && filter.enabled(),
                }
            })
            .collect();
        let points = plotted.points.clone();
        let (low, high) = GRAPH_FREQUENCIES;
        let axes = ResponseAxes::new(low as f32..=high as f32, gain_floor()..=-gain_floor())
            .expect("the graph's axes are valid");
        let height = match room.fits(Room::Wide) {
            true => GRAPH_TALL,
            false => GRAPH_SHORT,
        };

        let (bottom, top) = SPECTRUM_SCALE;
        ResponseGraph::new(&self.graph.state, axes)
            .h(theme.metrics.cover * height)
            .when(!spectrum.is_empty(), |graph| {
                graph.spectrum(spectrum, bottom..=top)
            })
            .response(points)
            .nodes(nodes)
            .reference(
                preamp as f32,
                t!("equalizer-graph-preamp", db = figure(preamp, 1)),
            )
            .bypassed(!on)
            .on_edit(cx.listener(|this, edit: &GraphEdit, _, cx| this.graph_edit(edit, cx)))
            .into_any_element()
    }

    /// Appends `filter` to the playing curve and selects it, unless the curve is full.
    pub(super) fn add_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        let curve = self.playback.read(cx).equalizer_curve().clone();
        if let Some(curve) = self.graph.add(&curve, filter) {
            self.apply(curve, cx);
        }
    }

    pub(super) fn remove_filter(&mut self, index: usize, cx: &mut Context<Self>) {
        let curve = self.playback.read(cx).equalizer_curve().clone();
        if let Some(curve) = self.graph.remove(&curve, index) {
            self.apply(curve, cx);
        }
    }

    /// Carries out what was done on the graph. A drag or a wheel step glides the filter it
    /// moves; adding and removing cross-fade like the list's own buttons.
    fn graph_edit(&mut self, edit: &GraphEdit, cx: &mut Context<Self>) {
        let curve = self.playback.read(cx).equalizer_curve().clone();
        let glided = match *edit {
            GraphEdit::Select(id) => {
                if self.graph.select(id) {
                    cx.notify();
                }
                None
            }
            GraphEdit::Move { id, position } => self.graph.change(&curve, id, |filter| {
                // An axis the drag did not move along comes back as given, and keeps the
                // filter's own value rather than a rounded one.
                let frequency = match position.frequency_hz == filter.frequency() as f32 {
                    true => filter.frequency(),
                    false => placed_frequency(position.frequency_hz),
                };
                let lift = lift(filter.kind());
                let moved = position.gain_db != plotted_db(filter.gain() * lift);
                let gain = match filter.kind().has_gain() && moved {
                    true => placed_gain(position.gain_db / lift as f32),
                    false => filter.gain(),
                };
                Filter::new(
                    filter.kind(),
                    frequency,
                    gain,
                    filter.width(),
                    filter.enabled(),
                )
            }),
            GraphEdit::Width { id, steps } => {
                let factor = 2f64.powf(f64::from(steps) * WIDTH_STEP);
                self.graph.change(&curve, id, |filter| {
                    let width = match filter.width() {
                        Width::Q(q) => {
                            Width::Q((q / factor).clamp(*Q_RANGE.start(), *Q_RANGE.end()))
                        }
                        Width::Octaves(octaves) => Width::Octaves(
                            (octaves * factor).clamp(*OCTAVES_RANGE.start(), *OCTAVES_RANGE.end()),
                        ),
                    };
                    Filter::new(
                        filter.kind(),
                        filter.frequency(),
                        filter.gain(),
                        width,
                        filter.enabled(),
                    )
                })
            }
            GraphEdit::Add(position) => {
                let (kind, _, _, q) = NEW_FILTER;
                let frequency = placed_frequency(position.frequency_hz);
                let gain = placed_gain(position.gain_db);
                if let Ok(filter) = Filter::new(kind, frequency, gain, Width::Q(q), true) {
                    self.add_filter(filter, cx);
                }
                None
            }
            GraphEdit::Remove(id) => {
                if let Some(index) = self.graph.index(id) {
                    self.remove_filter(index, cx);
                }
                None
            }
        };
        if let Some(curve) = glided {
            self.playback
                .update(cx, |playback, cx| playback.glide_equalizer_curve(curve, cx));
        }
    }
}

/// The response sampled on a log grid up to the axis's high end or the rate's Nyquist frequency,
/// whichever is lower, plus each enabled filter's centre in that band.
fn sample(curve: &Curve, response: &Response, rate: u32) -> Vec<ResponsePoint> {
    let (low, high) = GRAPH_FREQUENCIES;
    let high = high.min(f64::from(rate) / 2.);
    let ratio = high / low;
    let grid = (0..POINTS).map(|point| low * ratio.powf(point as f64 / (POINTS - 1) as f64));
    let centres = curve
        .filters()
        .iter()
        .filter(|filter| filter.enabled())
        .map(|filter| filter.frequency())
        .filter(|frequency| (low..=high).contains(frequency));
    let mut frequencies: Vec<f64> = grid.chain(centres).collect();
    frequencies.sort_by(f64::total_cmp);
    frequencies.dedup();
    frequencies
        .into_iter()
        .map(|frequency| ResponsePoint {
            frequency_hz: frequency as f32,
            gain_db: plotted_db(response.db(frequency)),
        })
        .collect()
}

/// A dragged frequency to a few significant figures, in range.
fn placed_frequency(frequency_hz: f32) -> f64 {
    let frequency = f64::from(frequency_hz);
    let scale = 10f64.powi(FREQUENCY_FIGURES - 1 - frequency.log10().floor() as i32);
    ((frequency * scale).round() / scale).clamp(*FREQUENCY_RANGE.start(), *FREQUENCY_RANGE.end())
}

/// A dragged gain to a tenth of a decibel, in range.
fn placed_gain(gain_db: f32) -> f64 {
    ((f64::from(gain_db) * 10.).round() / 10.).clamp(*GAIN_RANGE.start(), *GAIN_RANGE.end())
}

/// The bottom of the graph's gain axis, which is also as low as the drawn curve goes, so a
/// notch's centre at minus infinity draws at the floor.
fn gain_floor() -> f32 {
    *GAIN_RANGE.start() as f32
}

/// The share of its gain a filter's curve reaches at its frequency, where its node sits: half
/// for a shelf, whose centre is halfway up its step.
fn lift(kind: Kind) -> f64 {
    match kind {
        Kind::LowShelf | Kind::HighShelf => 0.5,
        _ => 1.,
    }
}

fn plotted_db(db: f64) -> f32 {
    (db as f32).max(gain_floor())
}
