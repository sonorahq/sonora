//! The arrow that answers a two-finger touchpad swipe across the page. `ui::swiped` reads the
//! gesture. This turns the page and draws how far the swipe has got.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Context, NavigationDirection, Pixels, Render, ScrollWheelEvent, SpringState, Window, div,
    point, px, svg,
};
use ui::{ActiveTheme as _, Springs, ease_out_cubic, mix, motion, paced};

const ICON: Pixels = px(16.);
/// How far the chevron leans towards the page it points to once the arrow is armed.
const LEAN: Pixels = px(2.);
/// How far past the turning point the arrow can still be dragged, as a share of the way there,
/// however far the fingers go.
const STRETCH: f32 = 0.3;
/// How small the arrow is as it first peeks in, against its full size.
const PEEK: f32 = 0.6;
/// How much bigger the arrow grows once letting go would turn the page. The spring that takes
/// it there overshoots, which is what makes it pop.
const ARMED: f32 = 0.12;
/// How much further the arrow grows as it flies off with the page it turned.
const LAUNCH_GROWTH: f32 = 0.35;
/// How long the arrow takes to fly on and fade once the page turns, before the chosen pace.
const LAUNCH: Duration = Duration::from_millis(260);
/// The longest step a spring takes in one frame, so a stalled frame does not fling it.
const STALL: Duration = Duration::from_millis(50);
/// How near its goal a spring has to be to stop asking for frames.
const REST: f32 = 0.002;

/// A round arrow that peeks in from the side of the page while a swipe pulls it back or forward.
/// It grows and fills in with a pop once letting go would turn the page, flies on and fades when
/// the page turns, and springs back out of sight when the swipe is let go short. It shows only
/// where there is a page to turn to.
pub(crate) struct SwipeCue {
    /// The side the arrow is on, which outlives the swipe while the arrow animates away.
    direction: NavigationDirection,
    /// How far the fingers have pulled, where 1 turns the page. `None` once they lift.
    pull: Option<f32>,
    /// How far in the arrow is drawn, on the same scale as `pull`.
    shown: SpringState,
    /// How armed the arrow looks, from 0 to 1 and a little past it on the way in.
    armed: SpringState,
    /// When the page turned, while the arrow flies off with it.
    launched: Option<Instant>,
    beat: Option<Instant>,
}

impl SwipeCue {
    pub fn new(_: &mut Context<Self>) -> Self {
        Self {
            direction: NavigationDirection::Back,
            pull: None,
            shown: SpringState::default(),
            armed: SpringState::default(),
            launched: None,
            beat: None,
        }
    }

    /// Follows a wheel event over the page: turns the page when a swipe completes, and redraws
    /// the arrow while one is under the fingers.
    pub fn follow(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(direction) = ui::swiped(event, cx) {
            match direction {
                NavigationDirection::Back => router::back(cx),
                NavigationDirection::Forward => router::forward(cx),
            }
            // The swipe is spent, so its momentum must not scroll the page it opened.
            window.stop_scroll_coast();
            self.direction = direction;
            self.pull = None;
            self.launched = Some(Instant::now());
            cx.notify();
            return;
        }

        let navigation = router::trail(cx);
        let navigation = navigation.read(cx);
        let pull = ui::swipe_pull(cx).filter(|(direction, _)| match direction {
            NavigationDirection::Back => navigation.can_go_back(),
            NavigationDirection::Forward => navigation.can_go_forward(),
        });
        let Some((direction, pull)) = pull else {
            if self.pull.take().is_some() {
                cx.notify();
            }
            return;
        };
        if direction != self.direction {
            self.shown = SpringState::default();
            self.armed = SpringState::default();
        }
        self.direction = direction;
        self.pull = Some(pull);
        self.launched = None;
        cx.notify();
    }

