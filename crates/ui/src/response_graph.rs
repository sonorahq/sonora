use gpui::prelude::*;
use gpui::{
    App, Bounds, Div, DragMoveEvent, ElementId, Empty, Hsla, MouseButton, MouseDownEvent,
    MouseUpEvent, Path, PathBuilder, Pixels, Point, Render, ScrollDelta, ShapedLine, SharedString,
    Stateful, StyleRefinement, TextRun, Window, canvas, div, linear_color_stop, linear_gradient,
    point, px, size,
};
use i18n::t;
use std::{cell::Cell, ops::RangeInclusive, rc::Rc};

use crate::{ActiveTheme as _, Text, Theme};

const HAIRLINE: Pixels = px(1.);
const CURVE_STROKE: Pixels = px(2.);
/// The width of a node's ring and of the level bar across a node whose gain is fixed.
const RING: Pixels = px(2.);
/// How far a selected or hovered node's halo reaches, as a multiple of the node's radius.
const HALO: f32 = 1.8;
/// How big a node at rest is drawn, as a share of its radius, which a selected or hovered one
/// grows to in full.
const NODE_REST: f32 = 0.75;
/// How opaque the area under the response is at its furthest reach, and while bypassed.
const AREA_FILL: f32 = 0.3;
const BYPASSED_FILL: f32 = 0.1;
/// How strongly the frequency lines, the level lines and the 0 dB line show over the plot.
const GRID_FAINT: f32 = 0.18;
const GRID_LEVEL: f32 = 0.3;
const GRID_ZERO: f32 = 0.3;
/// How strongly the axis labels show, quieter than the curve they describe.
const LABEL: f32 = 0.7;
/// How strongly the reference level's dashed line shows.
const REFERENCE: f32 = 0.3;
/// The dash and gap of the reference line, so it reads apart from the grid.
const DASH: [Pixels; 2] = [px(4.), px(3.)];
/// How many lines one wheel notch scrolls, so a notch is one width step on every platform.
#[cfg(target_os = "macos")]
const LINES_PER_NOTCH: f32 = 1.;
#[cfg(not(target_os = "macos"))]
const LINES_PER_NOTCH: f32 = 3.;
/// How far a held node must travel before it moves, so a click that selects it edits nothing.
const DEADZONE: f32 = 3.;
/// The curve coordinates that mark a path's triangle as wholly inside its shape.
const INSIDE: (Point<f32>, Point<f32>, Point<f32>) = (
    Point { x: 0., y: 1. },
    Point { x: 0., y: 1. },
    Point { x: 0., y: 1. },
);
const FREQUENCIES: [f32; 10] = [
    20., 50., 100., 200., 500., 1000., 2000., 5000., 10000., 20000.,
];

type Edit = Rc<dyn Fn(&GraphEdit, &mut Window, &mut App)>;

/// One response sample or node centre, in Hz and dB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResponsePoint {
    pub frequency_hz: f32,
    pub gain_db: f32,
}

/// Caller-owned filter handle. IDs must be unique and stable across renders.
/// Bypassed nodes remain editable; `gain_adjustable` controls vertical movement.
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

    /// Maps a value into plot pixels. Out-of-range values clamp to the plot edges;
    /// nonfinite inputs use the lower bound of their axis.
    pub fn project(self, value: ResponsePoint, bounds: Bounds<Pixels>) -> Point<Pixels> {
        let frequency = finite_clamp(value.frequency_hz, self.frequency);
        let gain = finite_clamp(value.gain_db, self.gain);
        let x = (frequency.ln() - self.frequency[0].ln())
            / (self.frequency[1].ln() - self.frequency[0].ln());
        let y = (self.gain[1] - gain) / (self.gain[1] - self.gain[0]);
        point(
            bounds.left() + bounds.size.width * x,
            bounds.top() + bounds.size.height * y,
        )
    }

    /// Inverts `project`, clamping outside pixels. A collapsed plot maps to its low-frequency,
    /// high-gain corner so a hidden or not-yet-measured element never divides by zero.
    pub fn unproject(self, position: Point<Pixels>, bounds: Bounds<Pixels>) -> ResponsePoint {
        let fraction = |value: Pixels, start: Pixels, length: Pixels| match length > Pixels::ZERO {
            true => ((value - start) / length).clamp(0., 1.),
            false => 0.,
        };
        let x = fraction(position.x, bounds.left(), bounds.size.width);
        let y = fraction(position.y, bounds.top(), bounds.size.height);
        ResponsePoint {
            frequency_hz: (self.frequency[0].ln()
                + x * (self.frequency[1].ln() - self.frequency[0].ln()))
            .exp()
            .clamp(self.frequency[0], self.frequency[1]),
            gain_db: self.gain[1] - y * (self.gain[1] - self.gain[0]),
        }
    }

    /// Where `frequency_hz` falls across the plot.
    fn x(self, frequency_hz: f32, bounds: Bounds<Pixels>) -> Pixels {
        let gain_db = self.gain[0];
        self.project(
            ResponsePoint {
                frequency_hz,
                gain_db,
            },
            bounds,
        )
        .x
    }

    /// Where `gain_db` falls down the plot.
    fn y(self, gain_db: f32, bounds: Bounds<Pixels>) -> Pixels {
        let frequency_hz = self.frequency[0];
        self.project(
            ResponsePoint {
                frequency_hz,
                gain_db,
            },
            bounds,
        )
        .y
    }
}

