//! The ambient background: a flowing colour field behind fullscreen, five soft
//! blobs under a heavy blur, darkened so lyrics stay readable. `Root` owns the
//! one entity, paints it under everything, title bar included, and paints no
//! background of its own beneath it: the field stands in for the page and
//! carries the window's own opacity, so a see-through window stays see-through
//! in fullscreen.
//!
//! `ambient` decides whether it is painted at all, and with it whether the
//! fullscreen controls frost what they float over. `ambient_motion` decides
//! whether the field drifts.
//!
//! The colours are the theme's `tint` and `tint_secondary`, which fullscreen
//! samples off the cover whatever the adaptive theme setting says, resolved
//! through `Theme::accent` rather than read off `Theme::selection`, which lags
//! a track change by the length of the theme's own fade. Art with no colour
//! leaves no tint and the stage stays neutral. `painted` eases the colours on
//! screen toward those, so a cover change washes in rather than cuts.

use std::cell::Cell;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{App, Bounds, Context, Hsla, Render, Rgba, Window, canvas, div, point, px};
use state::Sonora;
use ui::{ActiveTheme as _, Theme};

/// How many blobs make up the field.
const BLOBS: usize = 5;
/// The widest blur, in device pixels, the renderer still runs at full resolution. It picks
/// the buffer from the radius: four or less keeps every pixel, eight halves the frame, and
/// anything wider drops to a quarter. The field is already drawn at `DOWNSCALE`, so a wider
/// blur would shrink it a second time and the upscale would crawl over every seam. The discs
/// carry the falloff instead.
const BLUR_FULL: f32 = 4.;
/// How many times smaller the field is drawn than it is shown. The layer is
/// laid out at this fraction of the window and the compositor scales it back
/// up with a bilinear sample, so the discs and the blur cost a sixteenth of
/// the pixels. Nothing in the field is sharper than the blur, so the upscale
/// shows nothing.
const DOWNSCALE: f32 = 4.;
/// How much of itself the field keeps under the dark overlay that holds lyrics readable over
/// it. The overlay is folded into the colours rather than painted as a sheet of its own: over
/// an opaque stack multiplying every layer by what it lets through is the same picture, and a
/// black sheet this strong was most of what a see-through window's alpha went on.
const SHADE: f32 = 0.55;

/// One blob: base centre (fractions of the layer), diameter (fraction of the
/// layer's smaller side), drift period in seconds, phase, drift amplitude
/// (fractions).
///
/// A centre travels at 2 pi A W / T at its fastest, where A is its amplitude and W the width
/// it drifts across. On a 2560 wide window these periods put that around 65 pixels a second,
/// so a frame at the display rate moves a blob half a pixel. What reads as steps at this speed
/// is not the clock but the eight bit banding underneath, which holds a contour still and then
/// jumps it a whole band at once. The dither over the field is what deals with that.
const SPECS: [(f32, f32, f32, f32, f32, f32, f32); BLOBS] = [
    (0.22, 0.30, 1.10, 34., 0.0, 0.13, 0.10),
    (0.80, 0.24, 1.00, 27., 1.7, 0.11, 0.13),
    (0.52, 0.68, 1.20, 41., 3.4, 0.14, 0.09),
    (0.12, 0.78, 0.90, 24., 5.1, 0.10, 0.12),
    (0.85, 0.72, 0.72, 31., 2.5, 0.12, 0.11),
];

/// Concentric discs faking a radial falloff. They are what keeps the field smooth now that
/// the blur stays narrow enough to run at full resolution, and at `DOWNSCALE` a disc costs a
/// sixteenth of the fill it did at window size, so the stack can afford to be dense. The
/// spacing left between them has to stay inside the blur's reach or they read as rings.
const DISCS: usize = 24;
/// Opacity ramp from the outermost disc to the core. The discs stack, so a disc is far
/// fainter than the field it builds up to.
const DISC_FAINT: f32 = 0.027;
const DISC_STRONG: f32 = 0.079;
/// Time constant of the exponential ease onto a new palette, in seconds. The
/// field lands within a few percent of the target after about three of these.
const WASH: f32 = 0.8;

