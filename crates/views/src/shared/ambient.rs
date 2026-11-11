//! The ambient background: a flowing colour field behind fullscreen, five colours folded into
//! one another by slow waves in the `ui::Waves` shader, darkened so lyrics stay readable. `Root`
//! owns the one entity, paints it under everything, title bar included, and paints no
//! background of its own beneath it. The field stands in for the page and carries the window's
//! own opacity, so a see-through window stays see-through in fullscreen.
//!
//! `ambient` decides whether it is painted at all, and with it whether the fullscreen controls
//! frost what they float over. `ambient_motion` decides whether the field drifts.
//!
//! The colours are the theme's `tint` and `tint_secondary`, which fullscreen samples off the
//! cover whatever the adaptive theme setting says, resolved through `Theme::accent` rather than
//! read off `Theme::selection`, which lags a track change by the length of the theme's own fade.
//! Art with no colour leaves no tint and the stage stays neutral. `painted` eases the colours on
//! screen toward those, so a cover change washes in rather than cuts.

use std::time::Instant;

use gpui::prelude::*;
use gpui::{App, Context, Hsla, Pixels, Render, Rgba, Task, Window, div};
use state::Sonora;
use ui::{ActiveTheme as _, CoverPalette, Theme};

use crate::shared::adaptive;

/// How many colours make up the field.
const COLORS: usize = 5;
/// How much of itself the field keeps under the dark overlay that holds lyrics readable over
/// it. The overlay is folded into the colours rather than painted as a sheet of its own, which
/// over an opaque field is the same picture and leaves a see-through window's alpha alone.
const SHADE: f32 = 0.55;
/// Time constant of the exponential ease onto a new palette, in seconds. The
/// field lands within a few percent of the target after about three of these.
const WASH: f32 = 0.8;
/// How many times smaller across each side the field behind the window is drawn before it is
/// stretched back over it. Nothing in it is sharper than a heavy blur, so the stretch does not show,
/// and the shader runs on a sixteenth of the pixels.
const FIELD_SHRINK: f32 = 4.;

/// The flowing colour field. One sits behind the whole window in fullscreen, and a preview can
/// hold another of its own made with `Ambient::framed`.
pub(crate) struct Ambient {
    started: Instant,
    stepped: Instant,
    painted: Option<[Hsla; COLORS]>,
    /// Whether a wait for the next drift frame is already running.
    pacing: Option<Task<()>>,
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