/// Intent emitted by the graph. The caller applies edits and supplies fresh data on render.
#[derive(Clone, Copy, Debug)]
pub enum GraphEdit {
    Select(u64),
    /// A press on the graph that missed every node and added none, which clears the selection.
    Deselect,
    /// A drag. An axis the pointer has not left the dead zone along keeps the node's position
    /// exactly as the caller gave it, so a node beyond the axes is not pulled to their edge.
    Move {
        id: u64,
        position: ResponsePoint,
    },
    /// Positive wheel steps request a wider filter; precise scrolling emits fractional steps.
    Width {
        id: u64,
        steps: f32,
    },
    Add(ResponsePoint),
    Remove(u64),
}

/// Persistent pointer bookkeeping, separate from the caller's EQ model.
/// Keep one instance per graph, with an ID unique in its window.
#[derive(Clone)]
pub struct ResponseGraphState {
    id: SharedString,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    grabbed: Rc<Cell<Option<Grabbed>>>,
    /// The node the pointer was over when it last moved, which is the only one the wheel edits,
    /// so a page scrolled under a still pointer never changes a filter.
    hovered: Rc<Cell<Option<u64>>>,
}

impl ResponseGraphState {
    /// Creates pointer state to retain alongside the caller's filter data.
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            bounds: Rc::default(),
            grabbed: Rc::default(),
            hovered: Rc::default(),
        }
    }

    /// Whether a node is held, from its press until the button is released anywhere. A caller
    /// that fits its axes to the data keeps them still while this holds, so a drag never shifts
    /// the scale under the pointer.
    pub fn dragging(&self) -> bool {
        self.grabbed.get().is_some()
    }

    /// Lights node `id` as if the pointer were over it, for a caller whose own list points at a
    /// node from outside the graph. Returns whether that changed anything, so the caller knows
    /// to redraw. The wheel still edits only a node under the pointer.
    pub fn light(&self, id: Option<u64>) -> bool {
        self.hovered.replace(id) != id
    }
}

/// A pointer grab retains its initial offset even when the caller changes the node's data.
#[derive(Clone, Copy)]
struct Grabbed {
    id: u64,
    press: Point<Pixels>,
    offset: Point<Pixels>,
    /// The node's position as the caller gave it at the press.
    origin: ResponsePoint,
    gain_adjustable: bool,
    /// Whether the pointer has left the dead zone along the frequency and along the gain axis.
    /// A release before either is only a selection.
    along: (bool, bool),
}

/// Identifies this graph in GPUI's window-wide drag dispatch.
#[derive(Clone)]
struct GraphDrag(SharedString);

impl Render for GraphDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// A response plot with caller-sampled curves and caller-owned nodes. Samples must be in
/// ascending frequency order. Right-click removes a node and double-click on empty plot space
/// adds one, unless reshaping is off.
#[derive(IntoElement)]
pub struct ResponseGraph {
    base: Stateful<Div>,
    state: ResponseGraphState,
    axes: ResponseAxes,
    response: Vec<ResponsePoint>,
    spectrum: Option<Spectrum>,
    nodes: Vec<ResponseNode>,
    reference: Option<(f32, SharedString)>,
    bypassed: bool,
    reshape: bool,
    grid: bool,
    on_edit: Option<Edit>,
}

