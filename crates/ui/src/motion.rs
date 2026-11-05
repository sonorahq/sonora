use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationElement, AnimationExt as _, App, ElementId, Hsla, IntoElement, Pixels,
    Rgba, SharedString, SpringConfig, Styled, ease_in_out, ease_out_quint, px,
};
use i18n::t;

const CONTROL: Duration = Duration::from_millis(110);
const QUICK: Duration = Duration::from_millis(120);
const BASE: Duration = Duration::from_millis(200);
const SLOW: Duration = Duration::from_millis(320);

const ENTRANCE_EXTRA: Duration = Duration::from_millis(50);
const ENTRANCE_BLUR: Pixels = px(1.5);
const ENTRANCE_ZOOM: f32 = 0.01;

/// Shared physical-motion presets. Call sites still own their stopping threshold and maximum
/// timestep because those values depend on whether the spring moves pixels, opacity, or scale.
pub enum Springs {}

impl Springs {
    /// The tuned lyrics-follow motion. This is an established visual contract.
    pub const LYRICS_SCROLL: SpringConfig = SpringConfig::new(170., 23., 1.);
    /// The first tuned lyrics-row spring, before viewport staggering lowers its frequency.
    pub const LYRICS_ROW: SpringConfig = SpringConfig::new(210., 22., 1.);
    /// Fast, nearly critically damped feedback for direct UI transitions.
    pub const RESPONSIVE: SpringConfig = SpringConfig::new(360., 38., 1.);
    /// A soft spring that overshoots a little and swings back, for an icon settling after it spun.
    pub const SETTLE: SpringConfig = SpringConfig::new(160., 17., 1.);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Control,
    Quick,
    Base,
    Slow,
}

impl Motion {
    pub fn span(self) -> Duration {
        match self {
            Self::Control => CONTROL,
            Self::Quick => QUICK.mul_f32(pace().scale()),
            Self::Base => BASE.mul_f32(pace().scale()),
            Self::Slow => SLOW.mul_f32(pace().scale()),
        }
    }

    pub fn animation(self) -> Animation {
        let animation = Animation::new(self.span());

        match self {
            Self::Control | Self::Base => animation.with_easing(ease_in_out),
            Self::Quick | Self::Slow => animation.with_easing(ease_out_quint()),
        }
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

    fn scale(self) -> f32 {
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

pub fn entrance_span() -> Duration {
    Motion::Base.span() + ENTRANCE_EXTRA
}

/// One run of the entrance a screen makes as it appears. The renderer reads how much of the
/// screen is still hidden on every frame and asks for another frame while it runs.
#[derive(Clone, Copy, Debug)]
pub struct Entrance {
    started: Instant,
    span: Duration,
}

impl Entrance {
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            span: entrance_span(),
        }
    }

    pub fn span(self) -> Duration {
        self.span
    }

    /// How much of the screen is still hidden, from 1 as the entrance starts to 0 once it ends.
    pub fn hidden(self) -> f32 {
        if self.span.is_zero() {
            return 0.;
        }
        let elapsed = self.started.elapsed().as_secs_f32();
        let progress = (elapsed / self.span.as_secs_f32()).clamp(0., 1.);
        1. - ease_out_expo(progress)
    }

    pub fn running(self) -> bool {
        self.started.elapsed() < self.span
    }
}

fn entrance() -> Animation {
    Animation::new(entrance_span()).with_easing(ease_out_expo)
}

pub fn veiled<E: Styled>(element: E, hidden: f32) -> E {
    let hidden = hidden.clamp(0., 1.);

    element
        .layer_scale(1. - ENTRANCE_ZOOM * hidden)
        .blur(ENTRANCE_BLUR * hidden)
}

/// The whole entrance: the veil plus the fade that goes with it. Use this where
/// the element has to carry its own opacity; where a scrim can stand in for the
/// fade, prefer [`veiled`], which stays out of the way of a cached layout. Never
/// put this over a cached view: opacity is baked into primitives as they are
/// painted, and a cached view replays them, so the fade freezes at whatever
/// frame the cache was filled on.
pub fn entering<E: Styled>(element: E, hidden: f32) -> E {
    veiled(element, hidden).opacity(1. - hidden.clamp(0., 1.))
}

/// The entrance's fade with none of its filter, for the one thing that cannot ride the
/// entrance itself. `veiled` and `entering` are paint filters, and the renderer drops a
/// backdrop inside a filtered layer, so a surface that frosts what it covers fades its blur in
/// as a sibling underneath the rising panel rather than rising with it.
pub trait Fading: Sized {
    fn fading(self, id: impl Into<ElementId>) -> AnimationElement<Self>;
}

impl<E: Styled + IntoElement + 'static> Fading for E {
    fn fading(self, id: impl Into<ElementId>) -> AnimationElement<Self> {
        self.with_animation(id, entrance(), |element, delta| {
            element.opacity(delta.clamp(0., 1.))
        })
    }
}