    /// Moves the arrow on by the time since the last frame, and says whether it is still moving.
    fn step(&mut self, cx: &Context<Self>) -> bool {
        let now = Instant::now();
        let elapsed = self
            .beat
            .replace(now)
            .map_or(Duration::ZERO, |beat| now.duration_since(beat).min(STALL))
            .as_secs_f32();
        let animates = motion::animates(cx);
        let armed = match (self.pull, self.launched) {
            (_, Some(_)) => 1.,
            (Some(pull), None) if pull >= 1. => 1.,
            _ => 0.,
        };

        // The arrow keeps up with the fingers exactly, and only springs once they let go.
        let shown = match self.pull {
            Some(pull) => {
                self.shown = SpringState {
                    position: stretched(pull),
                    velocity: 0.,
                };
                None
            }
            None if self.launched.is_none() => Some(0.),
            None => None,
        };
        let mut moving = false;
        if let Some(shown) = shown {
            match animates {
                true => {
                    self.shown = Springs::RESPONSIVE.step(self.shown, shown, elapsed);
                    moving |= !Springs::RESPONSIVE.is_settled(self.shown, shown, REST);
                }
                false => self.shown = SpringState::default(),
            }
        }
        match animates {
            true => {
                self.armed = Springs::SETTLE.step(self.armed, armed, elapsed);
                moving |= !Springs::SETTLE.is_settled(self.armed, armed, REST);
            }
            false => {
                self.armed = SpringState {
                    position: armed,
                    velocity: 0.,
                }
            }
        }
        if let Some(launched) = self.launched
            && (!animates || launched.elapsed() >= paced(LAUNCH))
        {
            self.launched = None;
            self.shown = SpringState::default();
            self.armed = SpringState::default();
        }
        moving || self.launched.is_some()
    }
}

impl Render for SwipeCue {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let moving = self.step(cx);
        match moving {
            true => window.request_animation_frame(),
            false => self.beat = None,
        }

        let reach = self.shown.position.max(0.);
        let launch = self.launched.map(|launched| {
            ease_out_cubic((launched.elapsed().as_secs_f32() / paced(LAUNCH).as_secs_f32()).min(1.))
        });
        let visible = reach > REST || launch.is_some();
        let theme = cx.theme();
        let size = theme.metrics.control;
        let direction = self.direction;
        let armed = self.armed.position;
        let launch = launch.unwrap_or(0.);
        // From just out of sight to a gap's width in from the edge, and on past it as it flies.
        let edge = -size + (size + theme.metrics.pad) * reach + size * launch;
        let scale = (PEEK + (1. - PEEK) * reach.min(1.))
            * (1. + ARMED * armed)
            * (1. + LAUNCH_GROWTH * launch);
        let lean = LEAN * armed.clamp(0., 1.) + LEAN * 2. * launch;

        div().when(visible, |this| {
            this.absolute()
                .top_0()
                .bottom_0()
                .map(|this| match direction {
                    NavigationDirection::Back => this.left(edge),
                    NavigationDirection::Forward => this.right(edge),
                })
                .flex()
                .items_center()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(size)
                        .rounded_full()
                        .border_1()
                        .border_color(mix(theme.border, theme.primary, armed))
                        .bg(mix(theme.secondary, theme.primary, armed))
                        .shadow_md()
                        .opacity(reach.min(1.) * (1. - launch))
                        .layer_scale(scale.max(REST))
                        .child(
                            div()
                                .layer_translate(point(
                                    match direction {
                                        NavigationDirection::Back => -lean,
                                        NavigationDirection::Forward => lean,
                                    },
                                    Pixels::ZERO,
                                ))
                                .child(
                                    svg()
                                        .path(icons::path(match direction {
                                            NavigationDirection::Back => "icons/chevron-left.svg",
                                            NavigationDirection::Forward => {
                                                "icons/chevron-right.svg"
                                            }
                                        }))
                                        .size(ICON)
                                        .text_color(mix(
                                            theme.foreground,
                                            theme.primary_foreground,
                                            armed,
                                        )),
                                ),
                        ),
                )
        })
    }
}

/// Where the arrow is drawn for a pull, giving way past the turning point like a rubber band
/// so it never runs off across the page.
fn stretched(pull: f32) -> f32 {
    match pull <= 1. {
        true => pull,
        false => {
            let over = pull - 1.;
            1. + STRETCH * over / (1. + over)
        }
    }
}