/// Levels drawn behind the plot on a scale of their own.
struct Spectrum {
    points: Vec<ResponsePoint>,
    /// The levels at the bottom and the top of the plot.
    scale: [f32; 2],
}

impl ResponseGraph {
    /// Starts an empty graph. Retain `state` between renders and give the element its size.
    pub fn new(state: &ResponseGraphState, axes: ResponseAxes) -> Self {
        Self {
            base: div().id(ElementId::Name(state.id.clone())),
            state: state.clone(),
            axes,
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

    /// Sets the total response sampled in ascending frequency order.
    pub fn response(mut self, points: impl IntoIterator<Item = ResponsePoint>) -> Self {
        self.response = points.into_iter().collect();
        self
    }

    /// Draws levels behind the grid and the response, in ascending frequency order and on a
    /// scale of their own: `scale` runs from the level at the bottom of the plot to the one at
    /// the top, in dBFS.
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

    /// Sets the handles, including bypassed filters that should remain editable.
    pub fn nodes(mut self, nodes: impl IntoIterator<Item = ResponseNode>) -> Self {
        self.nodes = nodes.into_iter().collect();
        self
    }

    /// Draws a dashed level across the plot at `gain_db`, named by `label` at its left end, such
    /// as the gain a preamp applies on top of the response.
    pub fn reference(mut self, gain_db: f32, label: impl Into<SharedString>) -> Self {
        self.reference = Some((gain_db, label.into()));
        self
    }

    /// Draws the response and the reference level muted, for a curve that is not heard now.
    pub fn bypassed(mut self, bypassed: bool) -> Self {
        self.bypassed = bypassed;
        self
    }

    /// Whether double-click adds a node, right-click removes one and the wheel widens one. Off,
    /// nodes can only be selected and dragged and the wheel scrolls what holds the graph.
    pub fn reshape(mut self, reshape: bool) -> Self {
        self.reshape = reshape;
        self
    }

    /// Leaves out the grid and its labels, for a layer drawn under a graph that has them.
    pub fn without_grid(mut self) -> Self {
        self.grid = false;
        self
    }

    /// Reports all edit intents, including a final move on release outside the plot.
    pub fn on_edit(
        mut self,
        handler: impl Fn(&GraphEdit, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_edit = Some(Rc::new(handler));
        self
    }
}

impl Styled for ResponseGraph {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

/// One render's data and hit-testing rules, shared by painting and the pointer handlers, which
/// outlive the render that made them.
struct Plot {
    state: ResponseGraphState,
    axes: ResponseAxes,
    nodes: Vec<ResponseNode>,
    reference: Option<(f32, SharedString)>,
    bypassed: bool,
    reshape: bool,
    grid: bool,
    radius: Pixels,
    on_edit: Option<Edit>,
}

impl Plot {
    fn emit(&self, edit: GraphEdit, window: &mut Window, cx: &mut App) {
        if let Some(handler) = &self.on_edit {
            handler(&edit, window, cx);
        }
    }

    /// A selected node wins overlaps, then the nearest centre, then the later-drawn node.
    fn hit(&self, position: Point<Pixels>) -> Option<ResponseNode> {
        self.nodes
            .iter()
            .rev()
            .filter_map(|node| {
                let centre = self.axes.project(node.position, self.state.bounds.get());
                let dx = (position.x - centre.x).as_f32();
                let dy = (position.y - centre.y).as_f32();
                let distance = dx * dx + dy * dy;
                (distance <= (self.radius * 1.5).as_f32().powi(2)).then_some((*node, distance))
            })
            .min_by(|(a, da), (b, db)| b.selected.cmp(&a.selected).then(da.total_cmp(db)))
            .map(|(node, _)| node)
    }

    /// Selects the node under the pointer, capturing the offset before selection can rerender
    /// the caller's node data. A double click on empty plot adds a node while reshaping is on,
    /// and any other press off a node deselects.
    fn press(&self, event: &MouseDownEvent, window: &mut Window, cx: &mut App) {
        self.state.grabbed.set(None);
        if let Some(node) = self.hit(event.position) {
            let centre = self.axes.project(node.position, self.state.bounds.get());
            self.state.grabbed.set(Some(Grabbed {
                id: node.id,
                press: event.position,
                offset: event.position - centre,
                origin: node.position,
                gain_adjustable: node.gain_adjustable,
                along: (false, false),
            }));
            self.emit(GraphEdit::Select(node.id), window, cx);
            cx.stop_propagation();
        } else if self.reshape
            && event.click_count == 2
            && self.state.bounds.get().contains(&event.position)
        {
            self.emit(
                GraphEdit::Add(self.axes.unproject(event.position, self.state.bounds.get())),
                window,
                cx,
            );
            cx.stop_propagation();
        } else {
            self.emit(GraphEdit::Deselect, window, cx);
        }
    }

    /// Retains the grab constraint and consumes it on release, including outside the plot, when it
    /// also redraws so axes held for the drag can refit. Nothing is emitted until the pointer
    /// leaves the dead zone along an axis.
    fn moved(&self, position: Point<Pixels>, finished: bool, window: &mut Window, cx: &mut App) {
        let Some(mut grab) = self.state.grabbed.get() else {
            return;
        };
        let travel = position - grab.press;
        grab.along.0 |= travel.x.as_f32().abs() >= DEADZONE;
        grab.along.1 |= travel.y.as_f32().abs() >= DEADZONE;
        self.state.grabbed.set(match finished {
            true => None,
            false => Some(grab),
        });
        if finished {
            window.refresh();
        }
        if grab.along == (false, false) || !self.nodes.iter().any(|node| node.id == grab.id) {
            return;
        }
        let pointer = self
            .axes
            .unproject(position - grab.offset, self.state.bounds.get());
        let value = ResponsePoint {
            frequency_hz: match grab.along.0 {
                true => pointer.frequency_hz,
                false => grab.origin.frequency_hz,
            },
            gain_db: match grab.along.1 && grab.gain_adjustable {
                true => pointer.gain_db,
                false => grab.origin.gain_db,
            },
        };
        self.emit(
            GraphEdit::Move {
                id: grab.id,
                position: value,
            },
            window,
            cx,
        );
    }
}

impl RenderOnce for ResponseGraph {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            state,
            axes,
            response,
            spectrum,
            nodes,
            reference,
            bypassed,
            reshape,
            grid,
            on_edit,
        } = self;
        let theme = *cx.theme();
        let overrides = std::mem::take(base.style());
        let interactive = on_edit.is_some();
        let plot = Rc::new(Plot {
            state: state.clone(),
            axes,
            nodes,
            reference,
            bypassed,
            reshape,
            grid,
            radius: theme.metrics.pad,
            on_edit,
        });
        let measure = state.bounds.clone();
        let painted = plot.clone();
        let pressed = plot.clone();
        let dragged = plot.clone();
        let released = plot.clone();
        let removed = plot.clone();
        let hovering = plot.clone();
        let release = move |event: &MouseUpEvent, window: &mut Window, cx: &mut App| {
            released.moved(event.position, true, window, cx);
        };
        let mut graph = base
            .relative()
            .w_full()
            .h(theme.metrics.cover * 2.)
            .overflow_hidden()
            .rounded(theme.radius)
            .text_size(theme.text(Text::Tiny))
            .child(
                canvas(
                    move |bounds, _, _| {
                        let pad = theme.metrics.pad;
                        let plot_bounds = Bounds::from_corners(
                            point(bounds.left() + pad * 7., bounds.top() + pad * 3.),
                            point(
                                (bounds.right() - pad * 3.).max(bounds.left() + pad * 7.),
                                (bounds.bottom() - pad * 4.).max(bounds.top() + pad * 3.),
                            ),
                        );
                        measure.set(plot_bounds);
                        plot_bounds
                    },
                    move |_, bounds, window, cx| {
                        // a graph scrolled out of view costs nothing to skip and a lot to tessellate
                        if !window.content_mask().bounds.intersects(&bounds) {
                            return;
                        }
                        paint_plot(bounds, &painted, &response, &spectrum, &theme, window, cx);
                    },
                )
                .absolute()
                .size_full(),
            )
            .when(interactive, |this| {
                this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    pressed.press(event, window, cx)
                })
                .on_drag(GraphDrag(state.id.clone()), |drag, _, _, cx| {
                    cx.new(|_| drag.clone())
                })
                .on_drag_move(move |event: &DragMoveEvent<GraphDrag>, window, cx| {
                    if event.drag(cx).0 == dragged.state.id {
                        dragged.moved(event.event.position, false, window, cx);
                    }
                })
                .on_mouse_up(MouseButton::Left, release.clone())
                .on_mouse_up_out(MouseButton::Left, release)
                .when(reshape, |this| {
                    this.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        if let Some(node) = removed.hit(event.position) {
                            removed.state.grabbed.set(None);
                            removed.emit(GraphEdit::Remove(node.id), window, cx);
                            cx.stop_propagation();
                        }
                    })
                })
                .on_mouse_move(move |event, window, _| {
                    let hit = hovering.hit(event.position).map(|node| node.id);
                    if hovering.state.hovered.replace(hit) != hit {
                        window.refresh();
                    }
                })
                .on_hover({
                    let state = state.clone();
                    move |hovered, window, _| {
                        if !hovered && state.hovered.replace(None).is_some() {
                            window.refresh();
                        }
                    }
                })
                .when(reshape, |this| {
                    this.on_scroll_wheel(move |event, window, cx| {
                        let hovered = plot.state.hovered.get();
                        if let Some(node) = plot
                            .hit(event.position)
                            .filter(|node| hovered == Some(node.id))
                        {
                            let steps = match event.delta {
                                ScrollDelta::Lines(delta) => delta.y / LINES_PER_NOTCH,
                                ScrollDelta::Pixels(delta) => delta.y / theme.metrics.control,
                            };
                            plot.emit(GraphEdit::Width { id: node.id, steps }, window, cx);
                            cx.stop_propagation();
                        }
                    })
                })
            });
        graph.style().refine(&overrides);
        graph
    }
}

