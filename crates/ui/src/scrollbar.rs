use std::cell::Cell as Slot;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyWindowHandle, App, Context, DragMoveEvent, Empty, EntityId, ListState, MouseButton,
    MouseDownEvent, Pixels, Point, Render, ScrollHandle, SpringConfig, Task, WeakEntity, Window,
    anchored, deferred, div, img, point, px,
};

use crate::glide::Glide;
use crate::motion::{Glowing as _, Sprung, paced, spring};
use crate::theme::ActiveTheme as _;

const BAR: Pixels = px(6.);
const REACH: Pixels = px(2.);
const THUMB_INSET: Pixels = px(4.);
const MIN_THUMB: Pixels = px(24.);
const TRACK_INSET: Pixels = px(4.);
const LINGER: Duration = Duration::from_secs(2);
/// How long the spring takes to fade the bar in as the region scrolls and out once it settles,
/// before the listener's pace stretches it.
const FADE: Duration = Duration::from_millis(250);
/// How close the fade may come to shown or hidden and still count as landed.
const FADE_REST: f32 = 0.005;
const RESTING: f32 = 0.35;
const ACTIVE: f32 = 0.55;
const MIDDLE_SCROLL_DEADZONE: Pixels = px(8.);
const MIDDLE_SCROLL_SPEED: f32 = 16.;
const MIDDLE_SCROLL_MAX_SPEED: f32 = 200000.;
/// The glyph drawn where a middle-button auto-scroll was pressed, as browsers do.
const MIDDLE_SCROLL_MARKER: &str = "icons/scroll.svg";

type HoverGuard = Rc<dyn Fn(bool, AnyWindowHandle, &mut App)>;
type ScrollGuard = Rc<dyn Fn(Pixels, &mut App) -> Option<Pixels>>;

#[derive(Default)]
struct ActiveMiddleScroll(Option<WeakEntity<Scrollbar>>);

impl gpui::Global for ActiveMiddleScroll {}

#[derive(Clone)]
enum Target {
    Area(ScrollHandle),
    List(ListState),
}

impl Target {
    fn top(&self) -> Pixels {
        match self {
            Self::Area(scroll) => scroll.bounds().origin.y,
            Self::List(scroll) => scroll.viewport_bounds().origin.y,
        }
    }

    fn viewport(&self) -> Pixels {
        match self {
            Self::Area(scroll) => scroll.bounds().size.height,
            Self::List(scroll) => scroll.viewport_bounds().size.height,
        }
    }

    fn hidden(&self) -> Pixels {
        match self {
            Self::Area(scroll) => scroll.max_offset().y,
            Self::List(scroll) => scroll.max_offset_for_scrollbar().y,
        }
    }

    fn offset(&self) -> Pixels {
        match self {
            Self::Area(scroll) => scrolled(scroll),
            Self::List(scroll) => (-scroll.scroll_px_offset_for_scrollbar().y)
                .clamp(Pixels::ZERO, scroll.max_offset_for_scrollbar().y),
        }
    }

    fn set_offset(&self, offset: Pixels) {
        let point = point(Pixels::ZERO, -offset);
        match self {
            Self::Area(scroll) => scroll.set_offset(point),
            Self::List(scroll) => scroll.set_offset_from_scrollbar(point),
        }
    }

    fn drag_started(&self) {
        if let Self::List(scroll) = self {
            scroll.scrollbar_drag_started();
        }
    }

    fn drag_ended(&self) {
        if let Self::List(scroll) = self {
            scroll.scrollbar_drag_ended();
        }
    }
}

#[derive(Clone)]
struct Grab {
    owner: EntityId,
    start: Slot<Pixels>,
    offset: Slot<Pixels>,
}

/// One middle-button auto-scroll: `origin` is where the button went down and
/// `current` where the pointer is now. `dragged` remembers that the pointer once
/// left the deadzone while the button was held, which is what makes the release
/// end the mode.
#[derive(Clone, Copy)]
struct MiddleScroll {
    origin: Point<Pixels>,
    current: Pixels,
    last: Instant,
    armed: bool,
    dragged: bool,
}

impl Render for Grab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

pub fn scrolled(scroll: &ScrollHandle) -> Pixels {
    (-scroll.offset().y).clamp(Pixels::ZERO, scroll.max_offset().y)
}

