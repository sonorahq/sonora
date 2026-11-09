//! The equalizer's frequency response graph: a log-frequency, linear-gain plot of a caller-sampled
//! response over its grid, with the filters as nodes the listener drags, plus the shared drawing
//! helpers the small response sketches use.
//!
//! The curve, the area under it, the grid and the spectrum are one `rune::canvas` shape; the
//! labels, the nodes and the readout are elements laid over it, so each node takes its own
//! presses, drags and wheel.

use std::cell::Cell;
use std::ops::RangeInclusive;
use std::rc::Rc;

use i18n::t;
use rune::{
    Color, Cursor, Drag, El, LineCap, Phase, PointerPress, Shape, TextStyle, Tx, WheelEvent,
    canvas, el, row, text,
};

use crate::color::Hsla;
use crate::metrics::Text;
use crate::motion::Springs;
use crate::theme::Theme;

const HAIRLINE: f32 = 1.;
const CURVE_STROKE: f32 = 2.;
/// The width of a node's ring and of the level bar across a node whose gain is fixed.
const RING: f32 = 2.;
/// How far a selected or hovered node's halo reaches, as a multiple of the node's radius.
const HALO: f32 = 1.8;
/// How big a node at rest is drawn, as a share of its radius, which a selected one grows to in
/// full.
const NODE_REST: f32 = 0.75;
/// How opaque the area under the response is at its furthest reach, and while bypassed.
const AREA_FILL: f32 = 0.3;
const BYPASSED_FILL: f32 = 0.1;
/// How many bands stack up the area under the response, each from a step further out to the
/// curve, so it is clearest at the zero line and strongest at the curve, as a gradient would be.
const AREA_BANDS: usize = 4;
/// How strongly the frequency lines, the level lines and the 0 dB line show over the plot.
const GRID_FAINT: f32 = 0.18;
const GRID_LEVEL: f32 = 0.3;
const GRID_ZERO: f32 = 0.3;
/// How strongly the axis labels show, quieter than the curve they describe.
const LABEL: f32 = 0.7;
/// How strongly the reference level's dashed line shows.
const REFERENCE: f32 = 0.3;
/// The dash and gap of the reference line, so it reads apart from the grid.
const DASH: [f32; 2] = [4., 3.];
/// How strongly the spectrum behind the plot fills and outlines.
const SPECTRUM_FILL: f32 = 0.07;
const SPECTRUM_LINE: f32 = 0.2;
/// How opaque a node's halo is when it is selected, and while only hovered.
const HALO_SELECTED: f32 = 0.22;
const HALO_HOVER: f32 = 0.14;
/// How far a held node must travel before it moves, so a click that selects it edits nothing.
const DEADZONE: f32 = 3.;
/// How wide a frequency label's box is, in paddings; labels closer than this are skipped.
const LABEL_ROOM: f32 = 6.;
/// The plot's inset from the graph's edges, in paddings: left, top, right, bottom.
const INSET: [f32; 4] = [7., 3., 3., 4.];
const FREQUENCIES: [f32; 10] = [
    20., 50., 100., 200., 500., 1000., 2000., 5000., 10000., 20000.,
];

type EditFn<S> = Rc<dyn Fn(&mut S, GraphEdit, &mut Tx)>;

/// One response sample or node centre, in Hz and dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResponsePoint {
    pub frequency_hz: f32,
    pub gain_db: f32,
}

/// A caller-owned filter handle. Ids must be unique and stable across builds. Bypassed nodes
/// stay editable; `gain_adjustable` decides whether a drag moves the gain.
#[derive(Clone, Copy, Debug)]
pub struct ResponseNode {
    pub id: u64,
    pub position: ResponsePoint,
    /// The gain the readout shows, which a node need not sit at, as a shelf's sits on its curve.
    pub readout_db: f32,
    pub gain_adjustable: bool,
    pub selected: bool,
    pub enabled: bool,
}

