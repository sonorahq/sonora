use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, App, ElementId, Pixels, SpringState, StyleRefinement, Svg,
    Transformation, Window, percentage, px, svg,
};

use crate::motion::{self, Springs};
use crate::theme::ActiveTheme as _;

/// How long one turn takes.
const TURN: Duration = Duration::from_millis(900);
/// The spinner's size, the same as a button's icon.
const SIZE: Pixels = px(16.);
/// How close to rest, in turns, a settling icon has to come before it counts as still.
const REST: f32 = 0.002;
/// How far short of the whole turn, in turns, a stopping icon hands over to its spring.
const LEAD: f32 = 0.15;
/// How long one turn of a `Spin` takes, quicker than the spinner so a stopping icon reaches
/// its rest without a long run-on.
const SPIN: Duration = Duration::from_millis(600);

/// A turning arrow for work that has started and not answered yet. It holds still when the
/// system asks for less motion.
#[derive(IntoElement)]
pub struct Spinner {
    id: ElementId,
    base: Svg,
}

impl Spinner {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            base: svg(),
        }
    }
}

impl Styled for Spinner {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl RenderOnce for Spinner {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { id, mut base } = self;
        let overrides = std::mem::take(base.style());

        let mut icon = base
            .path(icons::path("icons/loader-circle.svg"))
            .size(SIZE)
            .flex_none()
            .text_color(cx.theme().muted_foreground);
        icon.style().refine(&overrides);
        match motion::animates(cx) {
            true => icon
                .with_animation(id, Animation::new(TURN).repeat(), |icon, turned| {
                    icon.with_transformation(Transformation::rotate(percentage(turned)))
                })
                .into_any_element(),
            false => icon.into_any_element(),
        }
    }
}

/// The angle of an icon that turns while work runs and springs to rest when the work ends. The
/// owner calls `start` and `stop` as the work comes and goes and reads `turn` on every render.
#[derive(Default)]
pub struct Spin {
    phase: Phase,
}

#[derive(Default)]
enum Phase {
    #[default]
    Still,
    /// Turning since `origin`, the moment it would have stood at rest.
    Turning { origin: Instant },
    /// Still turning from `origin` until `LEAD` short of the whole turn `to`, then springing the
    /// rest of the way, which takes `span`.
    Settling {
        origin: Instant,
        to: f32,
        span: Duration,
    },
}

impl Spin {
    /// Starts turning, or carries on from the angle a settling icon has reached.
    pub fn start(&mut self) {
        let turned = match self.phase {
            Phase::Turning { .. } => return,
            Phase::Still => 0.,
            Phase::Settling { .. } => self.angle().rem_euclid(1.),
        };
        self.phase = Phase::Turning {
            origin: Instant::now() - SPIN.mul_f32(turned),
        };
    }

    /// Lets a turning icon run on to the next whole turn at least `LEAD` ahead and spring into
    /// it, so it slows down, overshoots a little and swings back without a jolt.
    pub fn stop(&mut self) {
        let Phase::Turning { origin } = self.phase else {
            return;
        };
        self.phase = Phase::Settling {
            origin,
            to: (self.angle() + LEAD).ceil(),
            span: Springs::SETTLE.settle_time(landing(), 0., REST),
        };
    }

    /// The angle now in turns. It asks for another frame while the icon moves and stays at rest
    /// when the system asks for less motion.
    pub fn turn(&mut self, window: &Window, cx: &App) -> f32 {
        if !motion::animates(cx) {
            self.phase = Phase::Still;
        }
        if let Phase::Settling { origin, to, span } = self.phase
            && origin.elapsed() >= SPIN.mul_f32(to - LEAD) + span
        {
            self.phase = Phase::Still;
        }
        if !matches!(self.phase, Phase::Still) {
            window.request_animation_frame();
        }
        self.angle()
    }

    fn angle(&self) -> f32 {
        let turns = |origin: Instant| origin.elapsed().as_secs_f32() / SPIN.as_secs_f32();
        match self.phase {
            Phase::Still => 0.,
            Phase::Turning { origin } => turns(origin),
            Phase::Settling { origin, to, .. } => match turns(origin) - (to - LEAD) {
                coasting if coasting < 0. => turns(origin),
                landed => {
                    let since = landed * SPIN.as_secs_f32();
                    to + Springs::SETTLE.step(landing(), 0., since).position
                }
            },
        }
    }
}

/// Where the settling spring takes over: `LEAD` short of the target, at the turning speed.
fn landing() -> SpringState {
    SpringState {
        position: -LEAD,
        velocity: 1. / SPIN.as_secs_f32(),
    }
}