pub struct Scrollbar {
    scroll: ScrollHandle,
    list: Option<ListState>,
    seen: Pixels,
    /// Whether the bar shows, from the moment the region scrolls until it has rested a while.
    awake: bool,
    /// Whether the pointer is over the strip along the region's edge. With [`Self::on_thumb`]
    /// it keeps a bar that shows from fading out, but never brings up a hidden one.
    hovered: bool,
    /// Whether the pointer is over the thumb, which stands in from that strip and so hears the
    /// pointer apart from it.
    on_thumb: bool,
    always_visible: bool,
    /// Whether the bar only shows where the region is while it scrolls and takes no pointer.
    passive: bool,
    track_inset: Pixels,
    /// How far the thumb stands in from the region's side edge.
    edge_inset: Pixels,
    track_top: Pixels,
    track_bottom: Pixels,
    maximum: Option<Pixels>,
    hover_guard: Option<HoverGuard>,
    scroll_guard: Option<ScrollGuard>,
    linger: Option<Task<()>>,
    /// How far the bar has faded in, springing towards shown while it is awake.
    fade: Sprung,
    glide: Glide,
    following: bool,
    nudges: u64,
    middle_scroll: Option<MiddleScroll>,
    /// The part of the offset `quantize` rounded off, which the next frame adds back so motion
    /// smaller than a device pixel per frame still adds up.
    remainder: Slot<Pixels>,
}

impl Scrollbar {
    pub fn new(scroll: ScrollHandle) -> Self {
        Self {
            scroll,
            list: None,
            seen: Pixels::ZERO,
            awake: false,
            hovered: false,
            on_thumb: false,
            always_visible: false,
            passive: false,
            track_inset: Pixels::ZERO,
            edge_inset: THUMB_INSET,
            track_top: Pixels::ZERO,
            track_bottom: Pixels::ZERO,
            maximum: None,
            hover_guard: None,
            scroll_guard: None,
            linger: None,
            fade: Sprung::at(0.),
            glide: Glide::default(),
            following: false,
            nudges: 0,
            middle_scroll: None,
            remainder: Slot::new(Pixels::ZERO),
        }
    }

    pub fn inset() -> Self {
        Self::new(ScrollHandle::new())
            .always_visible()
            .track_inset(TRACK_INSET)
    }

    pub fn list(list: ListState) -> Self {
        let mut scrollbar = Self::new(ScrollHandle::new());
        scrollbar.list = Some(list);
        scrollbar
    }

    pub fn always_visible(mut self) -> Self {
        self.always_visible = true;
        self
    }

    /// Makes the bar a mark of where the region is scrolled to, drawn only while it moves. It
    /// never appears under the pointer and cannot be dragged or clicked, so it claims no strip
    /// along the region's edge. The wheel and the middle button scroll the region as before.
    pub fn passive(mut self) -> Self {
        self.passive = true;
        self
    }

    /// Switches the bar between [`Self::passive`] and one that can be hovered, dragged and
    /// clicked, for a region that only sometimes has room for it.
    pub fn set_passive(&mut self, passive: bool, cx: &mut Context<Self>) {
        if self.passive != passive {
            self.passive = passive;
            self.hovered = false;
            self.on_thumb = false;
            self.pointed(cx);
            cx.notify();
        }
    }

    pub fn paced(mut self, pace: f32) -> Self {
        self.glide.set_pace(pace);
        self
    }

    pub fn spring(mut self, spring: SpringConfig) -> Self {
        self.glide.set_spring(spring);
        self
    }

    pub fn watching(mut self, view: EntityId) -> Self {
        self.glide.watch(view);
        self
    }

    pub fn track_inset(mut self, inset: Pixels) -> Self {
        self.track_inset = inset;
        self
    }

    /// Stands the thumb `inset` in from the region's side edge instead of the usual distance,
    /// for a region whose own padding already keeps it clear of the content.
    pub fn edge_inset(mut self, inset: Pixels) -> Self {
        self.edge_inset = inset;
        self
    }

    /// Pins the track's top below a floating header, so the bar never slides under
    /// a blur. Unlike the inset this only moves the top, never the bottom.
    pub fn track_top(mut self, top: Pixels) -> Self {
        self.track_top = top.max(Pixels::ZERO);
        self
    }

