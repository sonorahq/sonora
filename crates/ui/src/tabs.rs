use std::cell::Cell;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Background, Div, ElementId, Hsla, Pixels, ScrollHandle, ScrollWheelEvent,
    SharedString, Stateful, StyleRefinement, Window, div, ease_in_out, linear_color_stop,
    linear_gradient, px,
};

use crate::button::Button;
use crate::glass::{blurring, glass};
use crate::glide::Glide;
use crate::motion::{Motion, Motioned as _, animates};
use crate::theme::ActiveTheme as _;

const LINE: f32 = 1.;
const INDENT: f32 = 16.;
const ICON: f32 = 20.;
/// The bar's own padding in rem: it paints `.p_1()`, so the items sit this
/// far inside its curve.
const PAD: f32 = 0.25;
/// How wide the fade at an edge with more tabs behind it is.
const FADE: f32 = 32.;
/// How far the row has to be off an edge before that edge fades, so a row resting against one
/// never flickers a sliver of gradient.
const EDGE: Pixels = px(1.);
/// How a fade leaves. There is nothing to ease in: the tabs behind the edge are already there
/// the moment the row moves, so the gradient arrives with them and only its exit is animated.
const HUSH: Motion = Motion::Base;

#[derive(IntoElement)]
pub struct Tabs {
    base: Div,
    items: Vec<AnyElement>,
}

/// A row of segment buttons in one pill. The bar is as wide as its items until the caller
/// gives it a `max_w`, where it stops and scrolls the items sideways instead, so it takes
/// any number of tabs. Items keep their own width and never shrink to fit.
///
/// A capped bar scrolls the way the rest of the app does: a wheel anywhere over it, in either
/// axis, glides the row sideways and never reaches the page behind, and each edge with tabs
/// behind it fades them out rather than cutting them off. There is no scrollbar.
///
/// Colours come from the theme (`secondary`, `border`) and any caller style wins over them,
/// so a call site stays in charge of the tint. `.blurred()` marks a bar that floats over
/// content: it always casts a soft shadow, and swaps the solid fill for `glass` while
/// `blurring` is on.
#[derive(IntoElement)]
pub struct TabBar {
    base: Div,
    id: ElementId,
    name: Option<SharedString>,
    items: Vec<Button>,
    blurred: bool,
    reveal: Option<usize>,
}

impl TabBar {
    #[track_caller]
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div(),
            id: id.into(),
            name: None,
            items: Vec::new(),
            blurred: false,
            reveal: None,
        }
    }

    /// Names the bar for screen readers, which read it on entering the bar before the option
    /// in focus.
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn items(mut self, items: impl IntoIterator<Item = Button>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    pub fn blurred(mut self) -> Self {
        self.blurred = true;
        self
    }

    /// Scrolls a capped bar just far enough to show the item at `index` each time the index
    /// changes, so a selection made elsewhere never waits behind an edge. While the index holds,
    /// the row stays where the user left it.
    pub fn reveal(mut self, index: Option<usize>) -> Self {
        self.reveal = index;
        self
    }
}

impl Styled for TabBar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

/// What one edge of the bar draws this frame.
#[derive(Clone, Copy, PartialEq)]
enum Wash {
    /// The edge has tabs behind it.
    Full,
    /// It no longer has, and the fade is on its way out: how much of it is left, from one at
    /// the moment the edge emptied down to nothing once `HUSH` has passed.
    Leaving(f32),
    None,
}

/// One edge's fade across frames: whether it was drawn at full strength last frame, and when
/// it started leaving, so the gradient outlives the tabs it was hiding.
#[derive(Clone, Copy, Default)]
struct Edge {
    shown: bool,
    leaving: Option<Instant>,
}

/// The sideways scroll of a bar too wide for its room: the handle gpui moves, and the glide
/// that turns a wheel step into a slide. A bar with no `max_w` never builds one.
struct Rail {
    scroll: ScrollHandle,
    glide: Glide,
    /// How far the row had scrolled and could scroll when it was last drawn. A bar that only
    /// learned its width during that frame has to draw once more for its fades to be right,
    /// and a `notify` raised from inside a render schedules nothing.
    seen: Cell<(Pixels, Pixels)>,
    /// The leading and trailing edge, in that order.
    edges: Cell<[Edge; 2]>,
    /// The item the row was last asked to show.
    revealed: Cell<Option<usize>>,
}

impl Rail {
    fn new() -> Self {
        Self {
            scroll: ScrollHandle::new(),
            glide: Glide::default(),
            seen: Cell::new((Pixels::ZERO, Pixels::ZERO)),
            edges: Cell::new([Edge::default(); 2]),
            revealed: Cell::new(None),
        }
    }

