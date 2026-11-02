//! The ambient background uses the upstream wave shader across the app and framed previews.
//! Artwork colours share the appearance transition. Strength blends the field with the base
//! theme while preserving window opacity; motion intensity controls its drift speed.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{App, Context, Pixels, Render, Task, Window, div};
use state::Sonora;
use ui::{ActiveTheme as _, AmbientPalette, AmbientStyle, CoverPalette, Theme};

/// The flowing colour field. One sits behind the whole window in fullscreen, and a preview can
/// hold another of its own made with `Ambient::framed`.
pub(crate) struct Ambient {
    started: Instant,
    /// The palette of the cover a framed field follows, `None` for the one behind the window.
    framed: Option<Sampled>,
}

/// The palette of the cover a framed field follows, sampled by the field itself.
#[derive(Default)]
struct Sampled {
    cover: Option<String>,
    palette: CoverPalette,
    _task: Option<Task<()>>,
}

impl Ambient {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let settings = Sonora::global(cx).settings.clone();
        // Turning the drift back on has no frame of its own to land in: with
        // motion off nothing asks for one.
        cx.observe(&settings, |_, _, cx| cx.notify()).detach();

        Self {
            started: Instant::now(),
            framed: None,
        }
    }

    /// A field for a preview that fills a frame rather than the window. It takes its colours
    /// from the cover `follow` hands it whatever the adaptive theme says. The frame rounds it.
    pub(crate) fn framed(cx: &mut Context<Self>) -> Self {
        Self {
            framed: Some(Sampled::default()),
            ..Self::new(cx)
        }
    }

    /// Samples `cover` for a framed field once it differs from the one sampled last. A field
    /// behind the window takes its colours from the theme and ignores this.
    pub(crate) fn follow(&mut self, cover: Option<String>, cx: &mut Context<Self>) {
        let Some(sampled) = self.framed.as_mut() else {
            return;
        };
        if sampled.cover == cover {
            return;
        }
        sampled.cover = cover.clone();
        let Some(cover) = cover else {
            sampled._task = None;
            sampled.palette = CoverPalette::default();
            cx.notify();
            return;
        };
        let palette = ui::palette(cover, cx);
        sampled._task = Some(cx.spawn(async move |this, cx| {
            let palette = palette.await;
            this.update(cx, |this, cx| {
                if let Some(sampled) = this.framed.as_mut() {
                    sampled.palette = palette;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
    /// The appearance worn by the field, shared with settings previews drawn over it.
    pub(crate) fn theme(&self, cx: &App) -> Theme {
        match self.framed.is_some() {
            true => Theme::for_kind(Sonora::global(cx).settings.read(cx).resolved_appearance()),
            false => *cx.theme(),
        }
    }
}

impl Render for Ambient {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme(cx);
        // Without motion the field is a still gradient: no frame requests, so
        // fullscreen stops redrawing once settled. Either the system preference
        // or the setting of its own is enough to stop it.
        let motion = match ui::motion::animates(cx) {
            true => Sonora::global(cx).settings.read(cx).ambient_motion(),
            false => 0.,
        };
        // The drift asks for the next frame off the display's own clock rather than a timer of
        // its own. A timer is never in phase with the refresh, so a field that moves a fraction
        // of a pixel per frame lands on one vsync, skips the next and doubles the one after,
        // which reads as a twitch however slow the motion is. Cost belongs in the field below,
        // not in the frame rate.
        let elapsed = match motion > 0. {
            true => {
                window.request_animation_frame();
                self.started.elapsed().as_secs_f32() * motion
            }
            false => 0.,
        };
        let style = self
            .framed
            .as_ref()
            .map(|sampled| {
                AmbientStyle::new(
                    1.,
                    AmbientPalette::new(
                        &Theme {
                            tint: sampled.palette.primary,
                            tint_secondary: sampled.palette.secondary,
                            ..theme
                        },
                        sampled.palette.background,
                    ),
                )
            })
            .unwrap_or(AmbientStyle::get(cx));
        // Root supplies the remaining base contribution, preserving the window opacity.
        let clarity = match theme.transparent {
            true => theme.background.a,
            false => 1.,
        } * match self.framed.is_some() {
            true => 1.,
            false => Sonora::global(cx).settings.read(cx).ambient_strength(),
        };
        let radius = match self.framed.is_some() {
            true => None,
            false => window_radius(window, cx),
        };
        // Lay the runner-up over the shadows, as in the upstream wave field.
        let [base, highlight, second, shadow, deep] = style.palette.colors;
        div()
            .id("ambient")
            .absolute()
            .inset_0()
            .opacity(clarity)
            .child(
                ui::Waves::new([base, highlight, shadow, deep, second])
                    .stage(style.palette.background)
                    .time(elapsed)
                    .light(theme.background.l >= 0.5)
                    .when_some(radius, ui::Waves::corner_radius),
            )
    }
}

/// Whether the ambient background is on. Fullscreen reads it for the frosted
/// glass on its controls as well, which only has the field to blur.
pub(crate) fn shown(cx: &App) -> bool {
    let settings = Sonora::global(cx).settings.read(cx);
    settings.ambient()
}

/// The radius of the window's own corners, where the chrome rounds them.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn window_radius(window: &Window, cx: &App) -> Option<Pixels> {
    crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx, window)
}

#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
fn window_radius(_window: &Window, _cx: &App) -> Option<Pixels> {
    None
}