/// `value` clamped to `range`, or the range's low end when it is not a number at all.
fn finite_clamp(value: f32, range: [f32; 2]) -> f32 {
    match value.is_finite() {
        true => value.clamp(range[0], range[1]),
        false => range[0],
    }
}

/// Paints the spectrum, the grid and its labels, the area under the response, the reference
/// level, the response and the nodes, in that order, inside the plot's `bounds`.
fn paint_plot(
    bounds: Bounds<Pixels>,
    plot: &Plot,
    response: &[ResponsePoint],
    spectrum: &Option<Spectrum>,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    if bounds.size.width <= Pixels::ZERO || bounds.size.height <= Pixels::ZERO {
        return;
    }
    let pad = theme.metrics.pad;
    let axes = plot.axes;
    if let Some(spectrum) = spectrum {
        paint_spectrum(bounds, axes, spectrum, theme, window);
    }
    if plot.grid {
        paint_grid(bounds, axes, theme, window, cx);
    }
    let curve: Vec<_> = response
        .iter()
        .filter(|value| value.frequency_hz.is_finite() && value.gain_db.is_finite())
        .map(|value| axes.project(*value, bounds))
        .collect();
    let (accent, fill, stroke, level, label) = match plot.bypassed {
        true => (
            theme.muted_foreground,
            BYPASSED_FILL,
            theme.muted_foreground.opacity(0.7),
            theme.muted_foreground.opacity(0.5),
            theme.muted_foreground,
        ),
        false => (
            theme.primary,
            AREA_FILL,
            theme.primary,
            theme.muted_foreground.opacity(REFERENCE),
            theme.muted_foreground.opacity(LABEL),
        ),
    };
    paint_area(&curve, axes.y(0., bounds), accent, fill, window);
    let reference = plot
        .reference
        .as_ref()
        .map(|(gain_db, label)| (axes.y(*gain_db, bounds), label));
    if let Some((y, _)) = reference {
        let mut path = PathBuilder::stroke(HAIRLINE).dash_array(&DASH);
        path.add_polygon(&[point(bounds.left(), y), point(bounds.right(), y)], false);
        if let Ok(path) = path.build() {
            window.paint_path(path, level);
        }
    }
    line(&curve, CURVE_STROKE, stroke, window);
    if let Some((y, name)) = reference {
        text(
            name.clone(),
            point(bounds.left() + pad, y - theme.text(Text::Tiny) - pad),
            0.,
            label,
            None,
            window,
            cx,
        );
    }
    let lit = plot
        .state
        .grabbed
        .get()
        .map(|grab| grab.id)
        .or_else(|| plot.state.hovered.get());
    for node in plot
        .nodes
        .iter()
        .filter(|node| !node.selected)
        .chain(plot.nodes.iter().filter(|node| node.selected))
    {
        paint_node(
            node,
            axes.project(node.position, bounds),
            plot.radius,
            lit == Some(node.id),
            theme,
            window,
        );
    }
    if let Some(node) = plot.nodes.iter().find(|node| node.selected) {
        readout(
            node,
            axes.project(node.position, bounds),
            bounds,
            plot.radius,
            theme,
            window,
            cx,
        );
    }
}

