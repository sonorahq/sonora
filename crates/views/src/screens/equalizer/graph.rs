//! The response graph: the curve heard with a node per filter the listener drags, its ids and
//! selection, the edits made on it, and the live spectrum drawn behind it.

use std::cell::Cell;
use std::time::{Duration, Instant};

use i18n::t;
use music::Level;
use music::equalizer::{
    Curve, FREQUENCY_RANGE, Filter, GAIN_RANGE, Invalid, Kind, OCTAVES_RANGE, Q_RANGE, Response,
    Width,
};
use rune::{El, Pixels, Tx, el};
use state::{Outputs, Store};
use ui::{
    GraphEdit, Hsla, ResponseAxes, ResponseGraph, ResponseGraphState, ResponseNode, ResponsePoint,
    Rgba, Room, Theme, motion::ease_out_cubic,
};

use super::rows::{NEW_FILTER, figure};
use crate::Ui;

/// The graph's frequency axis in hertz, the band people hear.
const GRAPH_FREQUENCIES: (f64, f64) = (20., 20_000.);
/// How many points the response is sampled at across the band, each enabled filter's centre
/// coming on top, so a narrow notch reaches the floor.
const POINTS: usize = 384;
/// The rate the response is drawn at while no stream is open.
const IDLE_RATE: u32 = 48_000;
/// How much one wheel step over a node narrows or widens it, as a power of two.
const WIDTH_STEP: f64 = 1. / 12.;
/// How many significant figures a dragged frequency keeps, so the field reads 1230 rather than
/// 1234.5678.
const FREQUENCY_FIGURES: i32 = 3;
/// The levels at the bottom and the top of the plot that the spectrum is drawn between, in dBFS.
const SPECTRUM_SCALE: (f32, f32) = (-90., 0.);
/// How long the spectrum takes to rise in once it arrives, as a fade-in of the sound would.
const SPECTRUM_RISE: Duration = Duration::from_millis(700);
/// About how far apart the spectrum's reads arrive. The levels glide from one read to the next
/// over this long, by the clock, so a read that comes late does not jolt them.
const SPECTRUM_GLIDE: Duration = Duration::from_millis(16);
/// How tall the graph is in cover sizes, with the room of a wide window, and with less.
const GRAPH_TALL: f32 = 2.8;
const GRAPH_SHORT: f32 = 2.;
/// The gain axis's half spans in decibels, narrowest first. The axis shows the narrowest one the
/// curve, the nodes and the preamp fit in with `HEADROOM` to spare, so a gentle preset fills the
/// plot and a dragged node can always go a little further.
const GAIN_SPANS: [f32; 5] = [6., 9., 12., 18., 24.];
/// How much room the axis keeps above the furthest reach, in decibels.
const HEADROOM: f32 = 1.5;
/// The plot's inset from the graph's edges, in paddings: left, top, right, bottom. The same as
/// `ui::ResponseGraph` draws its plot at, so the spectrum behind lines up with its grid.
const INSET: [f32; 4] = [7., 3., 3., 4.];
/// How strongly the spectrum behind the plot fills and outlines, as `ui::ResponseGraph` would.
const SPECTRUM_FILL: f32 = 0.07;
const SPECTRUM_LINE: f32 = 0.2;
/// The spectrum's outline, in points.
const HAIRLINE: f32 = 1.;
/// The most device pixels the spectrum's raster holds along either side.
const MAX_RASTER: f32 = 4096.;

/// What the graph keeps between frames: an id per filter of the playing curve, the selected
/// filter, and the response as last sampled. Filters that came from anywhere but this screen
/// get new ids, so a selection or a drag never carries into another profile's filters.
pub(super) struct Graph {
    pub(super) state: ResponseGraphState,
    /// One id per filter of `filters`, in its order.
    ids: Vec<u64>,
    next_id: u64,
    selected: Option<u64>,
    /// The filters the ids were last brought in step with.
    filters: Vec<Filter>,
    /// The filters this screen put in place last, until the playing curve shows them.
    own: Option<Vec<Filter>>,
    plotted: Option<Plotted>,
    /// The half span of the gain axis last shown. It holds still while a node is held, so the
    /// scale never shifts under a drag.
    span: Cell<f32>,
    /// The node the filter list lit while the pointer is over its row.
    lit: Option<u64>,
}