    /// The [`Self::track_top`] counterpart for a bar that already exists. Notifies only
    /// when the offset actually moved.
    pub fn set_track_top(&mut self, top: Pixels, cx: &mut Context<Self>) -> bool {
        let top = top.max(Pixels::ZERO);
        if self.track_top == top {
            return false;
        }
        self.track_top = top;
        cx.notify();
        true
    }

    pub fn remember_offset(&mut self, offset: Pixels) {
        self.seen = offset;
    }

    /// Keeps the entire track above an overlaid bottom control without reducing the region's
    /// scroll viewport. Notifies only when the clearance changes.
    pub fn set_track_bottom(&mut self, bottom: Pixels, cx: &mut Context<Self>) -> bool {
        let bottom = bottom.max(Pixels::ZERO);
        if self.track_bottom == bottom {
            return false;
        }
        self.track_bottom = bottom;
        cx.notify();
        true
    }

    pub fn nudges(&self) -> u64 {
        self.nudges
    }

    /// Whether the pointer is parked on the bar while it shows. Fullscreen reads this to keep
    /// its chrome awake while the reader holds the scrollbar.
    pub fn hovered(&self) -> bool {
        (self.hovered || self.on_thumb) && self.showing()
    }

    /// Records that the reader moved the view themselves. A precise scroll needs
    /// no smoothing, but it is still theirs, and anything following the view has
    /// to know to stop.
    pub fn stirred(&mut self) {
        if let Some(list) = &self.list {
            self.glide.jump(list, list.scroll_px_offset_for_scrollbar());
            self.following = false;
        } else if self.glide.stop_spring(&self.scroll) {
            self.following = false;
        }
        self.nudges = self.nudges.wrapping_add(1);
    }

    pub fn nudge(&mut self, window: &mut Window) {
        self.following = false;
        self.nudges = self.nudges.wrapping_add(1);
        match &self.list {
            Some(list) => self.glide.nudge(list, window),
            None => self.glide.nudge(&self.scroll, window),
        }
    }

    pub fn aim(&mut self, to: Pixels, window: &mut Window) {
        let across = self.scroll.offset().x;
        match &self.list {
            Some(list) => self.glide.aim(list, point(Pixels::ZERO, to), window),
            None => self.glide.aim(&self.scroll, point(across, to), window),
        }
        self.following = true;
    }

    pub fn place(&mut self, at: Pixels) {
        let across = self.scroll.offset().x;
        match &self.list {
            Some(list) => self.glide.jump(list, point(Pixels::ZERO, at)),
            None => self.glide.jump(&self.scroll, point(across, at)),
        }
        self.seen = match &self.list {
            Some(list) => list.scroll_px_offset_for_scrollbar().y,
            None => self.scroll.offset().y,
        };
        self.following = true;
    }

    pub fn goal(&self) -> Pixels {
        match &self.list {
            Some(list) => self.glide.goal(list).y,
            None => self.glide.goal(&self.scroll).y,
        }
    }

    pub fn presentation(&self) -> gpui::Point<Pixels> {
        match &self.list {
            Some(list) => self.glide.presentation(list),
            None => self.glide.presentation(&self.scroll),
        }
    }

    pub fn sync(&self) {
        match &self.list {
            Some(list) => self.glide.sync(list),
            None => self.glide.sync(&self.scroll),
        }
    }

    /// Rounds the region's offset to whole device pixels, so a header pinned against it holds
    /// still. What the rounding takes off is added back on the next call, otherwise a touchpad
    /// moving less than half a pixel per frame would stall and a fling would stop short of the top.
    /// Both stay inside the scroll range, since gpui clamps a rounded offset past the far end
    /// back to its fractional end and the remainder has to follow what is actually applied.
    pub fn quantize(&self, window: &Window) {
        let offset = self.scroll.offset();
        let max = self.scroll.max_offset().y;
        // Before the first layout there is no range yet, so only the top is a bound.
        let bounded = |y: Pixels| match max > Pixels::ZERO {
            true => y.clamp(-max, Pixels::ZERO),
            false => y.min(Pixels::ZERO),
        };
        let wanted = bounded(offset.y + self.remainder.get());
        let placed = bounded(crate::metrics::snapped(wanted, window));
        self.remainder.set(wanted - placed);
        if placed != offset.y {
            self.scroll.set_offset(point(offset.x, placed));
        }
    }

    /// How far down the region is scrolled, whichever kind of scrolling it does.
    pub fn offset(&self) -> Pixels {
        self.target().offset()
    }

