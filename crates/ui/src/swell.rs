use std::time::Duration;
use std::time::Instant;

use gpui::{App, Bounds, Context, Entity, Pixels, Point, Task};

use crate::drag::Edge;
use crate::scrollbar::Scrollbar;
use crate::theme::ActiveTheme as _;

/// How long a dragged pin has to hold an edge before the region starts scrolling.
const EDGE_DWELL: Duration = Duration::from_secs(1);
/// How often the scroll task steps once the dwell has passed.
const EDGE_HZ: u32 = 60;
const EDGE_FRAME: Duration = Duration::from_nanos(1_000_000_000 / EDGE_HZ as u64);
// row slices per frame once the dwell passes
const EDGE_STEP: f32 = 6.;
/// A drop target stops swelling when it has not received a drag update recently.
const EDGE_STALE: Duration = Duration::from_millis(250);

/// A dragged pin held at one end of a region, waiting out the dwell before it scrolls.
#[derive(Clone, Copy)]
struct EdgeHold {
    edge: Edge,
    since: Instant,
    seen: Instant,
}

/// The auto-scroll a view's region earns while a dragged pin is held against one of its
/// ends. The view feeds the pointer from a drag-move handler with [`EdgeSwell::hold`] and
/// implements [`Swells`], so the swell can step the region's scrollbar and tell when the
/// region stops being the one shown.
#[derive(Default)]
pub struct EdgeSwell {
    hold: Option<EdgeHold>,
    task: Option<Task<()>>,
}

/// What a view with an [`EdgeSwell`] provides: the bar of the region that swells and
/// whether that region is the one shown, so a drag outlives a switch away from it.
pub trait Swells: 'static {
    fn swell(&mut self) -> &mut EdgeSwell;
    fn swell_bar(&self) -> &Entity<Scrollbar>;
    fn swelling(&self, cx: &App) -> bool;
}

impl EdgeSwell {
    pub fn new() -> Self {
        Self {
            hold: None,
            task: None,
        }
    }

    /// Tracks the pointer against the top and bottom of `bounds`, restarting the dwell
    /// whenever it crosses to the other edge or back into the middle, and starting the
    /// scroll task while any edge is held. `bottom` lifts the lower band above chrome
    /// overlaying the region, such as the player bar, so the pointer can reach it.
    pub fn hold<V: Swells>(
        &mut self,
        bar: &Entity<Scrollbar>,
        bounds: Bounds<Pixels>,
        position: Point<Pixels>,
        bottom: Pixels,
        cx: &mut Context<V>,
    ) {
        let zone = cx.theme().metrics.list_row;
        let foot = bounds.origin.y + bounds.size.height - bottom;
        let edge = match position.y {
            y if y - bounds.origin.y < zone => Some(Edge::Above),
            y if foot - y < zone => Some(Edge::Below),
            _ => None,
        };
        if self.hold.map(|hold| hold.edge) != edge {
            self.hold = edge.map(|edge| {
                let now = Instant::now();
                EdgeHold {
                    edge,
                    since: now,
                    seen: now,
                }
            });
        } else if let Some(hold) = &mut self.hold {
            hold.seen = Instant::now();
        }
        if self.hold.is_some() {
            self.drive(bar.clone(), cx);
        }
    }

    /// Scrolls the held region once the dwell has passed, a slice of a row per frame,
    /// until the pin leaves the edges, the drag ends, or the view stops showing the region.
    fn drive<V: Swells>(&mut self, bar: Entity<Scrollbar>, cx: &mut Context<V>) {
        if self.task.is_some() {
            return;
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(EDGE_FRAME).await;
                let run = this
                    .update(cx, |view, cx| step(view, &bar, cx))
                    .unwrap_or(false);
                if !run {
                    break;
                }
            }
        }));
    }
}

/// One tick of the swell: scrolls the region once the dwell has passed and says whether to run again.
fn step<V: Swells>(view: &mut V, bar: &Entity<Scrollbar>, cx: &mut Context<V>) -> bool {
    let hold = view.swell().hold;
    let live = view.swelling(cx)
        && cx.has_active_drag()
        && hold.is_some_and(|hold| hold.seen.elapsed() < EDGE_STALE);
    let run = match (hold, live) {
        (Some(hold), true) => {
            if hold.since.elapsed() >= EDGE_DWELL {
                let slice = cx.theme().metrics.list_row / EDGE_STEP;
                let offset = match hold.edge {
                    Edge::Above => bar.read(cx).offset() - slice,
                    Edge::Below => bar.read(cx).offset() + slice,
                };
                bar.update(cx, |bar, cx| bar.drag_to(offset, cx));
            }
            true
        }
        _ => false,
    };
    if !run {
        view.swell().hold = None;
        view.swell().task = None;
    }
    run
}