/// Validated logarithmic frequency and linear gain axes.
#[derive(Clone, Copy, Debug)]
pub struct ResponseAxes {
    frequency: [f32; 2],
    gain: [f32; 2],
}

impl ResponseAxes {
    /// Rejects nonfinite, reversed or empty ranges, and nonpositive frequencies.
    pub fn new(frequency: RangeInclusive<f32>, gain: RangeInclusive<f32>) -> Option<Self> {
        let frequency = [*frequency.start(), *frequency.end()];
        let gain = [*gain.start(), *gain.end()];
        (frequency.into_iter().chain(gain).all(f32::is_finite)
            && frequency[0] > 0.
            && frequency[0] < frequency[1]
            && gain[0] < gain[1])
            .then_some(Self { frequency, gain })
    }

    /// Maps a value into the plot `[x, y, w, h]`. Out-of-range values clamp to the plot edges;
    /// nonfinite inputs use the lower bound of their axis.
    pub fn project(self, value: ResponsePoint, plot: [f32; 4]) -> [f32; 2] {
        let frequency = finite_clamp(value.frequency_hz, self.frequency);
        let gain = finite_clamp(value.gain_db, self.gain);
        let x = (frequency.ln() - self.frequency[0].ln())
            / (self.frequency[1].ln() - self.frequency[0].ln());
        let y = (self.gain[1] - gain) / (self.gain[1] - self.gain[0]);
        [plot[0] + plot[2] * x, plot[1] + plot[3] * y]
    }

    /// Inverts `project`, clamping points outside the plot. A collapsed plot maps to its
    /// low-frequency, high-gain corner, so a hidden graph never divides by zero.
    pub fn unproject(self, at: [f32; 2], plot: [f32; 4]) -> ResponsePoint {
        let fraction = |value: f32, start: f32, length: f32| match length > 0. {
            true => ((value - start) / length).clamp(0., 1.),
            false => 0.,
        };
        let x = fraction(at[0], plot[0], plot[2]);
        let y = fraction(at[1], plot[1], plot[3]);
        ResponsePoint {
            frequency_hz: (self.frequency[0].ln()
                + x * (self.frequency[1].ln() - self.frequency[0].ln()))
            .exp()
            .clamp(self.frequency[0], self.frequency[1]),
            gain_db: self.gain[1] - y * (self.gain[1] - self.gain[0]),
        }
    }

    fn x(self, frequency_hz: f32, plot: [f32; 4]) -> f32 {
        let gain_db = self.gain[0];
        self.project(
            ResponsePoint {
                frequency_hz,
                gain_db,
            },
            plot,
        )[0]
    }

    fn y(self, gain_db: f32, plot: [f32; 4]) -> f32 {
        let frequency_hz = self.frequency[0];
        self.project(
            ResponsePoint {
                frequency_hz,
                gain_db,
            },
            plot,
        )[1]
    }
}

/// What the graph asks of the caller, which applies it and builds the graph with fresh data.
#[derive(Clone, Copy, Debug)]
pub enum GraphEdit {
    Select(u64),
    /// A press on the plot that missed every node and added none, which clears the selection.
    Deselect,
    /// A drag. An axis the pointer has not left the dead zone along keeps the node's position
    /// exactly as the caller gave it, so a node beyond the axes is not pulled to their edge.
    Move {
        id: u64,
        position: ResponsePoint,
    },
    /// Positive wheel steps ask for a wider filter; a trackpad gives fractional steps.
    Width {
        id: u64,
        steps: f32,
    },
    Add(ResponsePoint),
    Remove(u64),
}

/// A node held by the pointer: where it was at the press, and whether the pointer has left the
/// dead zone along the frequency and the gain axis.
#[derive(Clone, Copy)]
struct Grabbed {
    id: u64,
    origin: ResponsePoint,
    centre: [f32; 2],
    gain_adjustable: bool,
    along: (bool, bool),
}