pub trait Rising: Sized {
    fn rising(self, id: impl Into<ElementId>) -> AnimationElement<Self>;
}

impl<E: Styled + IntoElement + 'static> Rising for E {
    fn rising(self, id: impl Into<ElementId>) -> AnimationElement<Self> {
        self.with_animation(id, entrance(), |element, delta| {
            entering(element, 1. - delta)
        })
    }
}

pub fn mix(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let (from, to) = (Rgba::from(from), Rgba::from(to));
    let step = t.clamp(0., 1.);
    let blend = |a: f32, b: f32| a + (b - a) * step;

    Rgba {
        r: blend(from.r, to.r),
        g: blend(from.g, to.g),
        b: blend(from.b, to.b),
        a: blend(from.a, to.a),
    }
    .into()
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

fn pace() -> Pace {
    match PACE.load(Ordering::Relaxed) {
        0 => Pace::Slow,
        2 => Pace::Quick,
        _ => Pace::Base,
    }
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

pub trait Motioned: Sized {
    fn motion(
        self,
        id: impl Into<ElementId>,
        motion: Motion,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self>;
}

impl<E: IntoElement + 'static> Motioned for E {
    fn motion(
        self,
        id: impl Into<ElementId>,
        motion: Motion,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> AnimationElement<Self> {
        self.with_animation(id, motion.animation(), animator)
    }
}

pub fn apply(stillness: Stillness, pace: Pace, cx: &mut App) {
    let generation = APPLY_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    PACE.store(
        match pace {
            Pace::Slow => 0,
            Pace::Base => 1,
            Pace::Quick => 2,
        },
        Ordering::Relaxed,
    );
    match stillness.still() {
        Some(still) => cx.set_reduce_motion(still),
        None => system::settle(cx, generation),
    }
}

pub fn animates(cx: &App) -> bool {
    !cx.reduce_motion()
}

/// How lit an element is under the pointer, eased in over `span` as the pointer arrives and out
/// as it leaves. A change halfway turns back from where the fade stands, so nothing jumps. Keep
/// one per element in `window.use_keyed_state`, tell it from `on_hover`, and read `level` while
/// drawing.
pub struct HoverFade {
    hovered: bool,
    /// Where the fade stood, unbent by easing, when the pointer last came or went.
    from: f32,
    since: Option<Instant>,
    span: Duration,
}

impl HoverFade {
    pub fn new(span: Duration) -> Self {
        Self {
            hovered: false,
            from: 0.,
            since: None,
            span,
        }
    }

    /// Turns the fade towards `hovered`, starting from wherever it stands now.
    pub fn set(&mut self, hovered: bool) {
        if hovered == self.hovered {
            return;
        }
        self.from = self.linear();
        self.hovered = hovered;
        self.since = Some(Instant::now());
    }

    /// How lit the element is, from 0 at rest to 1 hovered, eased. It asks for the next frame
    /// while the fade runs and lands at once under reduced motion.
    pub fn level(&mut self, window: &mut gpui::Window, cx: &App) -> f32 {
        let target = f32::from(u8::from(self.hovered));
        if cx.reduce_motion() {
            self.since = None;
            self.from = target;
        }
        let at = self.linear();
        if at == target {
            self.since = None;
            self.from = target;
        } else {
            window.request_animation_frame();
        }
        css_ease(at)
    }

    /// Where the fade stands before easing, moving from `from` towards the target at one span
    /// for the whole way.
    fn linear(&self) -> f32 {
        let target = f32::from(u8::from(self.hovered));
        let Some(since) = self.since else {
            return self.from;
        };
        let travelled = since.elapsed().as_secs_f32() / self.span.as_secs_f32().max(f32::EPSILON);
        match self.hovered {
            true => (self.from + travelled).min(target),
            false => (self.from - travelled).max(target),
        }
    }
}

pub(crate) struct Movement {
    drawn: bool,
    turned: Option<Instant>,
}

impl Movement {
    pub(crate) fn new(checked: bool) -> Self {
        Self {
            drawn: checked,
            turned: None,
        }
    }

    pub(crate) fn turning(&mut self, checked: bool) -> bool {
        if self.drawn != checked {
            self.drawn = checked;
            self.turned = Some(Instant::now());
        }

        self.turned
            .is_some_and(|turned| turned.elapsed() < Motion::Control.span())
    }
}

static APPLY_GENERATION: AtomicU64 = AtomicU64::new(0);

/// The operating system's reduce-motion preference. Every probe answers through
/// `App::set_reduce_motion`, which repaints on a change. A generation check keeps a late probe
/// from replacing a newer motion setting.
mod system {
    use gpui::App;

    /// Reduce Motion under System Settings > Accessibility > Display.
    #[cfg(target_os = "macos")]
    pub(super) fn settle(cx: &mut App, _generation: u64) {
        use objc2_app_kit::NSWorkspace;

        let still = NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        cx.set_reduce_motion(still);
    }

    /// "Animation effects" under Settings > Accessibility > Visual effects.
    #[cfg(windows)]
    pub(super) fn settle(cx: &mut App, _generation: u64) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SPI_GETCLIENTAREAANIMATION, SystemParametersInfoW,
        };

        let mut animated: windows_sys::core::BOOL = 1;
        // SAFETY: SPI_GETCLIENTAREAANIMATION writes one BOOL through pvparam, which is what
        // `animated` is, and asks nothing else of the caller.
        let answered = unsafe {
            SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, (&raw mut animated).cast(), 0)
        };
        cx.set_reduce_motion(answered != 0 && animated == 0);
    }

    /// Reads the standardized XDG reduced-motion preference.
    ///
    /// Older portals may not expose `org.freedesktop.appearance.reduced-motion`; that failure
    /// leaves animations enabled.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub(super) fn settle(cx: &mut App, generation: u64) {
        use ashpd::desktop::settings::{ReducedMotion, Settings};
        use gpui::AppContext as _;
        use std::sync::atomic::Ordering;

        use super::APPLY_GENERATION;

        cx.spawn(async move |cx| {
            let still = cx
                .background_spawn(async {
                    let settings = Settings::new().await.ok()?;
                    let reduced = settings.reduced_motion().await.ok()?;

                    Some(reduced == ReducedMotion::ReducedMotion)
                })
                .await;
            cx.update(|cx| {
                if APPLY_GENERATION.load(Ordering::SeqCst) == generation {
                    cx.set_reduce_motion(still.unwrap_or(false));
                }
            });
        })
        .detach();
    }

    #[cfg(not(any(
        target_os = "macos",
        windows,
        target_os = "linux",
        target_os = "freebsd"
    )))]
    pub(super) fn settle(cx: &mut App, _generation: u64) {
        cx.set_reduce_motion(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_motion_leaves_the_preference_to_the_platform() {
        assert_eq!(Stillness::System.still(), None);
        assert_eq!(Stillness::Always.still(), Some(true));
        assert_eq!(Stillness::Never.still(), Some(false));
    }

    #[test]
    fn expo_easing_has_css_endpoints_and_shape() {
        assert_eq!(ease_out_expo(0.), 0.);
        assert_eq!(ease_out_expo(1.), 1.);
        assert!(ease_out_expo(0.25) > 0.8);
        assert!(ease_out_expo(0.5) > 0.97);

        let mut previous = 0.;
        for step in 1..=20 {
            let value = ease_out_expo(step as f32 / 20.);
            assert!(value >= previous);
            previous = value;
        }
    }
}