        let now = Instant::now();
        Self {
            started: now,
            stepped: now,
            painted: None,
            pacing: None,
            framed: None,
        }
    }

    /// A field for a preview that fills a frame rather than the window. It takes its colours
    /// from the cover `follow` hands it whatever the adaptive theme says, and leaves its corners
    /// square for the frame to round.
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

    /// The theme this field is painted from: the app's own behind the window, and in a frame
    /// the one fullscreen would wear for the sampled cover. A preview draws over it with the same.
    pub(crate) fn theme(&self, cx: &App) -> Theme {
        match &self.framed {
            None => *cx.theme(),
            Some(sampled) => adaptive::stage(sampled.palette, cx),
        }
    }

    /// Schedules the next drift update without keeping display callbacks pending between updates.
    fn pace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pacing.is_some() {
            return;
        }
        let timer = super::animation::next(4, cx);
        let handle = window.window_handle();
        self.pacing = Some(cx.spawn(async move |this, cx| {
            timer.await;
            if this
                .update(cx, |this, cx| {
                    this.pacing = None;
                    cx.notify();
                })
                .is_err()
            {
                return;
            }
            handle
                .update(cx, |_, window, _| window.on_next_frame(|_, _| {}))
                .ok();
        }));
    }

    /// Eases the painted colours one frame toward `target` and returns them.
    /// The step is the real time since the last frame, so a gap long enough to
    /// mean the field was off screen lands on the target outright and entering
    /// fullscreen never washes in from whatever played before. With motion off
    /// there are no frames to ease over, so the target is taken as it is.
    fn wash(&mut self, target: [Hsla; COLORS], animates: bool) -> [Hsla; COLORS] {
        let now = Instant::now();
        let step = now.duration_since(self.stepped).as_secs_f32();
        self.stepped = now;

        let painted = match self.painted {
            Some(painted) if animates => {
                let delta = 1. - (-step / WASH).exp();
                std::array::from_fn(|index| blend(painted[index], target[index], delta))
            }
            _ => target,
        };
        self.painted = Some(painted);
        painted
    }

    /// Resolves the five field colours from the theme: the cover's leading hue
    /// carried from a light highlight down to a dark shadow, plus its runner-up
    /// where the art names one. Art with no colour leaves no tints behind, and
    /// then the stage is quiet neutrals.
    pub(crate) fn colors(theme: &Theme) -> [Hsla; COLORS] {
        let neutral = |light: f32| Hsla {
            h: 0.06,
            s: 0.12,
            l: light,
            a: 1.,
        };
        // A dark theme gets a dark stage; lift the floor a little so the
        // pools stay apart from the stage.
        let floor = (0.13 + theme.background.l * 0.8).clamp(0.13, 0.30);

        let Some(tint) = theme.tint else {
            return [
                neutral(floor),
                neutral(floor + 0.025),
                neutral(floor + 0.05),
                neutral(floor + 0.015),
                neutral(floor + 0.04),
            ];
        };
        let accent = theme.accent(tint);
        let base = Hsla {
            h: accent.h,
            s: (accent.s * 0.9 + 0.04).clamp(0.45, 0.75),
            l: (0.40 + (accent.l - 0.45) * 0.5).clamp(0.28, 0.52),
            a: 1.,
        };
        let second = match theme.tint_secondary {
            Some(second) => Hsla {
                h: second.h,
                s: (second.s * 0.9).clamp(0.4, 0.72),
                l: (0.38 + (second.l - 0.45) * 0.5).clamp(0.28, 0.5),
                a: 1.,
            },
            None => neutral(floor + 0.03),
        };
        [
            base,
            Hsla {
                l: (base.l + 0.14).clamp(0.3, 0.62),
                s: (base.s - 0.08).clamp(0.4, 0.75),
                ..base
            },
            second,
            Hsla {
                l: (base.l - 0.13).clamp(0.18, 0.45),
                s: (base.s + 0.04).clamp(0.45, 0.8),
                ..base
            },
            Hsla {
                h: (base.h + 0.97).rem_euclid(1.),
                l: (base.l - 0.16).clamp(0.16, 0.4),
                s: (base.s - 0.02).clamp(0.4, 0.78),
                ..base
            },
        ]
    }
}

impl Render for Ambient {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme(cx);
        // Without motion the field is a still gradient: no frame requests, so
        // fullscreen stops redrawing once settled. Either the system preference
        // or the setting of its own is enough to stop it.
        let animates =
            ui::motion::animates(cx) && Sonora::global(cx).settings.read(cx).ambient_motion();
        let elapsed = match animates {
            true => {
                self.pace(window, cx);
                self.started.elapsed().as_secs_f32()
            }
            false => 0.,
        };
        let colors = self.wash(Self::colors(&theme), animates);
        // The field stands in for the page rather than sitting on it, so it carries the
        // window's own opacity and `Root` paints no background beneath it.
        let clarity = match theme.transparent {
            true => theme.background.a,
            false => 1.,
        };
        // The field reaches into the corners the chrome rounds, so it takes that radius itself.
        // A framed field leaves its corners to the frame.
        let radius = match self.framed {
            None => window_radius(window, cx),
            Some(_) => None,
        };

        let waves = ui::Waves::new(layered(colors).map(shaded))
            .stage(shaded(Hsla {
                a: 1.,
                ..theme.background
            }))
            .time(elapsed);