/// Pointer bookkeeping kept beside the caller's EQ model, one per graph. Clones share it.
#[derive(Clone, Default)]
pub struct ResponseGraphState {
    grabbed: Rc<Cell<Option<Grabbed>>>,
    lit: Rc<Cell<Option<u64>>>,
}

impl ResponseGraphState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a node is held, from its press until the release. A caller that fits its axes to
    /// the data keeps them still while this holds, so a drag never shifts the scale under the
    /// pointer.
    pub fn dragging(&self) -> bool {
        self.grabbed.get().is_some()
    }

    /// Lights node `id` as if the pointer were over it, for a caller whose own list points at a
    /// node from outside the graph. Returns whether that changed anything.
    pub fn light(&self, id: Option<u64>) -> bool {
        self.lit.replace(id) != id
    }
}

/// Levels drawn behind the plot on a scale of their own.
struct Spectrum {
    points: Vec<ResponsePoint>,
    /// The levels at the bottom and the top of the plot.
    scale: [f32; 2],
}

/// A response plot with caller-sampled curves and caller-owned nodes. Samples must be in
/// ascending frequency order. A right-click removes a node, a double-click on empty plot adds one
/// and the wheel over a node widens it, unless reshaping is off.
pub struct ResponseGraph<S: 'static> {
    key: String,
    state: ResponseGraphState,
    axes: ResponseAxes,
    width: f32,
    height: Option<f32>,
    response: Vec<ResponsePoint>,
    spectrum: Option<Spectrum>,
    nodes: Vec<ResponseNode>,
    reference: Option<(f32, String)>,
    bypassed: bool,
    reshape: bool,
    grid: bool,
    on_edit: Option<EditFn<S>>,
}