    /// What each edge draws, given whether it has tabs behind it now. An edge that just emptied
    /// keeps its gradient for as long as `HUSH` takes, which is what the animation runs over;
    /// with motion off it simply goes.
    fn washes(&self, wanted: [bool; 2], animates: bool) -> [Wash; 2] {
        let span = HUSH.span();
        let mut edges = self.edges.get();
        let mut washes = [Wash::None; 2];

        for (side, want) in wanted.into_iter().enumerate() {
            let edge = &mut edges[side];
            if want {
                *edge = Edge {
                    shown: true,
                    leaving: None,
                };
                washes[side] = Wash::Full;
                continue;
            }
            if edge.shown {
                *edge = Edge {
                    shown: false,
                    leaving: animates.then(Instant::now),
                };
            } else if edge.leaving.is_some_and(|since| since.elapsed() >= span) {
                edge.leaving = None;
            }
            washes[side] = match edge.leaving {
                Some(since) => Wash::Leaving(remaining(since, span)),
                None => Wash::None,
            };
        }

        self.edges.set(edges);
        washes
    }

    /// How far the row is scrolled from the left and how far it could go, as the last layout
    /// measured it.
    fn reach(&self) -> (Pixels, Pixels) {
        let hidden = self.scroll.max_offset().x.max(Pixels::ZERO);
        (
            (-self.scroll.offset().x).clamp(Pixels::ZERO, hidden),
            hidden,
        )
    }
}

/// What a capped bar's render needs out of its `Rail`: the pieces the row scrolls with, and
/// what each edge draws.
#[derive(Clone)]
struct Rails {
    scroll: ScrollHandle,
    glide: Glide,
    washes: [Wash; 2],
}

impl RenderOnce for TabBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            mut base,
            id,
            name,
            items,
            blurred,
            reveal,
        } = self;
        let theme = *cx.theme();
        let glassy = blurred && blurring(cx);
        let rem = window.rem_size();
        let overrides = std::mem::take(base.style());
        // Caller styles win over the bar defaults below, so the rounding the
        // bar actually paints can differ from the theme radius: read it back
        // out of the overrides. `.rounded()` sets every corner at once, so
        // the top left stands in for them all.
        let rounding = overrides
            .corner_radii
            .top_left
            .map(|length| length.to_pixels(rem))
            .unwrap_or(theme.radius)
            .max(px(0.));
        // Concentric corners: the items sit one padding inside the bar.
        let radius = (rounding - rem * PAD).max(px(0.));
        let capped = overrides.max_size.width.is_some();
        let moving = animates(cx);
        let rails = capped.then(|| {
            let rail = window.use_keyed_state((id.clone(), "rail"), cx, |_, _| Rail::new());
            let rail = rail.read(cx);
            // gpui measures the item against the row's bounds from the last layout, so a row
            // that has not been laid out yet waits a frame before it scrolls
            if rail.revealed.get() != reveal {
                match rail.scroll.bounds().size.width > Pixels::ZERO {
                    true => {
                        rail.revealed.set(reveal);
                        if let Some(index) = reveal {
                            rail.scroll.scroll_to_item(index);
                        }
                    }
                    false => window.request_animation_frame(),
                }
            }
            rail.glide.sync(&rail.scroll);

            let (at, hidden) = rail.reach();
            if rail.seen.replace((at, hidden)) != (at, hidden) {
                window.request_animation_frame();
            }
            Rails {
                scroll: rail.scroll.clone(),
                glide: rail.glide.clone(),
                washes: rail.washes([at > EDGE, at < hidden - EDGE], moving),
            }
        });

        let row = div()
            .id(id)
            .role(gpui::accesskit::Role::TabList)
            .when_some(name, |row, name| row.aria_label(name))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .when_some(rails.clone(), |row, rails| {
                // No `restrict_scroll_to_axis`: a bar that scrolls in one axis only takes the
                // wheel whichever way it was turned, which is what a row of tabs wants.
                row.overflow_x_scroll()
                    .track_scroll(&rails.scroll)
                    .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                        // gpui has already moved the row by the time this runs, so a row with
                        // nowhere to go is the one case where the page behind still gets the
                        // wheel. Anywhere else the bar keeps it.
                        if rails.scroll.max_offset().x <= Pixels::ZERO {
                            return;
                        }
                        match event.delta.precise() {
                            true => rails.glide.sync(&rails.scroll),
                            false => rails.glide.nudge(&rails.scroll, window),
                        }
                        cx.stop_propagation();
                    })
            })
            .children({
                let count = items.len();
                items.into_iter().enumerate().map(move |(index, item)| {
                    item.tab()
                        .aria_position_in_set(index + 1)
                        .aria_size_of_set(count)
                        .flex_shrink_0()
                        .rounded(radius)
                })
            });
        // A glass bar has no flat fill to paint a fade with: a wash of its own thin fill hides
        // nothing. It fades the tabs themselves instead, and leaves the blur behind them whole.
        let row = match (glassy, rails.as_ref()) {
            (true, Some(rails)) => {
                // The mask reads its strength off the rail's clock rather than an animation
                // element, so nothing else asks for the frames it needs while an edge empties.
                if rails
                    .washes
                    .iter()
                    .any(|wash| matches!(wash, Wash::Leaving(_)))
                {
                    window.request_animation_frame();
                }
                masked(row, rails.washes).into_any_element()
            }
            _ => row.into_any_element(),
        };

        let mut bar = base
            .relative()
            .flex()
            .p_1()
            .rounded(theme.radius)
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.border)
            .when(blurred, |bar| bar.shadow_sm())
            .when(glassy, |bar| glass(bar, cx))
            .child(row)
            .when_some(rails.filter(|_| !glassy), |bar, rails| {
                bar.children(
                    rails
                        .washes
                        .into_iter()
                        .enumerate()
                        .filter_map(|(side, wash)| fade(theme.secondary, radius, side == 0, wash)),
                )
            });

        bar.style().refine(&overrides);
        bar
    }
}