        div()
            .id("ambient")
            .absolute()
            .inset_0()
            .opacity(clarity)
            .map(|this| match self.framed {
                // A preview is small enough to draw at full size.
                Some(_) => this.child(waves),
                None => {
                    let viewport = window.viewport_size();
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .w((viewport.width / FIELD_SHRINK).ceil())
                            .h((viewport.height / FIELD_SHRINK).ceil())
                            .layer_scale(FIELD_SHRINK)
                            .layer_scale_origin(gpui::point(0., 0.))
                            .child(waves.when_some(
                                radius.map(|radius| radius / FIELD_SHRINK),
                                ui::Waves::corner_radius,
                            )),
                    )
                }
            })
    }
}

/// Whether the ambient background is on. Fullscreen reads it for the frosted
/// glass on its controls as well, which only has the field to blur.
pub(crate) fn shown(cx: &App) -> bool {
    Sonora::global(cx).settings.read(cx).ambient()
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

/// The colours in the order the field lays them down, the runner-up last. Each pool covers the
/// ones under it, so drawn third it sank beneath the two shadows and a cover's second colour
/// barely showed.
fn layered(colors: [Hsla; COLORS]) -> [Hsla; COLORS] {
    let [base, highlight, second, shadow, deep] = colors;
    [base, highlight, shadow, deep, second]
}

/// A colour as the dark overlay leaves it. Black at `SHADE` over an opaque colour is that
/// colour's channels cut by what the sheet lets through, so cutting them here is the same
/// picture with one layer fewer to spend the window's alpha on.
fn shaded(color: Hsla) -> Hsla {
    let rgba = Rgba::from(color);
    let kept = 1. - SHADE;

    Hsla::from(Rgba {
        r: rgba.r * kept,
        g: rgba.g * kept,
        b: rgba.b * kept,
        a: rgba.a,
    })
}

/// Straight-line blend between two colours through RGB, so a wash between two
/// unrelated hues passes through grey rather than through every hue between
/// them.
fn blend(from: Hsla, to: Hsla, delta: f32) -> Hsla {
    let (from, to) = (Rgba::from(from), Rgba::from(to));
    let channel = |from: f32, to: f32| from + (to - from) * delta;

    Hsla::from(Rgba {
        r: channel(from.r, to.r),
        g: channel(from.g, to.g),
        b: channel(from.b, to.b),
        a: channel(from.a, to.a),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tinted(hue: f32) -> Theme {
        let mut theme = Theme::dark();
        theme.tint = Some(Hsla {
            h: hue,
            s: 0.6,
            l: 0.4,
            a: 1.,
        });
        theme.selection = Hsla {
            h: hue,
            s: 0.7,
            l: 0.44,
            a: 1.,
        };
        theme.tint_secondary = Some(Hsla {
            h: (hue + 0.5).rem_euclid(1.),
            s: 0.6,
            l: 0.4,
            a: 1.,
        });
        theme
    }

    #[test]
    fn accent_theme_yields_its_family() {
        let colors = Ambient::colors(&tinted(0.78));

        for (index, color) in colors.iter().enumerate() {
            // The runner-up sits in slot two; everything else is family.
            if index == 2 {
                let gap = (color.h - 0.28).abs().min(1. - (color.h - 0.28).abs());
                assert!(gap < 0.06, "second hue was {}", color.h);
                continue;
            }
            let gap = (color.h - 0.78).abs().min(1. - (color.h - 0.78).abs());
            assert!(gap < 0.06, "hue was {}", color.h);
        }
        assert!(colors[1].l > colors[0].l, "highlight lifts");
        assert!(colors[3].l < colors[0].l, "shadow sinks");
    }

    #[test]
    fn untinted_theme_yields_neutrals() {
        let colors = Ambient::colors(&Theme::dark());

        assert!(colors.iter().all(|color| color.s < 0.2));
    }
}