impl<S: 'static> ResponseGraph<S> {
    /// An empty graph `width` points wide. Keep `state` beside the caller's EQ model.
    pub fn new(
        key: impl Into<String>,
        state: &ResponseGraphState,
        axes: ResponseAxes,
        width: f32,
    ) -> Self {
        Self {
            key: key.into(),
            state: state.clone(),
            axes,
            width,
            height: None,
            response: Vec::new(),
            spectrum: None,
            nodes: Vec::new(),
            reference: None,
            bypassed: false,
            reshape: true,
            grid: true,
            on_edit: None,
        }
    }

    /// The graph's height, twice the theme's cover size unless given.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// The total response, sampled in ascending frequency order.
    pub fn response(mut self, points: impl IntoIterator<Item = ResponsePoint>) -> Self {
        self.response = points.into_iter().collect();
        self
    }

    /// Levels behind the grid and the response, in ascending frequency order and on a scale of
    /// their own: `scale` runs from the level at the bottom of the plot to the one at the top.
    pub fn spectrum(
        mut self,
        points: impl IntoIterator<Item = ResponsePoint>,
        scale: RangeInclusive<f32>,
    ) -> Self {
        self.spectrum = (scale.start() < scale.end()).then(|| Spectrum {
            points: points.into_iter().collect(),
            scale: [*scale.start(), *scale.end()],
        });
        self
    }

    /// The handles, bypassed filters that stay editable included.
    pub fn nodes(mut self, nodes: impl IntoIterator<Item = ResponseNode>) -> Self {
        self.nodes = nodes.into_iter().collect();
        self
    }

    /// A dashed level across the plot at `gain_db`, named by `label` at its left end, such as
    /// the gain a preamp applies on top of the response.
    pub fn reference(mut self, gain_db: f32, label: impl Into<String>) -> Self {
        self.reference = Some((gain_db, label.into()));
        self
    }

    /// Draws the response and the reference level muted, for a curve that is not heard now.
    pub fn bypassed(mut self, bypassed: bool) -> Self {
        self.bypassed = bypassed;
        self
    }

    /// Whether a double-click adds a node, a right-click removes one and the wheel widens one.
    pub fn reshape(mut self, reshape: bool) -> Self {
        self.reshape = reshape;
        self
    }

    /// Leaves out the grid and its labels, for a layer drawn under a graph that has them.
    pub fn without_grid(mut self) -> Self {
        self.grid = false;
        self
    }

    /// Every edit the graph asks for. Without it the graph only draws.
    pub fn on_edit(mut self, f: impl Fn(&mut S, GraphEdit, &mut Tx) + 'static) -> Self {
        self.on_edit = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let pad = theme.metrics.pad;
        let width = self.width.max(0.);
        let height = self.height.unwrap_or(theme.metrics.cover * 2.);
        let left = pad * INSET[0];
        let top = pad * INSET[1];
        let plot = [
            left,
            top,
            (width - pad * INSET[2] - left).max(0.),
            (height - pad * INSET[3] - top).max(0.),
        ];
        let radius = pad;
        let tiny = TextStyle::new(theme.text(Text::Tiny), 400.);

        let mut graph = el()
            .key(self.key.clone())
            .role("group")
            .size(width, height)
            .radius(theme.radius)
            .clip_rounded(theme.radius)
            .child(
                canvas(self.shape(plot, theme))
                    .pin(Some(0.), Some(0.), Some(0.), Some(0.))
                    .a11y_hidden(),
            );
        if self.grid {
            graph = graph.children(self.labels(plot, theme, tiny));
        }
        if let Some((gain_db, label)) = &self.reference {
            let y = self.axes.y(*gain_db, plot);
            let color = match self.bypassed {
                true => theme.muted_foreground,
                false => theme.muted_foreground.opacity(LABEL),
            };
            graph = graph.child(
                text(label.clone())
                    .font(tiny)
                    .color(color.color())
                    .absolute(plot[0] + pad, y - tiny.size - pad),
            );
        }
        let lit = self
            .state
            .grabbed
            .get()
            .map(|grab| grab.id)
            .or(self.state.lit.get());
        let ordered = self
            .nodes
            .iter()
            .filter(|node| !node.selected)
            .chain(self.nodes.iter().filter(|node| node.selected));
        for node in ordered {
            let centre = self.axes.project(node.position, plot);
            graph = graph.child(self.node(node, centre, radius, lit == Some(node.id), theme));
        }
        if let Some(node) = self.nodes.iter().find(|node| node.selected) {
            let centre = self.axes.project(node.position, plot);
            graph = graph.child(readout(node, centre, plot, radius, theme, tiny));
        }

        let (Some(on_edit), true) = (self.on_edit.clone(), self.reshape) else {
            return match self.on_edit.clone() {
                Some(on_edit) => graph
                    .on_press_tx(move |s: &mut S, tx: &mut Tx| on_edit(s, GraphEdit::Deselect, tx)),
                None => graph,
            };
        };
        let axes = self.axes;
        graph.on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
            if drag.phase != Phase::Down {
                return;
            }
            let inside = drag.x >= plot[0]
                && drag.x <= plot[0] + plot[2]
                && drag.y >= plot[1]
                && drag.y <= plot[1] + plot[3];
            let edit = match drag.clicks >= 2 && inside {
                true => GraphEdit::Add(axes.unproject([drag.x, drag.y], plot)),
                false => GraphEdit::Deselect,
            };
            on_edit(s, edit, tx);
        })
    }

    /// The spectrum, the grid lines, the area under the response, the reference level and the
    /// response itself, in that order, as one shape in the graph's own points.
    fn shape(&self, plot: [f32; 4], theme: &Theme) -> Shape {
        let mut shape = Shape::new();
        if plot[2] <= 0. || plot[3] <= 0. {
            return shape;
        }
        let axes = self.axes;
        if let Some(spectrum) = &self.spectrum {
            let scale = ResponseAxes {
                gain: spectrum.scale,
                ..axes
            };
            let points = projected(&spectrum.points, scale, plot);
            if points.len() >= 2 {
                let floor = plot[1] + plot[3];
                let mut area = points.clone();
                area.push([points[points.len() - 1][0], floor]);
                area.push([points[0][0], floor]);
                shape = shape
                    .fill(area, theme.muted_foreground.opacity(SPECTRUM_FILL).color())
                    .stroke(
                        points,
                        HAIRLINE,
                        theme.muted_foreground.opacity(SPECTRUM_LINE).color(),
                    );
            }
        }
        if self.grid {
            shape = grid(shape, axes, plot, theme);
        }
        let curve = projected(&self.response, axes, plot);
        let (accent, fill, stroke, level) = match self.bypassed {
            true => (
                theme.muted_foreground,
                BYPASSED_FILL,
                theme.muted_foreground.opacity(0.7),
                theme.muted_foreground.opacity(0.5),
            ),
            false => (
                theme.primary,
                AREA_FILL,
                theme.primary,
                theme.muted_foreground.opacity(REFERENCE),
            ),
        };
        shape = area(shape, &curve, axes.y(0., plot), accent, fill);
        if let Some((gain_db, _)) = &self.reference {
            let y = axes.y(*gain_db, plot);
            shape = dashed(shape, plot[0], plot[0] + plot[2], y, level.color());
        }
        if curve.len() >= 2 {
            shape = shape.stroke(curve, CURVE_STROKE, stroke.color());
        }
        shape
    }

    /// The frequencies along the bottom and the gains down the left.
    fn labels(&self, plot: [f32; 4], theme: &Theme, tiny: TextStyle) -> Vec<El> {
        let pad = theme.metrics.pad;
        let color = theme.muted_foreground.opacity(LABEL).color();
        let room = pad * LABEL_ROOM;
        let mut labels = Vec::new();
        let mut last = plot[0] - pad * 10.;
        for hz in self.frequencies() {
            let x = self.axes.x(hz, plot);
            if x - last < room {
                continue;
            }
            last = x;
            let label = match hz >= 1000. {
                true => t!("response-graph-khz", value = hz / 1000.),
                false => t!("response-graph-hz", value = hz),
            };
            labels.push(
                row()
                    .w(room)
                    .justify_center()
                    .absolute(x - room / 2., plot[1] + plot[3] + pad)
                    .child(text(label.to_string()).font(tiny).color(color)),
            );
        }
        let step = gain_step(self.axes.gain[1] - self.axes.gain[0]);
        let start = (self.axes.gain[0] / step).ceil() as i32;
        let end = (self.axes.gain[1] / step).floor() as i32;
        // the floor's label would sit on the first frequency's in the corner
        for tick in start + 1..=end {
            let db = tick as f32 * step;
            let y = self.axes.y(db, plot);
            labels.push(
                row()
                    .w(room)
                    .justify_end()
                    .absolute(plot[0] - pad - room, y - tiny.size / 2.)
                    .child(
                        text(t!("response-graph-db", value = db).to_string())
                            .font(tiny)
                            .color(color),
                    ),
            );
        }
        labels
    }

    fn frequencies(&self) -> impl Iterator<Item = f32> + '_ {
        FREQUENCIES
            .into_iter()
            .filter(|hz| *hz >= self.axes.frequency[0] && *hz <= self.axes.frequency[1])
    }

    /// One node: a ring round a clear centre, or filled when selected, with a soft halo when it
    /// is selected, lit or under the pointer. A node whose gain is fixed carries a level bar.
    fn node(
        &self,
        node: &ResponseNode,
        centre: [f32; 2],
        radius: f32,
        lit: bool,
        theme: &Theme,
    ) -> El {
        let accent = match node.enabled {
            true => theme.primary,
            false => theme.muted_foreground,
        };
        let reach = radius * HALO;
        let dot = match node.selected || lit {
            true => radius,
            false => radius * NODE_REST,
        };
        let (fill, ring) = match node.selected {
            true => (accent, theme.background),
            false => (theme.background, accent),
        };
        let (glow, shown) = match (node.selected, lit) {
            (true, _) => (HALO_SELECTED, 1.),
            (false, true) => (HALO_HOVER, 1.),
            (false, false) => (HALO_HOVER, 0.),
        };
        let mut mark = el()
            .size(dot * 2., dot * 2.)
            .radius(dot)
            .fill(fill.color())
            .border(RING, ring.color())
            .center()
            .motion(Springs::QUICK);
        if !node.gain_adjustable {
            mark = mark.child(el().size(dot, RING).fill(ring.color()));
        }
        let mut handle = el()
            .key(format!("{}-node-{}", self.key, node.id))
            .role("button")
            .label(readout_text(node))
            .focusable(true)
            .absolute(centre[0] - reach, centre[1] - reach)
            .size(reach * 2., reach * 2.)
            .center()
            .hover_group()
            .cursor(Cursor::Grab)
            .child(
                el().pin(Some(0.), Some(0.), Some(0.), Some(0.))
                    .radius(reach)
                    .fill(accent.opacity(glow).color())
                    .group_opacity(shown, 1.),
            )
            .child(mark);
        let Some(on_edit) = self.on_edit.clone() else {
            return handle;
        };
        let id = node.id;
        let picked = on_edit.clone();
        handle =
            handle.on_press_tx(move |s: &mut S, tx: &mut Tx| picked(s, GraphEdit::Select(id), tx));
        let state = self.state.clone();
        let axes = self.axes;
        let plot = self.plot_of(theme);
        let origin = node.position;
        let gain_adjustable = node.gain_adjustable;
        let moved = on_edit.clone();
        handle = handle.on_drag(
            move |s: &mut S, drag: &mut Drag, tx: &mut Tx| match drag.phase {
                Phase::Down => {
                    state.grabbed.set(Some(Grabbed {
                        id,
                        origin,
                        centre,
                        gain_adjustable,
                        along: (false, false),
                    }));
                    moved(s, GraphEdit::Select(id), tx);
                }
                Phase::Move | Phase::Up => {
                    let Some(mut grab) = state.grabbed.get().filter(|grab| grab.id == id) else {
                        return;
                    };
                    grab.along.0 |= drag.dx.abs() >= DEADZONE;
                    grab.along.1 |= drag.dy.abs() >= DEADZONE;
                    state.grabbed.set(match drag.phase {
                        Phase::Up => None,
                        _ => Some(grab),
                    });
                    if drag.cancelled() {
                        let position = grab.origin;
                        moved(s, GraphEdit::Move { id, position }, tx);
                        return;
                    }
                    if grab.along == (false, false) {
                        return;
                    }
                    let pointer =
                        axes.unproject([grab.centre[0] + drag.dx, grab.centre[1] + drag.dy], plot);
                    let position = ResponsePoint {
                        frequency_hz: match grab.along.0 {
                            true => pointer.frequency_hz,
                            false => grab.origin.frequency_hz,
                        },
                        gain_db: match grab.along.1 && grab.gain_adjustable {
                            true => pointer.gain_db,
                            false => grab.origin.gain_db,
                        },
                    };
                    moved(s, GraphEdit::Move { id, position }, tx);
                }
            },
        );
        if !self.reshape {
            return handle;
        }
        let removed = on_edit.clone();
        let control = theme.metrics.control;
        handle
            .on_secondary_press(move |s: &mut S, _: PointerPress, tx: &mut Tx| {
                removed(s, GraphEdit::Remove(id), tx)
            })
            .on_wheel(move |s: &mut S, wheel: WheelEvent, tx: &mut Tx| {
                on_edit(
                    s,
                    GraphEdit::Width {
                        id,
                        steps: wheel.dy / control,
                    },
                    tx,
                );
                true
            })
    }

    fn plot_of(&self, theme: &Theme) -> [f32; 4] {
        let pad = theme.metrics.pad;
        let height = self.height.unwrap_or(theme.metrics.cover * 2.);
        let left = pad * INSET[0];
        let top = pad * INSET[1];
        [
            left,
            top,
            (self.width.max(0.) - pad * INSET[2] - left).max(0.),
            (height - pad * INSET[3] - top).max(0.),
        ]
    }
}