    /// The height of the part on screen.
    pub fn viewport(&self) -> Pixels {
        self.target().viewport()
    }

    pub fn scroll(&self) -> &ScrollHandle {
        &self.scroll
    }

    /// Starts middle-button auto-scrolling at the pointer's current vertical position.
    pub fn middle_scroll_start(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> bool {
        let target = self.target();
        if target.hidden() <= Pixels::ZERO {
            return false;
        }
        self.stirred();
        self.middle_scroll = Some(MiddleScroll {
            origin: position,
            current: position.y,
            last: Instant::now(),
            armed: false,
            dragged: false,
        });
        self.schedule_middle_scroll(window, cx);
        true
    }

    /// Updates the pointer position while middle-button auto-scrolling is active.
    pub fn middle_scroll_move(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &Context<Self>,
    ) {
        let Some(middle) = self.middle_scroll.as_mut() else {
            return;
        };
        middle.current = position.y;
        if (middle.current - middle.origin.y).abs() > MIDDLE_SCROLL_DEADZONE {
            middle.dragged = true;
        }
        self.schedule_middle_scroll(window, cx);
    }

    pub fn middle_scroll_cancel(&mut self) {
        self.middle_scroll = None;
    }

    /// Answers a middle-button release at `position` the way a browser does. A press
    /// that was held and dragged ends with the release, while a plain click leaves the
    /// mode on until the next press. The release position counts as a drag too, since
    /// a pointer that left the window sends no moves. Returns whether the mode ended.
    pub fn middle_scroll_release(&mut self, position: Point<Pixels>) -> bool {
        let Some(middle) = self.middle_scroll else {
            return false;
        };
        let dragged =
            middle.dragged || (position.y - middle.origin.y).abs() > MIDDLE_SCROLL_DEADZONE;
        if !dragged {
            return false;
        }
        self.middle_scroll = None;
        true
    }

    fn schedule_middle_scroll(&mut self, window: &mut Window, cx: &Context<Self>) {
        let Some(middle) = self.middle_scroll.as_mut() else {
            return;
        };
        if middle.armed {
            return;
        }
        middle.armed = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.middle_scroll_frame(window, cx);
        });
    }

    fn middle_scroll_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut middle) = self.middle_scroll else {
            return;
        };
        middle.armed = false;

        let now = Instant::now();
        let elapsed = now.duration_since(middle.last).as_secs_f32().clamp(0., 0.1);
        middle.last = now;

        let distance = middle.current - middle.origin.y;
        let direction = distance.signum();
        let distance = distance.abs() - MIDDLE_SCROLL_DEADZONE;
        if distance <= Pixels::ZERO || elapsed <= 0. {
            self.middle_scroll = Some(middle);
            return;
        }

        let speed = (distance.as_f32() * MIDDLE_SCROLL_SPEED * 2.).min(MIDDLE_SCROLL_MAX_SPEED);
        let target = self.target();
        let hidden = self.maximum.unwrap_or_else(|| target.hidden());
        let offset =
            (target.offset() + px(direction * speed * elapsed)).clamp(Pixels::ZERO, hidden);
        if offset != target.offset() {
            target.set_offset(offset);
            self.moved(offset, cx);
            self.middle_scroll = Some(middle);
            self.schedule_middle_scroll(window, cx);
        } else {
            self.middle_scroll = Some(middle);
        }
    }

    pub fn on_scroll(
        mut self,
        guard: impl Fn(Pixels, &mut App) -> Option<Pixels> + 'static,
    ) -> Self {
        self.scroll_guard = Some(Rc::new(guard));
        self
    }

    pub fn set_max_offset(&mut self, maximum: Option<Pixels>, cx: &mut Context<Self>) -> bool {
        let maximum = maximum.map(|maximum| maximum.max(Pixels::ZERO));
        if self.maximum == maximum {
            return false;
        }
        self.maximum = maximum;
        cx.notify();
        true
    }

    fn target(&self) -> Target {
        self.list.as_ref().map_or_else(
            || Target::Area(self.scroll.clone()),
            |list| Target::List(list.clone()),
        )
    }

    pub(crate) fn set_hover_guard(
        &mut self,
        guard: impl Fn(bool, AnyWindowHandle, &mut App) + 'static,
    ) {
        self.hover_guard = Some(Rc::new(guard));
    }

    /// Shows the bar and lets it fade out once it has rested for `LINGER`. A bar under the
    /// pointer stays, and the pointer leaving it starts the rest over.
    fn wake(&mut self, cx: &mut Context<Self>) {
        self.awake = true;
        cx.notify();
        self.linger = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LINGER).await;
            this.update(cx, |this, cx| {
                if this.hovered() {
                    return;
                }
                this.awake = false;
                cx.notify();
            })
            .ok();
        }));
    }

    /// Answers the pointer coming onto the bar or leaving it. It holds a bar that shows, even one
    /// already fading out, and leaving starts its rest over, but it never brings up a hidden one.
    fn pointed(&mut self, cx: &mut Context<Self>) {
        if self.showing() {
            self.wake(cx);
        }
    }

    /// Whether the bar can be seen, counting one that is still fading out.
    fn showing(&self) -> bool {
        self.always_visible || self.awake || self.fade.position() > 0.
    }

    fn moved(&mut self, offset: Pixels, cx: &mut Context<Self>) {
        if let Some(list) = &self.list {
            self.glide.jump(list, list.scroll_px_offset_for_scrollbar());
            self.following = false;
        } else if self.glide.stop_spring(&self.scroll) {
            self.following = false;
        }
        self.nudges = self.nudges.wrapping_add(1);
        if let Some(maximum) = self
            .scroll_guard
            .as_ref()
            .and_then(|guard| guard(offset, cx))
        {
            self.maximum = Some(maximum.max(Pixels::ZERO));
        }
        self.wake(cx);
    }
}