/// Labels the frequencies along the bottom and the gains down the left, each with a faint line
/// across the plot, and draws the 0 dB line a little stronger than the rest.
fn paint_grid(
    bounds: Bounds<Pixels>,
    axes: ResponseAxes,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
    let pad = theme.metrics.pad;
    let mut last_label = bounds.left() - pad * 10.;
    for hz in FREQUENCIES
        .into_iter()
        .filter(|hz| *hz >= axes.frequency[0] && *hz <= axes.frequency[1])
    {
        let x = axes.x(hz, bounds);
        if x - last_label < pad * 6. {
            continue;
        }
        last_label = x;
        window.paint_quad(gpui::fill(
            Bounds::new(
                point(x - HAIRLINE / 2., bounds.top()),
                size(HAIRLINE, bounds.size.height),
            ),
            theme.border.opacity(GRID_FAINT),
        ));
        let label = match hz >= 1000. {
            true => t!("response-graph-khz", value = hz / 1000.),
            false => t!("response-graph-hz", value = hz),
        };
        text(
            label,
            point(x, bounds.bottom() + pad),
            0.5,
            theme.muted_foreground.opacity(LABEL),
            None,
            window,
            cx,
        );
    }
    let step = gain_step(axes.gain[1] - axes.gain[0]);
    let start = (axes.gain[0] / step).ceil() as i32;
    let end = (axes.gain[1] / step).floor() as i32;
    for tick in start..=end {
        let db = tick as f32 * step;
        let y = axes.y(db, bounds);
        let color = match tick == 0 {
            true => theme.muted_foreground.opacity(GRID_ZERO),
            false => theme.border.opacity(GRID_LEVEL),
        };
        window.paint_quad(gpui::fill(
            Bounds::new(
                point(bounds.left(), y - HAIRLINE / 2.),
                size(bounds.size.width, HAIRLINE),
            ),
            color,
        ));
        // the floor's label would sit on the first frequency's in the corner
        if tick == start {
            continue;
        }
        text(
            t!("response-graph-db", value = db),
            point(bounds.left() - pad, y - theme.text(Text::Tiny) / 2.),
            1.,
            theme.muted_foreground.opacity(LABEL),
            None,
            window,
            cx,
        );
    }
}