/// The selected node's frequency and gain in a popover-coloured tag above it, or below it when
/// the plot has no room above, led by a dot in the node's colour.
fn readout(
    node: &ResponseNode,
    centre: [f32; 2],
    plot: [f32; 4],
    radius: f32,
    theme: &Theme,
    tiny: TextStyle,
) -> El {
    let label = readout_text(node);
    let pad = theme.metrics.pad;
    let dot = tiny.size / 2.;
    let (text_width, _) = rune::text::measure(&label, &[], tiny, None);
    let width = pad + dot + pad * 0.75 + text_width + pad;
    let height = tiny.size + pad * 1.25;
    let gap = radius * HALO + pad / 2.;
    let above = centre[1] - gap - height;
    let top = match above >= plot[1] {
        true => above,
        false => centre[1] + gap,
    };
    let left = (centre[0] - width / 2.)
        .min(plot[0] + plot[2] - width)
        .max(plot[0]);
    let accent = match node.enabled {
        true => theme.primary,
        false => theme.muted_foreground,
    };
    row()
        .absolute(left, top)
        .size(width, height)
        .px(pad)
        .gap(pad * 0.75)
        .center_y()
        .radius(theme.radius.min(height / 2.))
        .fill(theme.popover.color())
        .border(HAIRLINE, theme.border.color())
        .a11y_hidden()
        .motion(Springs::QUICK)
        .child(el().size(dot, dot).radius(dot / 2.).fill(accent.color()))
        .child(
            text(label)
                .font(tiny)
                .color(theme.popover_foreground.color()),
        )
}