/// The response the graph draws, and what it was sampled for.
struct Plotted {
    curve: Curve,
    rate: u32,
    response: Response,
    points: Vec<ResponsePoint>,
    /// How far from 0 dB the curve reaches, leaving out the dips of filters with no gain, which
    /// fall without limit towards a notch's centre or past a pass filter's corner.
    reach: f32,
}

impl Default for Graph {
    fn default() -> Self {
        Self {
            state: ResponseGraphState::new(),
            ids: Vec::new(),
            next_id: 0,
            selected: None,
            filters: Vec::new(),
            own: None,
            plotted: None,
            span: Cell::new(GAIN_SPANS[0]),
            lit: None,
        }
    }
}

impl Graph {
    /// Brings the ids in step with `curve`, which plays now. Filters this screen did not put in
    /// place get new ids and lose the selection.
    fn follow(&mut self, curve: &Curve) {
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
    fn edited(&mut self, curve: &Curve) {
        self.own = Some(curve.filters().to_vec());
    }

    /// The id of the filter at `index` of the playing curve.
    pub(super) fn id(&self, index: usize) -> Option<u64> {
        self.ids.get(index).copied()
    }

    fn index(&self, id: u64) -> Option<usize> {
        self.ids.iter().position(|known| *known == id)
    }

    /// Where the selected filter stands in the playing curve, if one is selected.
    pub(super) fn selected_index(&self) -> Option<usize> {
        self.selected.and_then(|id| self.index(id))
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

    /// Samples the response of `curve` at `rate` again when either changed. Returns whether it
    /// did.
    fn plot(&mut self, curve: &Curve, rate: u32) -> bool {
        let stale = self
            .plotted
            .as_ref()
            .is_none_or(|plotted| plotted.rate != rate || &plotted.curve != curve);
        if stale {
            let response = curve.response(rate);
            let points = sample(curve, &response, rate);
            let reach = reach(curve, rate, &points);
            self.plotted = Some(Plotted {
                curve: curve.clone(),
                rate,
                response,
                points,
                reach,
            });
        }
        stale
    }
}

/// Brings the graph in step with the playing curve and the rate the output plays at, before a
/// frame. Returns whether anything it draws changed.
pub(super) fn follow(ui: &mut Ui) -> bool {
    let rate = rate(&ui.store.outputs);
    let equalizer = &ui.store.equalizer;
    let graph = &mut ui.screens.equalizer.graph;
    let before = graph.ids.clone();
    graph.follow(equalizer.curve());
    let plotted = graph.plot(equalizer.played(), rate);
    plotted || graph.ids != before
}

/// Selects filter `id`, or none.
fn select(ui: &mut Ui, id: Option<u64>) {
    ui.screens.equalizer.graph.selected = id;
}

/// Selects the filter at `index` of the playing curve.
pub(super) fn select_index(ui: &mut Ui, index: usize) {
    let id = ui.screens.equalizer.graph.id(index);
    select(ui, id);
}

/// Lights the node of the filter at `index` while the pointer is over its row, and puts it out
/// when the pointer leaves, unless another row lit a node since.
pub(super) fn light(ui: &mut Ui, index: usize, hovered: bool) {
    let graph = &mut ui.screens.equalizer.graph;
    let Some(id) = graph.id(index) else {
        return;
    };
    let lit = match hovered {
        true => Some(id),
        false if graph.lit == Some(id) => None,
        false => return,
    };
    graph.lit = lit;
    graph.state.light(lit);
}

/// Plays `curve`, an edit made on this screen.
pub(super) fn apply(ui: &mut Ui, curve: Curve) {
    ui.screens.equalizer.graph.edited(&curve);
    let Store {
        equalizer,
        settings,
        cx,
        ..
    } = &mut ui.store;
    equalizer.set_curve(curve, settings, cx);
}

/// Appends `filter` to the playing curve and selects it, unless the curve is full.
pub(super) fn add_filter(ui: &mut Ui, filter: Filter) {
    let curve = ui.store.equalizer.curve().clone();
    if let Some(curve) = ui.screens.equalizer.graph.add(&curve, filter) {
        apply(ui, curve);
    }
}

pub(super) fn remove_filter(ui: &mut Ui, index: usize) {
    let curve = ui.store.equalizer.curve().clone();
    if let Some(curve) = ui.screens.equalizer.graph.remove(&curve, index) {
        apply(ui, curve);
    }
}

/// Carries out what was done on the graph. A drag or a wheel step glides the filter it moves,
/// and adding and removing cross-fade like the list's own buttons. A dragged or added gain stays
/// within the axis shown, a shelf's whole step included. A node stands raised by the tone, so the
/// tone at its frequency comes off the gain it is dragged or added to.
fn graph_edit(ui: &mut Ui, edit: GraphEdit) {
    let curve = ui.store.equalizer.curve().clone();
    let tone = ui.store.equalizer.tone().response(rate(&ui.store.outputs));
    let graph = &mut ui.screens.equalizer.graph;
    let span = graph.span.get();
    let glided = match edit {
        GraphEdit::Select(id) => {
            select(ui, Some(id));
            None
        }
        GraphEdit::Deselect => {
            select(ui, None);
            None
        }
        GraphEdit::Move { id, position } => graph.change(&curve, id, |filter| {
            // An axis the drag did not move along comes back as given, and keeps the filter's
            // own value rather than a rounded one.
            let frequency = match position.frequency_hz == filter.frequency() as f32 {
                true => filter.frequency(),
                false => placed_frequency(position.frequency_hz),
            };
            let lift = lift(filter.kind());
            let shown = filter.gain() * lift + tone.db(filter.frequency());
            let moved = position.gain_db != plotted_db(shown);
            let gain = match filter.kind().has_gain() && moved {
                true => {
                    let own = f64::from(position.gain_db) - tone.db(frequency);
                    placed_gain((own / lift) as f32, span)
                }
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
            graph.change(&curve, id, |filter| {
                let width = match filter.width() {
                    Width::Q(q) => Width::Q((q / factor).clamp(*Q_RANGE.start(), *Q_RANGE.end())),
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
            let own = f64::from(position.gain_db) - tone.db(frequency);
            let gain = placed_gain(own as f32, span);
            if let Ok(filter) = Filter::new(kind, frequency, gain, Width::Q(q), true) {
                add_filter(ui, filter);
            }
            None
        }
        GraphEdit::Remove(id) => {
            if let Some(index) = graph.index(id) {
                remove_filter(ui, index);
            }
            None
        }
    };
    if let Some(curve) = glided {
        let store = &mut ui.store;
        store.equalizer.glide_curve(curve, &mut store.cx);
    }
}

/// The live response heard, the tone's bass and treble included, a node per filter of the
/// sound, and the preamp as a level across it, over the spectrum of what plays. A node sits on
/// its own filter's curve at its frequency, raised or lowered by the tone there. The graph is
/// `width` wide and stands as tall as `room` lets it.
pub(super) fn view(ui: &Ui, width: f32, room: Room, scale: f32, still: bool) -> El {
    let theme = &ui.theme;
    let equalizer = &ui.store.equalizer;
    let graph = &ui.screens.equalizer.graph;
    let on = equalizer.enabled();
    let advanced = ui.store.settings.equalizer_advanced();
    let curve = equalizer.curve();
    let rate = rate(&ui.store.outputs);
    let tone = equalizer.tone().response(rate);
    let preamp = equalizer.preamp_db();
    let height = height(room, theme);
    let (points, reach, response) = match &graph.plotted {
        Some(plotted) => (
            plotted.points.clone(),
            plotted.reach,
            Some(&plotted.response),
        ),
        None => (Vec::new(), 0., None),
    };
    let nodes: Vec<ResponseNode> = curve
        .filters()
        .iter()
        .zip(&graph.ids)
        .map(|(filter, id)| {
            let (gain, position) = match filter.kind().has_gain() {
                true => (
                    filter.gain(),
                    filter.gain() * lift(filter.kind()) + tone.db(filter.frequency()),
                ),
                false => {
                    let gain = response.map_or(0., |response| response.db(filter.frequency()));
                    (gain, gain)
                }
            };
            ResponseNode {
                id: *id,
                position: ResponsePoint {
                    frequency_hz: filter.frequency() as f32,
                    gain_db: plotted_db(position),
                },
                readout_db: plotted_db(gain),
                gain_adjustable: filter.kind().has_gain(),
                selected: graph.selected == Some(*id),
                enabled: on && filter.enabled(),
            }
        })
        .collect();
    let reach = nodes
        .iter()
        .filter(|node| node.gain_adjustable)
        .map(|node| node.position.gain_db.abs())
        .fold(reach.max(preamp.abs() as f32), f32::max);
    let span = match graph.state.dragging() {
        true => graph.span.get(),
        false => fitted_span(reach),
    };
    graph.span.set(span);
    let (low, high) = GRAPH_FREQUENCIES;
    let axes = ResponseAxes::new(low as f32..=high as f32, -span..=span)
        .expect("the graph's axes are valid");
    let filters = curve.filters().len();

    let plot = plot(theme, width, height);
    let behind = ui
        .screens
        .equalizer
        .spectrum
        .view(plot, scale, still, theme);
    let response = ResponseGraph::new("equalizer-response", &graph.state, axes, width)
        .height(height)
        .response(points)
        .nodes(nodes)
        .reference(
            preamp as f32,
            t!("equalizer-graph-preamp", db = figure(preamp, 1)),
        )
        .bypassed(!on)
        .reshape(advanced)
        .on_edit(|ui: &mut Ui, edit: GraphEdit, _: &mut Tx| {
            graph_edit(ui, edit);
            ui.settle();
        })
        .build(theme);

    // The plot draws on a canvas that screen readers cannot see into, so its box stands in as
    // one image that says how many filters shape the sound and where the preamp sits.
    el().key("equalizer-graph")
        .role("image")
        .label(t!("equalizer-graph"))
        .description(t!(
            "equalizer-graph-detail",
            count = filters,
            db = figure(preamp, 1)
        ))
        .w(width)
        .h(height)
        .mt(crate::shared::page::rems(theme, 0.75))
        .child(behind)
        .child(response.absolute(0., 0.))
}

/// How tall the graph stands in `room`.
fn height(room: Room, theme: &Theme) -> f32 {
    let height = match room.fits(Room::Wide) {
        true => GRAPH_TALL,
        false => GRAPH_SHORT,
    };
    theme.metrics.cover * height
}

/// Where `ui::ResponseGraph` draws its plot in a graph `width` by `height`.
fn plot(theme: &Theme, width: f32, height: f32) -> [f32; 4] {
    let pad = theme.metrics.pad;
    let left = pad * INSET[0];
    let top = pad * INSET[1];
    [
        left,
        top,
        (width - pad * INSET[2] - left).max(0.),
        (height - pad * INSET[3] - top).max(0.),
    ]
}

/// The rate the output plays at, or the idle one while no stream is open.
fn rate(outputs: &Outputs) -> u32 {
    outputs
        .current()
        .map_or(IDLE_RATE, |device| device.format.rate)
}

/// The spectrum of what plays, drawn into pixels of its own behind the graph. A read is drawn
/// from the window's poll, so following it never runs the view: only its pixels change.
pub(super) struct Spectrum {
    pixels: Pixels,
    /// What the last view asked for: the plot, the device scale, the colour, and whether motion
    /// is reduced.
    frame: Cell<Frame>,
    /// Whether the frame changed since the pixels were last drawn.
    stale: Cell<bool>,
    levels: Vec<Level>,
    /// The levels shown when the latest read arrived, in dBFS, which it glides from.
    from: Vec<f32>,
    /// When the latest read arrived.
    arrived: Instant,
    /// When the spectrum last arrived after none, which its rise counts from.
    since: Option<Instant>,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Frame {
    plot: [f32; 4],
    scale: f32,
    color: Hsla,
    still: bool,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            pixels: Pixels::new(),
            frame: Cell::default(),
            stale: Cell::new(true),
            levels: Vec::new(),
            from: Vec::new(),
            arrived: Instant::now(),
            since: None,
        }
    }
}

impl Spectrum {
    /// The element the spectrum draws in, over the plot of a graph. Built again when the size,
    /// the scale or the theme change, which redraws the pixels on the next poll.
    fn view(&self, plot: [f32; 4], scale: f32, still: bool, theme: &Theme) -> El {
        let frame = Frame {
            plot,
            scale,
            color: theme.muted_foreground,
            still,
        };
        if self.frame.replace(frame) != frame {
            self.stale.set(true);
        }
        el().key("equalizer-spectrum")
            .absolute(plot[0], plot[1])
            .size(plot[2], plot[3])
            .pixels(&self.pixels)
            .a11y_hidden()
    }

    /// Clears the spectrum, for a watch that starts afresh.
    pub(super) fn reset(&mut self) {
        self.levels.clear();
        self.from.clear();
        self.since = None;
        self.stale.set(true);
    }

    /// Takes in a `read` of the levels when there is one and they differ from the last, and draws the spectrum where it
    /// glides or rises to now. Returns whether it is still moving between reads.
    pub(super) fn feed(&mut self, read: Option<Option<Vec<Level>>>) -> bool {
        let levels = read.map(Option::unwrap_or_default);
        let fresh = levels.as_ref().is_some_and(|levels| *levels != self.levels);
        if let Some(levels) = levels.filter(|_| fresh) {
            self.from = self.shown();
            self.arrived = Instant::now();
            self.levels = levels;
        }
        let rise = match self.levels.is_empty() {
            true => {
                self.since = None;
                1.
            }
            false => {
                let since = *self.since.get_or_insert_with(Instant::now);
                match self.frame.get().still {
                    true => 1.,
                    false => ease_out_cubic(
                        (since.elapsed().as_secs_f32() / SPECTRUM_RISE.as_secs_f32()).min(1.),
                    ),
                }
            }
        };
        let moving = !self.levels.is_empty() && (rise < 1. || self.glided() < 1.);
        if fresh || moving || self.stale.replace(false) {
            self.draw(rise);
        }
        moving
    }

    /// How far the latest read has glided in, from 0 as it arrives to 1. Reduced motion takes it
    /// in at once.
    fn glided(&self) -> f32 {
        match self.frame.get().still {
            true => 1.,
            false => (self.arrived.elapsed().as_secs_f32() / SPECTRUM_GLIDE.as_secs_f32()).min(1.),
        }
    }

    /// The levels on screen now, in dBFS, part way from the last read to the latest.
    fn shown(&self) -> Vec<f32> {
        let glided = self.glided();
        self.levels
            .iter()
            .enumerate()
            .map(|(index, level)| match self.from.get(index) {
                Some(from) => from + (level.dbfs - from) * glided,
                None => level.dbfs,
            })
            .collect()
    }

    /// Rasterises the spectrum, risen by `rise`, into the pixels: the area under it faint and
    /// its outline a hairline. It comes in the way sound fades in, a gain that climbs from
    /// below the plot's floor to none, so it rises from the bottom rather than appearing at once.
    fn draw(&self, rise: f32) {
        let frame = self.frame.get();
        let scale = frame.scale.max(1.);
        let width = (frame.plot[2] * scale).min(MAX_RASTER).round().max(1.) as u32;
        let height = (frame.plot[3] * scale).min(MAX_RASTER).round().max(1.) as u32;
        let (floor, ceiling) = SPECTRUM_SCALE;
        let fade = (floor - ceiling) * (1. - rise);
        let (low, high) = GRAPH_FREQUENCIES;
        let (low, high) = ((low as f32).ln(), (high as f32).ln());
        let points: Vec<[f32; 2]> = self
            .levels
            .iter()
            .zip(self.shown())
            .filter(|(level, dbfs)| level.frequency > 0. && dbfs.is_finite())
            .map(|(level, dbfs)| {
                let x = (level.frequency.ln() - low) / (high - low);
                let gain = (dbfs + fade).clamp(floor, ceiling);
                let y = (ceiling - gain) / (ceiling - floor);
                [
                    x.clamp(0., 1.) * width as f32,
                    y.clamp(0., 1.) * height as f32,
                ]
            })
            .collect();
        let mut alpha = vec![0f32; (width * height) as usize];
        if points.len() >= 2 {
            let crests = crests(&points, width);
            let line = HAIRLINE * scale / 2.;
            for (column, crest) in crests.iter().enumerate() {
                let Some(crest) = *crest else {
                    continue;
                };
                let before = crests[column.saturating_sub(1)].unwrap_or(crest);
                let after = crests[(column + 1).min(crests.len() - 1)].unwrap_or(crest);
                let slope = (after - before) / 2.;
                let stretch = (1. + slope * slope).sqrt();
                for y in 0..height {
                    let middle = y as f32 + 0.5;
                    let under = (middle - crest + 0.5).clamp(0., 1.) * SPECTRUM_FILL;
                    let distance = (middle - crest).abs() / stretch;
                    let stroke = (line + 0.5 - distance).clamp(0., 1.) * SPECTRUM_LINE;
                    alpha[(y * width) as usize + column] = stroke + under * (1. - stroke);
                }
            }
        }
        let ink = Rgba::from(frame.color);
        let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
        let (r, g, b) = (channel(ink.r), channel(ink.g), channel(ink.b));
        let mut rgba = Vec::with_capacity(alpha.len() * 4);
        for a in alpha {
            rgba.extend([r, g, b, channel(a * ink.a)]);
        }
        self.pixels.set(width, height, rgba);
    }
}

/// Where the line through `points` crosses each of `width` pixel columns, or `None` for a column
/// beyond its ends.
fn crests(points: &[[f32; 2]], width: u32) -> Vec<Option<f32>> {
    let mut at = 0;
    (0..width)
        .map(|column| {
            let x = column as f32 + 0.5;
            if x < points[0][0] || x > points[points.len() - 1][0] {
                return None;
            }
            while at + 2 < points.len() && points[at + 1][0] < x {
                at += 1;
            }
            let (a, b) = (points[at], points[at + 1]);
            let t = match b[0] - a[0] > f32::EPSILON {
                true => ((x - a[0]) / (b[0] - a[0])).clamp(0., 1.),
                false => 0.,
            };
            Some(a[1] + (b[1] - a[1]) * t)
        })
        .collect()
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

/// How far from 0 dB the response sampled at `points` reaches. That is its highest boost, or the
/// deepest cut of its peaks and shelves alone, read from the same curve without the gainless
/// filters.
fn reach(curve: &Curve, rate: u32, points: &[ResponsePoint]) -> f32 {
    let filters = curve
        .filters()
        .iter()
        .filter(|filter| filter.kind().has_gain())
        .copied()
        .collect();
    let gains = Curve::new(curve.preamp(), filters)
        .ok()
        .map(|curve| curve.response(rate));
    points
        .iter()
        .map(|point| {
            let cut = gains
                .as_ref()
                .map_or(0., |gains| gains.db(f64::from(point.frequency_hz)).abs());
            point.gain_db.max(cut as f32)
        })
        .fold(0., f32::max)
}

/// The narrowest of `GAIN_SPANS` that `reach` fits in with `HEADROOM` to spare, or the widest
/// when none does.
fn fitted_span(reach: f32) -> f32 {
    GAIN_SPANS
        .into_iter()
        .find(|span| reach + HEADROOM <= *span)
        .unwrap_or(GAIN_SPANS[GAIN_SPANS.len() - 1])
}

/// A dragged gain to a tenth of a decibel, within `span` either side of 0 dB and in range.
fn placed_gain(gain_db: f32, span: f32) -> f64 {
    let span = f64::from(span);
    ((f64::from(gain_db) * 10.).round() / 10.)
        .clamp(-span, span)
        .clamp(*GAIN_RANGE.start(), *GAIN_RANGE.end())
}

/// The share of its gain a filter's curve reaches at its frequency, where its node sits: half
/// for a shelf, whose centre is halfway up its step.
fn lift(kind: Kind) -> f64 {
    match kind {
        Kind::LowShelf | Kind::HighShelf => 0.5,
        _ => 1.,
    }
}

/// As low as a curve or a node is handed to the graph, so a notch's centre at minus infinity
/// stays finite and draws at the floor of whichever axis is shown.
fn plotted_db(db: f64) -> f32 {
    (db as f32).max(*GAIN_RANGE.start() as f32)
}