/// The distance between gain labels for an axis `span` decibels tall: 1, 2, 3, 6 or 12 times a
/// power of ten, the first to leave no more than about six gaps. A decibel axis then reads in
/// steps of 6, the ones an ear and an engineer count in.
fn gain_step(span: f32) -> f32 {
    let target = span / 6.;
    let decade = 10f32.powf(target.log10().floor());
    [1., 2., 3., 6., 12.]
        .into_iter()
        .find(|n| n * decade >= target)
        .unwrap_or(12.)
        * decade
}

/// Fills between the zero line at `zero` and the response `curve`, the boosts and the cuts as
/// two shapes that each fade from `fill` at the curve's furthest reach to clear at the line.
fn paint_area(curve: &[Point<Pixels>], zero: Pixels, color: Hsla, fill: f32, window: &mut Window) {
    for boost in [true, false] {
        let side = |y: Pixels| match boost {
            true => y.min(zero),
            false => y.max(zero),
        };
        if curve.iter().all(|sample| side(sample.y) == zero) {
            continue;
        }
        let sides: Vec<_> = curve
            .iter()
            .map(|sample| point(sample.x, side(sample.y)))
            .collect();
        let angle = match boost {
            true => 180.,
            false => 0.,
        };
        if let Some(path) = area_under(&sides, zero) {
            window.paint_path(
                path,
                linear_gradient(
                    angle,
                    linear_color_stop(color.opacity(fill), 0.),
                    linear_color_stop(color.opacity(0.), 1.),
                ),
            );
        }
    }
}