/// The readout a node's tag shows and a screen reader reads: its frequency and signed gain.
fn readout_text(node: &ResponseNode) -> String {
    let hz = node.position.frequency_hz;
    let frequency = match hz >= 1000. {
        // Rounded in f64, because an f32 hundredth widens to a long tail of digits in Fluent.
        true => t!(
            "response-graph-khz",
            value = f64::from((hz / 10.).round()) / 100.
        ),
        false => t!("response-graph-hz", value = hz.round()),
    };
    // A tenth of a decibel, rounded in f64 for the same reason, and the sign left to Fluent.
    let gain = (f64::from(node.readout_db) * 10.).round() / 10.;
    let direction = match gain {
        gain if gain > 0. => "up",
        gain if gain < 0. => "down",
        _ => "flat",
    };
    t!(
        "response-graph-readout",
        frequency = frequency.to_string(),
        gain = gain.abs(),
        direction = direction
    )
    .to_string()
}

/// A faint line per labelled frequency and per level step, the 0 dB line a little stronger.
fn grid(mut shape: Shape, axes: ResponseAxes, plot: [f32; 4], theme: &Theme) -> Shape {
    let pad = theme.metrics.pad;
    let (top, bottom) = (plot[1], plot[1] + plot[3]);
    let (left, right) = (plot[0], plot[0] + plot[2]);
    let mut last = left - pad * 10.;
    for hz in FREQUENCIES
        .into_iter()
        .filter(|hz| *hz >= axes.frequency[0] && *hz <= axes.frequency[1])
    {
        let x = axes.x(hz, plot);
        if x - last < pad * LABEL_ROOM {
            continue;
        }
        last = x;
        shape = shape.line(
            [x, top],
            [x, bottom],
            HAIRLINE,
            LineCap::Butt,
            theme.border.opacity(GRID_FAINT).color(),
        );
    }
    let step = gain_step(axes.gain[1] - axes.gain[0]);
    let start = (axes.gain[0] / step).ceil() as i32;
    let end = (axes.gain[1] / step).floor() as i32;
    for tick in start..=end {
        let y = axes.y(tick as f32 * step, plot);
        let color = match tick == 0 {
            true => theme.muted_foreground.opacity(GRID_ZERO),
            false => theme.border.opacity(GRID_LEVEL),
        };
        shape = shape.line(
            [left, y],
            [right, y],
            HAIRLINE,
            LineCap::Butt,
            color.color(),
        );
    }
    shape
}