/// Makes `scrollbar` the only active middle-button auto-scroll target.
pub fn activate_middle_scroll(scrollbar: &gpui::Entity<Scrollbar>, cx: &mut App) {
    let previous = cx.default_global::<ActiveMiddleScroll>().0.take();
    if let Some(previous) = previous.filter(|previous| previous != scrollbar) {
        previous
            .update(cx, |scrollbar, _| scrollbar.middle_scroll_cancel())
            .ok();
    }
    cx.default_global::<ActiveMiddleScroll>().0 = Some(scrollbar.downgrade());
}

/// Stops the active middle-button auto-scroll, if any.
pub fn cancel_middle_scroll(cx: &mut App) -> bool {
    let Some(active) = cx.default_global::<ActiveMiddleScroll>().0.take() else {
        return false;
    };
    active
        .update(cx, |scrollbar, _| scrollbar.middle_scroll_cancel())
        .ok();
    true
}

/// Hands a middle-button release at `position` to the active auto-scroll, if any.
/// Returns whether the release ended it, which only a held-and-dragged press does.
pub fn release_middle_scroll(position: Point<Pixels>, cx: &mut App) -> bool {
    let Some(active) = cx.default_global::<ActiveMiddleScroll>().0.clone() else {
        return false;
    };
    let ended = active
        .update(cx, |scrollbar, _| scrollbar.middle_scroll_release(position))
        .unwrap_or(true);
    if ended {
        cx.default_global::<ActiveMiddleScroll>().0 = None;
    }
    ended
}

/// Updates the active middle-button auto-scroll target from a window-level mouse move.
pub fn update_middle_scroll(position: Point<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(active) = cx.default_global::<ActiveMiddleScroll>().0.clone() else {
        return;
    };
    active
        .update(cx, |scrollbar, cx| {
            scrollbar.middle_scroll_move(position, window, cx);
        })
        .ok();
}