/// The gradient over an edge that has tabs behind it: the bar's own fill melting into nothing,
/// so the row slides out of sight instead of being cut off. It carries no interactivity, so a
/// tab underneath still takes the click.
fn fade(fill: Hsla, radius: Pixels, leading: bool, wash: Wash) -> Option<AnyElement> {
    // gpui measures the angle from the top, clockwise, so the fill leads at the left edge at
    // 90 degrees and at the right edge at 270.
    let angle = match leading {
        true => 90.,
        false => 270.,
    };
    let strip = div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(FADE))
        .bg(melting(fill, angle, 1.));
    let strip = match leading {
        true => strip.left_0().rounded_l(radius),
        false => strip.right_0().rounded_r(radius),
    };

    match wash {
        Wash::None => None,
        Wash::Full => Some(strip.into_any_element()),
        Wash::Leaving(_) => Some(
            strip
                .motion(("tab-fade", usize::from(leading)), HUSH, move |strip, t| {
                    strip.bg(melting(fill, angle, 1. - t))
                })
                .into_any_element(),
        ),
    }
}

/// The row faded out towards each edge that has tabs behind it, for a glass bar. An edge on
/// its way out narrows its fade by what `Wash::Leaving` says is left, so the scrolling row is
/// never wrapped and keeps its place in the tree while an edge empties. With neither edge fading
/// the row goes back unmasked, since even an empty mask opens a layer and drops every backdrop
/// in it.
fn masked(row: Stateful<Div>, washes: [Wash; 2]) -> Stateful<Div> {
    let width = |wash| match wash {
        Wash::Full => px(FADE),
        Wash::Leaving(left) => px(FADE) * left,
        Wash::None => Pixels::ZERO,
    };
    match washes {
        [Wash::None, Wash::None] => row,
        [leading, trailing] => row.fade_sides(width(leading), width(trailing)),
    }
}

/// How much of a leaving edge's fade is left, eased the way `HUSH` eases, from one when the
/// edge emptied down to nothing once `span` has passed.
fn remaining(since: Instant, span: Duration) -> f32 {
    match span.is_zero() {
        true => 0.,
        false => 1. - ease_in_out((since.elapsed().as_secs_f32() / span.as_secs_f32()).min(1.)),
    }
}

/// The bar's fill at `strength`, melting into nothing across the strip.
fn melting(fill: Hsla, angle: f32, strength: f32) -> Background {
    linear_gradient(
        angle,
        linear_color_stop(fill.opacity(strength), 0.),
        linear_color_stop(fill.opacity(0.), 1.),
    )
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new()
    }
}

impl Tabs {
    pub fn new() -> Self {
        Self {
            base: div(),
            items: Vec::new(),
        }
    }

    pub fn items(mut self, items: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.items = items
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect();
        self
    }
}

impl Styled for Tabs {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Tabs {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { mut base, items } = self;
        let theme = cx.theme();
        let height = theme.metrics.control;
        let middle = height / 2.;
        let border = theme.sidebar_border;
        let overrides = std::mem::take(base.style());

        let mut tabs = base
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .ml(px(INDENT))
            .child(
                div()
                    .absolute()
                    .left(px(ICON - INDENT))
                    .top_0()
                    .bottom(middle)
                    .w(px(LINE))
                    .bg(border),
            )
            .children(items.into_iter().map(|item| {
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .h(height)
                    .pl_3()
                    .child(div().flex().flex_1().child(item))
            }));

        tabs.style().refine(&overrides);
        tabs
    }
}