/// The distance between gain labels for an axis `span` decibels tall: 1, 2, 3, 6 or 12 times a
/// power of ten, the first to leave no more than about six gaps.
fn gain_step(span: f32) -> f32 {
    let target = span / 6.;
    let decade = 10f32.powf(target.log10().floor());
    [1., 2., 3., 6., 12.]
        .into_iter()
        .find(|n| n * decade >= target)
        .unwrap_or(12.)
        * decade
}

/// The area between the zero line at `zero` and the response `curve`, the boosts and the cuts
/// apart, each stacked from `AREA_BANDS` bands that reach from a step further out to the curve,
/// so it fades from `fill` at the curve to clear at the line.
fn area(mut shape: Shape, curve: &[[f32; 2]], zero: f32, color: Hsla, fill: f32) -> Shape {
    if curve.len() < 2 {
        return shape;
    }
    let band = 1. - (1. - fill).powf(1. / AREA_BANDS as f32);
    let color = color.opacity(band).color();
    for boost in [true, false] {
        let away = |y: f32| match boost {
            true => y < zero,
            false => y > zero,
        };
        for run in runs(curve, zero, &away) {
            for step in 0..AREA_BANDS {
                let t = step as f32 / AREA_BANDS as f32;
                let mut polygon = run.clone();
                polygon.extend(run.iter().rev().map(|[x, y]| [*x, zero + (y - zero) * t]));
                shape = shape.fill(polygon, color);
            }
        }
    }
    shape
}