/// Paints one node as a ring around a clear centre, or filled when it is selected, with a soft
/// halo when it is selected or `lit` under the pointer. A node whose gain is fixed carries a
/// level bar across it.
fn paint_node(
    node: &ResponseNode,
    centre: Point<Pixels>,
    radius: Pixels,
    lit: bool,
    theme: &Theme,
    window: &mut Window,
) {
    let accent = match node.enabled {
        true => theme.primary,
        false => theme.muted_foreground,
    };
    let active = node.selected || lit;
    if active {
        let halo = radius * HALO;
        let glow = match node.selected {
            true => 0.22,
            false => 0.14,
        };
        window.paint_quad(
            gpui::fill(
                Bounds::new(centre - point(halo, halo), size(halo * 2., halo * 2.)),
                accent.opacity(glow),
            )
            .corner_radii(halo),
        );
    }
    let radius = match active {
        true => radius,
        false => radius * NODE_REST,
    };
    let (fill, ring) = match node.selected {
        true => (accent, theme.background),
        false => (theme.background, accent),
    };
    window.paint_quad(gpui::quad(
        Bounds::new(
            centre - point(radius, radius),
            size(radius * 2., radius * 2.),
        ),
        radius,
        fill,
        RING,
        ring,
        gpui::BorderStyle::Solid,
    ));
    if !node.gain_adjustable {
        line(
            &[
                point(centre.x - radius / 2., centre.y),
                point(centre.x + radius / 2., centre.y),
            ],
            RING,
            ring,
            window,
        );
    }
}

/// Fills the spectrum from the bottom of the plot up to its levels, dim so it stays behind the
/// response. It carries no scale of its own, since it only hints at what the curve shapes.
fn paint_spectrum(
    bounds: Bounds<Pixels>,
    axes: ResponseAxes,
    spectrum: &Spectrum,
    theme: &Theme,
    window: &mut Window,
) {
    let scale = ResponseAxes {
        gain: spectrum.scale,
        ..axes
    };
    let points: Vec<_> = spectrum
        .points
        .iter()
        .filter(|value| value.frequency_hz.is_finite() && value.gain_db.is_finite())
        .map(|value| scale.project(*value, bounds))
        .collect();
    if let Some(path) = area_under(&points, bounds.bottom()) {
        window.paint_path(
            path,
            linear_gradient(
                180.,
                linear_color_stop(theme.muted_foreground.opacity(0.12), 0.),
                linear_color_stop(theme.muted_foreground.opacity(0.03), 1.),
            ),
        );
    }
    if let Some(path) = band(&points, HAIRLINE) {
        window.paint_path(path, theme.muted_foreground.opacity(0.2));
    }
}

