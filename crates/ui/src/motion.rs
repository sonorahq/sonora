//! Motion: the springs every element moves on, and the listener's pace and stillness choices.
//!
//! Rune animates layout, colour, opacity and presence itself, retargeting a spring mid-flight,
//! so a view only says which spring a change rides on. These are Sonora's.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use i18n::{SharedString, t};
use rune::Spring;

/// Shared springs, in Rune's duration-and-bounce terms.
pub enum Springs {}

impl Springs {
    /// Direct feedback: hovers, toggles, a control's colour.
    pub const CONTROL: Spring = Spring::new(0.18, 0.0);
    /// Small layout moves: a row opening, a badge appearing.
    pub const QUICK: Spring = Spring::new(0.24, 0.0);
    /// The default for layout and presence.
    pub const BASE: Spring = Spring::new(0.32, 0.0);
    /// Panels and pages sliding in and out.
    pub const SLOW: Spring = Spring::new(0.42, 0.0);
    /// The press-and-release of a button or card: shrinks under the pointer and springs back
    /// with a slight overshoot.
    pub const PRESS: Spring = Spring::new(0.3, 0.32);
    /// A soft spring that overshoots a little and swings back, for an icon settling after it spun.
    pub const SETTLE: Spring = Spring::new(0.45, 0.25);
    /// Theme colours fading from one palette to the next.
    pub const THEME: Spring = Spring::new(0.4, 0.0);
    /// The lyrics view following the sung line.
    pub const LYRICS_SCROLL: Spring = Spring::new(0.5, 0.0);
}

/// How far a pressed control shrinks.
pub const PRESS_SCALE: f32 = 0.96;
/// How far a pressed card or tile shrinks; larger things move less.
pub const PRESS_SCALE_CARD: f32 = 0.975;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Control,
    Quick,
    Base,
    Slow,
}

impl Motion {
    /// The spring for this step at the listener's pace.
    pub fn spring(self) -> Spring {
        let base = match self {
            Self::Control => return Springs::CONTROL,
            Self::Quick => Springs::QUICK,
            Self::Base => Springs::BASE,
            Self::Slow => Springs::SLOW,
        };
        Spring::new(base.duration * pace().scale(), base.bounce)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Pace {
    Slow,
    #[default]
    Base,
    Quick,
}

impl Pace {
    pub const ALL: [Self; 3] = [Self::Slow, Self::Base, Self::Quick];

    pub fn id(self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Base => "base",
            Self::Quick => "quick",
        }
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::Slow => t!("pace-slow"),
            Self::Base => t!("pace-base"),
            Self::Quick => t!("pace-quick"),
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "slow" => Self::Slow,
            "quick" => Self::Quick,
            _ => Self::Base,
        }
    }

    /// How much longer (or shorter) than the base pace motion runs.
    pub fn scale(self) -> f32 {
        match self {
            Self::Slow => 1.6,
            Self::Base => 1.,
            Self::Quick => 0.6,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Saver {
    #[default]
    Off,
    Light,
    Medium,
    Strong,
}

impl Saver {
    pub const ALL: [Self; 4] = [Self::Off, Self::Light, Self::Medium, Self::Strong];

    pub fn id(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Strong => "strong",
        }
    }

    pub fn label(self) -> SharedString {
        match self.rate() {
            Some(fps) => t!("saver-fps", fps = fps),
            None => t!("saver-off"),
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "light" => Self::Light,
            "medium" => Self::Medium,
            "strong" => Self::Strong,
            _ => Self::Off,
        }
    }

    pub fn interval(self) -> Option<Duration> {
        self.rate()
            .map(|rate| Duration::from_nanos(1_000_000_000 / rate as u64))
    }

    fn rate(self) -> Option<u32> {
        match self {
            Self::Off => None,
            Self::Light => Some(90),
            Self::Medium => Some(60),
            Self::Strong => Some(30),
        }
    }
}

/// Stretches or shortens `span` by the chosen pace, for motion that sets its own length.
pub fn paced(span: Duration) -> Duration {
    span.mul_f32(pace().scale())
}

/// The pace the listener chose, which every [`Motion`] spring follows.
pub fn pace() -> Pace {
    match PACE.load(Ordering::Relaxed) {
        0 => Pace::Slow,
        2 => Pace::Quick,
        _ => Pace::Base,
    }
}

pub fn set_pace(pace: Pace) {
    PACE.store(
        match pace {
            Pace::Slow => 0,
            Pace::Base => 1,
            Pace::Quick => 2,
        },
        Ordering::Relaxed,
    );
}

static PACE: AtomicU8 = AtomicU8::new(1);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stillness {
    #[default]
    System,
    Always,
    Never,
}

impl Stillness {
    pub const ALL: [Self; 3] = [Self::System, Self::Always, Self::Never];

    pub fn id(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Always => "always",
            Self::Never => "never",
        }
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::System => t!("motion-system"),
            Self::Always => t!("motion-always"),
            Self::Never => t!("motion-never"),
        }
    }

    pub fn from_id(id: &str) -> Self {
        match id {
            "always" => Self::Always,
            "never" => Self::Never,
            _ => Self::System,
        }
    }

    /// Whether motion is reduced: `Some` for a hard choice, `None` when the system decides.
    pub fn still(self) -> Option<bool> {
        match self {
            Self::System => None,
            Self::Always => Some(true),
            Self::Never => Some(false),
        }
    }
}

/// The CSS `ease` timing function, cubic-bezier(0.25, 0.1, 0.25, 1), for motion copied from
/// the web.
pub fn css_ease(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0.25, 0.1, 0.25, 1.)
}

/// The CSS `ease-out` timing function, cubic-bezier(0, 0, 0.58, 1).
pub fn css_ease_out(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0., 0., 0.58, 1.)
}

pub fn ease_out_expo(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0.16, 1., 0.3, 1.)
}

pub fn ease_out_quad(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0.5, 1., 0.89, 1.)
}

pub fn ease_out_cubic(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0.33, 1., 0.68, 1.)
}

pub fn ease_in_out_cubic(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0.65, 0., 0.35, 1.)
}

pub fn ease_in_out_expo(progress: f32) -> f32 {
    cubic_bezier(progress.clamp(0., 1.), 0.87, 0., 0.13, 1.)
}

fn cubic_bezier(progress: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    if progress == 0. || progress == 1. {
        return progress;
    }

    let axis = |t: f32, a: f32, b: f32| {
        let remaining = 1. - t;
        3. * remaining * remaining * t * a + 3. * remaining * t * t * b + t * t * t
    };
    let slope = |t: f32| {
        let remaining = 1. - t;
        3. * remaining * remaining * x1 + 6. * remaining * t * (x2 - x1) + 3. * t * t * (1. - x2)
    };

    let mut parameter = progress;
    for _ in 0..6 {
        let gradient = slope(parameter);
        if gradient.abs() <= f32::EPSILON {
            break;
        }
        parameter = (parameter - (axis(parameter, x1, x2) - progress) / gradient).clamp(0., 1.);
    }
    axis(parameter, y1, y2)
}