/// The stretches of `curve` on one side of the zero line, each starting and ending where the
/// curve crosses it.
fn runs(curve: &[[f32; 2]], zero: f32, away: &impl Fn(f32) -> bool) -> Vec<Vec<[f32; 2]>> {
    let crossing = |a: [f32; 2], b: [f32; 2]| {
        let t = match (b[1] - a[1]).abs() > f32::EPSILON {
            true => ((zero - a[1]) / (b[1] - a[1])).clamp(0., 1.),
            false => 0.,
        };
        [a[0] + (b[0] - a[0]) * t, zero]
    };
    let mut runs = Vec::new();
    let mut open: Option<Vec<[f32; 2]>> = None;
    for (index, point) in curve.iter().enumerate() {
        match (away(point[1]), open.as_mut()) {
            (true, Some(run)) => run.push(*point),
            (true, None) => {
                let start = match index {
                    0 => [point[0], zero],
                    _ => crossing(curve[index - 1], *point),
                };
                open = Some(vec![start, *point]);
            }
            (false, Some(_)) => {
                if let Some(mut run) = open.take() {
                    run.push(crossing(curve[index - 1], *point));
                    runs.push(run);
                }
            }
            (false, None) => {}
        }
    }
    if let Some(mut run) = open
        && let Some(last) = curve.last()
    {
        run.push([last[0], zero]);
        runs.push(run);
    }
    runs
}

/// A dashed level from `left` to `right` at `y`.
fn dashed(mut shape: Shape, left: f32, right: f32, y: f32, color: Color) -> Shape {
    let mut x = left;
    while x < right {
        let end = (x + DASH[0]).min(right);
        shape = shape.line([x, y], [end, y], HAIRLINE, LineCap::Butt, color);
        x += DASH[0] + DASH[1];
    }
    shape
}

/// `points` mapped into the plot, nonfinite samples left out.
pub(crate) fn projected(
    points: &[ResponsePoint],
    axes: ResponseAxes,
    plot: [f32; 4],
) -> Vec<[f32; 2]> {
    points
        .iter()
        .filter(|value| value.frequency_hz.is_finite() && value.gain_db.is_finite())
        .map(|value| axes.project(*value, plot))
        .collect()
}

/// `value` clamped to `range`, or the range's low end when it is not a number at all.
fn finite_clamp(value: f32, range: [f32; 2]) -> f32 {
    match value.is_finite() {
        true => value.clamp(range[0], range[1]),
        false => range[0],
    }
}