/// The selected node's frequency and gain in a popover-coloured tag above it, or below it when
/// the plot has no room above, led by a dot in the node's colour.
fn readout(
    node: &ResponseNode,
    centre: Point<Pixels>,
    bounds: Bounds<Pixels>,
    radius: Pixels,
    theme: &Theme,
    window: &mut Window,
    cx: &mut App,
) {
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
    let label = t!(
        "response-graph-readout",
        frequency = frequency.to_string(),
        gain = gain.abs(),
        direction = direction
    );
    let size_text = theme.text(Text::Tiny);
    let shaped = shape(label, theme.popover_foreground, window, cx);
    let pad = theme.metrics.pad;
    let dot = size_text / 2.;
    let tag = size(
        pad + dot + pad * 0.75 + shaped.width + pad,
        size_text + pad * 1.25,
    );
    let gap = radius * HALO + pad / 2.;
    let above = centre.y - gap - tag.height;
    let top = match above >= bounds.top() {
        true => above,
        false => centre.y + gap,
    };
    let left = (centre.x - tag.width / 2.)
        .min(bounds.right() - tag.width)
        .max(bounds.left());
    let tag = Bounds::new(point(left, top), tag);
    window.paint_quad(gpui::quad(
        tag,
        theme.radius.min(tag.size.height / 2.),
        theme.popover,
        HAIRLINE,
        theme.border,
        gpui::BorderStyle::Solid,
    ));
    let accent = match node.enabled {
        true => theme.primary,
        false => theme.muted_foreground,
    };
    window.paint_quad(
        gpui::fill(
            Bounds::new(
                point(left + pad, top + (tag.size.height - dot) / 2.),
                size(dot, dot),
            ),
            accent,
        )
        .corner_radii(dot / 2.),
    );
    let _ = shaped.paint(
        point(
            left + pad + dot + pad * 0.75,
            top + (tag.size.height - size_text) / 2.,
        ),
        size_text,
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// Strokes the polyline through `points`, which needs two of them at least.
pub(crate) fn line(points: &[Point<Pixels>], width: Pixels, color: Hsla, window: &mut Window) {
    if points.len() < 2 {
        return;
    }
    let mut path = PathBuilder::stroke(width);
    path.add_polygon(points, false);
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// Paints `label` in the tiny text size with its top at `position.y`, and `anchor` of its width
/// left of `position.x`: 0 starts it there, 0.5 centres it and 1 ends it there. A `plate` fill
/// and border colour put a rounded tag behind it first.
fn text(
    label: SharedString,
    position: Point<Pixels>,
    anchor: f32,
    color: Hsla,
    plate: Option<(Hsla, Hsla)>,
    window: &mut Window,
    cx: &mut App,
) {
    let theme = *cx.theme();
    let line = shape(label, color, window, cx);
    let origin = point(position.x - line.width() * anchor, position.y);
    if let Some((fill, border)) = plate {
        let pad = theme.metrics.pad / 2.;
        let size_text = theme.text(Text::Tiny);
        let plate_bounds = Bounds::new(
            origin - point(pad * 1.5, pad / 2.),
            size(line.width() + pad * 3., size_text + pad),
        );
        window.paint_quad(gpui::quad(
            plate_bounds,
            theme.radius.min(size_text),
            fill,
            HAIRLINE,
            border,
            gpui::BorderStyle::Solid,
        ));
    }
    let _ = line.paint(
        origin,
        theme.text(Text::Tiny),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

/// `label` shaped in the tiny text size and `color`.
fn shape(label: SharedString, color: Hsla, window: &mut Window, cx: &App) -> ShapedLine {
    let run = TextRun {
        len: label.len(),
        font: window.text_style().font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let size = cx.theme().text(Text::Tiny);
    window.text_system().shape_line(label, size, &[run], None)
}

/// The area between `points`, which run left to right, and the level `floor` below them, as two
/// triangles a span. That is all such a shape needs, so it is built directly rather than
/// tessellated, which the spectrum would pay for every frame.
fn area_under(points: &[Point<Pixels>], floor: Pixels) -> Option<Path<Pixels>> {
    let first = points.first()?;
    let mut path = Path::new(point(first.x, floor));
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        path.push_triangle((point(a.x, floor), a, b), INSIDE);
        path.push_triangle((point(a.x, floor), b, point(b.x, floor)), INSIDE);
    }
    Some(path)
}

/// The polyline through `points` as a band `width` wide, two triangles a segment with no joins,
/// which a hairline has no use for. It is built directly for the same reason as `area_under`.
fn band(points: &[Point<Pixels>], width: Pixels) -> Option<Path<Pixels>> {
    let first = points.first()?;
    let mut path = Path::new(*first);
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let (dx, dy) = ((b.x - a.x) / px(1.), (b.y - a.y) / px(1.));
        let length = dx.hypot(dy);
        if length == 0. {
            continue;
        }
        let half = width / 2. / length;
        let normal = point(half * -dy, half * dx);
        path.push_triangle((a + normal, a - normal, b + normal), INSIDE);
        path.push_triangle((b + normal, a - normal, b - normal), INSIDE);
    }
    Some(path)
}
