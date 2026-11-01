use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use gpui::{App, Global, Window, ease_in_out};

use crate::motion::mix;
use crate::{ActiveTheme as _, AmbientPalette, CoverPalette, Look, Theme, ThemeOverrides};

const FADE: Duration = Duration::from_millis(800);
const AMBIENT_FADE: Duration = Duration::from_secs(4);
const SURFACE_WEIGHT: f32 = 0.68;
const HEADER_WEIGHT: f32 = 0.5;

/// Ambient's current runtime treatment, separate from a theme's colour tokens.
#[derive(Clone, Copy, Default)]
pub struct AmbientStyle {
    pub strength: f32,
    pub palette: AmbientPalette,
    /// Artwork-dependent centres, interpolated independently of colour and orbital motion.
    pub centers: [[f32; 2]; 5],
}

impl Global for AmbientStyle {}

impl AmbientStyle {
    /// Chooses an artwork layout once, so colour interpolation cannot bend its motion path.
    pub fn new(strength: f32, palette: AmbientPalette) -> Self {
        let centers = palette.colors.map(|color| {
            let phase = (TAU * color.h).sin() * color.s * 2. + color.s * 2.3 + color.l * 3.8;
            [0.5 + 0.18 * phase.sin(), 0.5 + 0.18 * (phase * 1.7).cos()]
        });
        Self {
            strength,
            palette,
            centers,
        }
    }

    /// The current treatment, or the ordinary opaque-page treatment before initialization.
    pub fn get(cx: &App) -> Self {
        cx.try_global::<Self>().copied().unwrap_or_default()
    }

    pub fn active(self) -> bool {
        self.strength > 0.
    }
}

/// A resolved colour theme and its optional runtime background, sharing one transition clock.
#[derive(Clone, Copy)]
pub struct Appearance {
    pub theme: Theme,
    pub ambient: AmbientStyle,
}

#[derive(Default)]
struct Transition {
    generation: u64,
}

impl Global for Transition {}

impl Appearance {
    pub(crate) fn new(theme: Theme) -> Self {
        Self {
            theme,
            ambient: AmbientStyle::default(),
        }
    }

    /// Resolves Ambient without changing the ordinary theme-building path. Zero strength uses it directly.
    pub fn resolve(
        look: Look,
        overrides: &ThemeOverrides,
        cover: CoverPalette,
        strength: f32,
    ) -> Self {
        let strength = strength.clamp(0., 1.);
        if strength == 0. {
            return Self::new(Theme::for_look(look, overrides));
        }
        let look = Look {
            tint: cover.primary,
            tint_secondary: cover.secondary,
            ..look
        };
        let mut theme = Theme::for_kind(look.kind);
        if let Some(tint) = cover.primary {
            let mut tinted = theme.tinted(tint);
            let dark = theme.background.l < 0.5;
            // Pure black cannot show a hue. Lift low surfaces just enough for the
            // artwork wash to read as clearly as it does on coloured theme files.
            for surface in [
                &mut tinted.sidebar,
                &mut tinted.secondary,
                &mut tinted.muted,
            ] {
                surface.s = surface.s.max(tint.s * 0.4);
                if dark {
                    surface.l = surface.l.max(0.14);
                }
            }
            // The solid player surface needs visible colour in light mode too; a
            // near-white fill hides its hue even at full artwork strength.
            tinted.secondary.s = tinted.secondary.s.max(tint.s * 0.6);
            tinted.secondary.l = match dark {
                true => tinted.secondary.l.max(0.18),
                false => tinted.secondary.l.min(0.84),
            };
            // Keep text nearly neutral: saturated ink loses contrast against the field's
            // changing hues, especially for small labels and secondary track information.
            for text in [&mut tinted.foreground, &mut tinted.popover_foreground] {
                text.s = text.s.min(0.08);
                text.l = match dark {
                    true => 0.98,
                    false => 0.10,
                };
            }
            for text in [
                &mut tinted.muted_foreground,
                &mut tinted.table_head_foreground,
            ] {
                text.s = text.s.min(0.10);
                text.l = match dark {
                    true => 0.92,
                    false => 0.26,
                };
            }
            theme = theme.mixed(&tinted, strength);
        }
        theme = Theme::for_palette(theme, look, overrides);
        theme.sidebar.a = 0.;
        theme.secondary = theme
            .secondary
            .opacity(1. - strength * (1. - SURFACE_WEIGHT));
        theme.table_head = theme
            .table_head
            .opacity(1. - strength * (1. - HEADER_WEIGHT));
        let palette = AmbientPalette::new(&theme, cover.background);
        Self {
            theme,
            ambient: AmbientStyle::new(strength, palette),
        }
    }

    pub fn set(self, cx: &mut App) {
        cx.default_global::<Transition>().generation += 1;
        self.apply(cx);
    }

    /// Fades both outputs from their current displayed values, replacing an interrupted fade.
    pub fn fade(self, cx: &mut App) {
        let Some(handle) = cx
            .windows()
            .first()
            .copied()
            .filter(|_| crate::motion::animates(cx))
        else {
            self.set(cx);
            return;
        };
        let from = Self {
            theme: *cx.theme(),
            ambient: AmbientStyle::get(cx),
        };
        cx.default_global::<Transition>().generation += 1;
        let generation = cx.default_global::<Transition>().generation;
        let start = Instant::now();
        from.mixed(self, 0.).apply(cx);
        // Defer until the current window event releases its borrow.
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, _| {
                    window.on_next_frame(move |window, cx| {
                        fade_frame(from, self, start, generation, window, cx)
                    });
                })
                .ok();
        });
    }

    fn mixed(self, mut other: Self, delta: f32) -> Self {
        other.theme = self.theme.mixed(&other.theme, delta);
        if self.ambient.active() || other.ambient.active() {
            for (from, to) in self
                .ambient
                .centers
                .into_iter()
                .zip(&mut other.ambient.centers)
            {
                for (from, to) in from.into_iter().zip(to) {
                    *to = from + (*to - from) * delta;
                }
            }
            let from = self.ambient.palette;
            let to = &mut other.ambient.palette;
            for (from, to) in from.colors.into_iter().zip(&mut to.colors) {
                *to = mix(from, *to, delta);
            }
            to.background = mix(from.background, to.background, delta);
        }
        other
    }

    fn apply(self, cx: &mut App) {
        cx.set_global(self.theme);
        cx.set_global(self.ambient);
        cx.refresh_windows();
    }
}

/// Advances the runtime appearance on GPUI's display clock; a newer target cancels old frames.
fn fade_frame(
    from: Appearance,
    to: Appearance,
    start: Instant,
    generation: u64,
    window: &mut Window,
    cx: &mut App,
) {
    if cx.default_global::<Transition>().generation != generation {
        return;
    }
    // Artwork changes also move the field, so give them more time than a theme switch.
    let duration = match from.ambient.active() && to.ambient.active() {
        true => AMBIENT_FADE,
        false => FADE,
    };
    let delta = match crate::motion::animates(cx) {
        true => (start.elapsed().as_secs_f32() / duration.as_secs_f32()).min(1.),
        false => 1.,
    };
    from.mixed(to, ease_in_out(delta)).apply(cx);
    if delta < 1. {
        window.on_next_frame(move |window, cx| fade_frame(from, to, start, generation, window, cx));
    }
}