pub(crate) struct Ambient {
    started: Instant,
    stepped: Instant,
    bounds: Rc<Cell<Bounds<gpui::Pixels>>>,
    painted: Option<[Hsla; BLOBS]>,
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
            bounds: Rc::new(Cell::new(Bounds::default())),
            painted: None,
        }
    }

    /// Eases the painted colours one frame toward `target` and returns them.
    /// The step is the real time since the last frame, so a gap long enough to
    /// mean the field was off screen lands on the target outright and entering
    /// fullscreen never washes in from whatever played before. With motion off
    /// there are no frames to ease over, so the target is taken as it is.
    fn wash(&mut self, target: [Hsla; BLOBS], animates: bool) -> [Hsla; BLOBS] {
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

    /// Resolves the five blob colours from the theme: the cover's leading hue
    /// carried from a light highlight down to a dark shadow, plus its runner-up
    /// where the art names one. Art with no colour leaves no tints behind, and
    /// then the stage is quiet neutrals.
    fn colors(theme: &Theme) -> [Hsla; BLOBS] {
        let neutral = |light: f32| Hsla {
            h: 0.06,
            s: 0.12,
            l: light,
            a: 1.,
        };
        // A dark theme gets a dark stage; lift the floor a little so the
        // blobs stay apart from the base.
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
        let theme = *cx.theme();
        // Without motion the field is a still gradient: no frame requests, so
        // fullscreen stops redrawing once settled. Either the system preference
        // or the setting of its own is enough to stop it.
        let animates =
            ui::motion::animates(cx) && Sonora::global(cx).settings.read(cx).ambient_motion();
        // The drift asks for the next frame off the display's own clock rather than a timer of
        // its own. A timer is never in phase with the refresh, so a blob that moves a fraction
        // of a pixel per frame lands on one vsync, skips the next and doubles the one after,
        // which reads as a twitch however slow the motion is. Cost belongs in the field below,
        // not in the frame rate.
        let elapsed = match animates {
            true => {
                window.request_animation_frame();
                self.started.elapsed().as_secs_f32()
            }
            false => 0.,
        };
        let colors = self.wash(Self::colors(&theme), animates);
        // The field stands in for the page rather than sitting on it, so it carries the
        // window's own opacity and `Root` paints no background beneath it. One opacity thins
        // every layer at once; a quad still adds to what it covers, which is why the blobs
        // read a shade denser than the gaps between them.
        let clarity = match theme.transparent {
            true => theme.background.a,
            false => 1.,
        };
        // Geometry resolves against the layer's own pixel bounds, measured a
        // frame ago by the canvas, so the discs stay circular at any window
        // aspect ratio. The field is laid out at a fraction of those bounds
        // and scaled back up from the top left corner.
        let bounds = self.bounds.get();
        let wide = bounds.size.width.as_f32().max(1.) / DOWNSCALE;
        let high = bounds.size.height.as_f32().max(1.) / DOWNSCALE;
        // The field reaches into the corners the chrome rounds, and GPUI clips only to a
        // rectangle, so the sheet takes the window's radius itself.
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let radius = crate::chrome::window_radius(Sonora::global(cx).settings.read(cx), cx, window);
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let radius: Option<gpui::Pixels> = None;

        div()
            .id("ambient")
            .absolute()
            .inset_0()
            .overflow_hidden()
            .when_some(radius, |this, radius| this.rounded(radius))
            .opacity(clarity)
            .child(
                canvas(
                    {
                        let bounds = self.bounds.clone();
                        move |got, _, _| bounds.set(got)
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when_some(radius, |this, radius| this.rounded(radius))
                    .bg(shaded(Hsla {
                        a: 1.,
                        ..theme.background
                    })),
            )
            // The blur spreads the blobs past any shape they could be cut to, so a rounded window
            // fades them out toward every edge instead. The fade sits on an unblurred layer the
            // size of the window, since a blurred one measures it from past its own blur reach.
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .when_some(radius, |this, radius| {
                        this.fade_edges(radius * 2., radius * 2.)
                            .fade_sides(radius * 2., radius * 2.)
                    })
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top_0()
                            .w(px(wide))
                            .h(px(high))
                            .layer_scale(DOWNSCALE)
                            .layer_scale_origin(point(0., 0.))
                            .blur(px(BLUR_FULL / window.scale_factor()))
                            .children(SPECS.iter().enumerate().map(|(index, spec)| {
                                let (base_x, base_y, size, period, phase, amp_x, amp_y) = *spec;
                                let spin = TAU * elapsed / period;
                                let x = base_x + amp_x * (spin + phase).sin();
                                let y = base_y + amp_y * (spin * 0.83 + phase * 1.7).cos();
                                let grown = wide.min(high)
                                    * size
                                    * (1. + 0.12 * (spin * 0.6 + phase * 2.3).sin());
                                let color = colors[index];
                                div()
                                    .absolute()
                                    .left(px(x * wide - grown / 2.))
                                    .top(px(y * high - grown / 2.))
                                    .size(px(grown))
                                    .children((0..DISCS).map(move |step| {
                                        let fraction = 1.
                                            - step as f32 / DISCS as f32 * (1. - 1. / DISCS as f32);
                                        let opacity = DISC_FAINT
                                            + step as f32 / (DISCS as f32 - 1.)
                                                * (DISC_STRONG - DISC_FAINT);
                                        let stepped = grown * fraction;
                                        div()
                                            .absolute()
                                            .left(px((grown - stepped) / 2.))
                                            .top(px((grown - stepped) / 2.))
                                            .size(px(stepped))
                                            .rounded_full()
                                            .bg(shaded(color).opacity(opacity))
                                    }))
                            })),
                    ),
            )
            // Every buffer the field passes through holds eight bits a channel, and a gradient
            // this wide and this dark steps through only a few dozen of them, so its steps read
            // as bands that slide with the blobs. The dither scatters each step over
            // neighbouring pixels instead.
            .child(ui::grain(window))
    }
}

/// Whether the ambient background is on. Fullscreen reads it for the frosted
/// glass on its controls as well, which only has the field to blur.
pub(crate) fn shown(cx: &App) -> bool {
    Sonora::global(cx).settings.read(cx).ambient()
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