impl Render for Scrollbar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.target();
        let viewport = target.viewport();
        let hidden = self.maximum.unwrap_or_else(|| target.hidden());
        let offset = target.offset().min(hidden);

        if offset != self.seen {
            self.seen = offset;
            if !self.following {
                self.wake(cx);
            }
        }
        // A followed glide lets go only once it has landed and its landing has been seen above,
        // so neither the creep toward the goal nor the final step reads as the reader scrolling.
        if self.following && !self.glide.gliding() {
            self.following = false;
        }

        if viewport <= Pixels::ZERO || hidden <= Pixels::ZERO {
            return div().into_any_element();
        }

        let theme = *cx.theme();
        let content = viewport + hidden;
        let progress = (offset / hidden).clamp(0., 1.);
        let track = (viewport - self.track_inset * 2. - self.track_top - self.track_bottom)
            .max(Pixels::ZERO);
        let thumb = (track * (viewport / content)).max(MIN_THUMB).min(track);
        let travel = track - thumb;
        let shown = match self.always_visible {
            true => 1.,
            false => {
                let target = f32::from(u8::from(self.awake));
                let config = spring(paced(FADE).as_secs_f32(), 0.);
                self.fade.follow(target, config, FADE_REST, window, cx)
            }
        };
        let resting = RESTING * shown;

        let jump = target.clone();
        let drag = target.clone();
        let started = target.clone();
        let owner = cx.entity_id();
        let bar = cx.weak_entity();
        let hover_guard = self.hover_guard.clone();
        let anchor = self.middle_scroll.map(|middle| middle.origin);
        let marker = theme.metrics.control;
        let mark = anchor.map(|anchor| {
            deferred(
                anchored()
                    .position(anchor)
                    .offset(point(-marker / 2., -marker / 2.))
                    .child(
                        div()
                            .size(marker)
                            .child(img(icons::path(MIDDLE_SCROLL_MARKER)).size_full()),
                    ),
            )
        });

        if self.passive {
            return div()
                .absolute()
                .top(self.track_inset + self.track_top)
                .right(self.edge_inset)
                .w(BAR)
                .h(track)
                .child(
                    div()
                        .absolute()
                        .top(travel * progress)
                        .w(BAR)
                        .h(thumb)
                        .rounded_full()
                        .bg(theme.muted_foreground.opacity(resting)),
                )
                .children(mark)
                .into_any_element();
        }

        div()
            // The entity id keeps every bar distinct, since ids alone would
            // collide the moment two bars share a parent.
            .id(("scrollbar", cx.entity_id()))
            .role(gpui::accesskit::Role::ScrollBar)
            .aria_orientation(gpui::accesskit::Orientation::Vertical)
            .aria_numeric_value(f64::from(progress))
            .aria_min_numeric_value(0.)
            .aria_max_numeric_value(1.)
            .occlude()
            .absolute()
            .top(self.track_inset + self.track_top)
            .right(-REACH)
            .w(BAR + REACH)
            .h(track)
            .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                this.hovered = *hovered;
                this.pointed(cx);
                if let Some(guard) = hover_guard.as_ref() {
                    guard(*hovered, window.window_handle(), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    let local = event.position.y
                        - jump.top()
                        - this.track_inset
                        - this.track_top
                        - thumb / 2.;
                    let fraction = (local / travel).clamp(0., 1.);
                    let offset = hidden * fraction;
                    this.stirred();
                    jump.set_offset(offset);
                    this.moved(offset, cx);
                }),
            )
            .child(
                div()
                    .id(("scrollbar-thumb", cx.entity_id()))
                    .absolute()
                    .top(travel * progress)
                    .right(self.edge_inset + REACH)
                    .w(BAR)
                    .h(thumb)
                    .rounded_full()
                    .bg(theme.muted_foreground.opacity(resting))
                    .when(shown > 0., |thumb| thumb.cursor_pointer())
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                            this.stirred();
                            started.drag_started();
                            this.wake(cx);
                            cx.stop_propagation();
                        }),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener({
                            let released = target.clone();
                            move |_, _, _, _| released.drag_ended()
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(move |_, _, _, _| target.drag_ended()),
                    )
                    .on_drag(
                        Grab {
                            owner,
                            start: Slot::new(Pixels::ZERO),
                            offset: Slot::new(offset),
                        },
                        |grab, _, window, cx| {
                            grab.start.set(window.mouse_position().y);
                            cx.new(|_| grab.clone())
                        },
                    )
                    .on_drag_move(
                        cx.listener(move |this, event: &DragMoveEvent<Grab>, _, cx| {
                            let (start, base) = {
                                let grab = event.drag(cx);
                                if grab.owner != owner {
                                    return;
                                }
                                (grab.start.get(), grab.offset.get())
                            };
                            let moved = event.event.position.y - start;
                            let scrolled = base + moved * (hidden / travel);
                            let clamped = scrolled.clamp(Pixels::ZERO, hidden);
                            drag.set_offset(clamped);
                            this.moved(clamped, cx);
                        }),
                    )
                    .glowed(theme.muted_foreground.opacity(ACTIVE * shown))
                    .on_hover(move |hovered, _, cx| {
                        bar.update(cx, |this, cx| {
                            this.on_thumb = *hovered;
                            this.pointed(cx);
                        })
                        .ok();
                    }),
            )
            .children(mark)
            .into_any_element()
    }
}
