//! The seek and volume slider: a thin track filled to a fraction with a round thumb on it, that
//! seeks where it is pressed and follows a drag, and thickens under the pointer.
//!
//! The track's geometry lives in a [`ScrubberState`] the owner keeps, so playback can move the
//! fill from any thread with [`ScrubberState::set`] without rebuilding the view, through Rune's
//! live rects. A view that passes a fraction in [`Scrubber::new`] sets the same values itself.

use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use rune::{Cursor, Drag, El, Live, Phase, PointerEvent, Spring, TextStyle, Tx, el, row, text};

use crate::color::Hsla;
use crate::label::Weight;
use crate::metrics::Text;
use crate::paint::rems;
use crate::theme::Theme;

/// The track's thickness, the thumb's side and the height the pointer can press in, as parts of
/// the theme's pad.
const TRACK: f32 = 0.5;
const THUMB: f32 = 1.5;
const HIT: f32 = 3.;
/// How wide the bubble over the track is, as parts of the pad.
const BUBBLE: f32 = 7.;
/// How far the bubble stands off the track unless `lift` says otherwise.
const LIFT: f32 = 12.;
/// How far one keyboard or screen reader step moves the slider, as a share of its length.
const STEP: f32 = 0.05;
/// How much thicker the track and the thumb draw under the pointer.
const GROW_TRACK: f32 = 1.6;
const GROW_THUMB: f32 = 1.2;
/// How fast the drawn geometry catches up with the value: quick enough that a drag feels held,
/// slow enough that playback's steps and the hover growth glide.
const CATCH_UP: Spring = Spring::new(0.08, 0.);
/// A radius larger than any track, which the renderer caps at half its thickness.
const ROUND: f32 = 9999.;

type Slide<S> = Rc<dyn Fn(&mut S, f32, &mut Tx)>;
type Release<S> = Rc<dyn Fn(&mut S, &mut Tx)>;
type Hover<S> = Rc<dyn Fn(&mut S, Option<f32>, &mut Tx)>;

/// What the drawn geometry is computed from.
#[derive(Clone, Copy, Debug, Default)]
struct Geometry {
    /// The pressable area's laid out size, unknown until its first layout.
    size: [f32; 2],
    fraction: f32,
    vertical: bool,
    line: f32,
    pin: f32,
    enabled: bool,
    hovered: bool,
    /// Where the bubble points, and its width.
    bubble: Option<f32>,
    bubble_width: f32,
    measured: bool,
}

/// A scrubber's geometry and the live values it draws through. Keep one per scrubber in the
/// window's state; it is cheap to clone and can be moved to a producer thread.
#[derive(Clone)]
pub struct ScrubberState {
    geometry: Arc<Mutex<Geometry>>,
    track: Live<[f32; 4]>,
    fill: Live<[f32; 4]>,
    thumb: Live<[f32; 4]>,
    bubble: Live<[f32; 2]>,
}

impl Default for ScrubberState {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrubberState {
    pub fn new() -> Self {
        Self {
            geometry: Arc::default(),
            track: Live::new([0.; 4]),
            fill: Live::new([0.; 4]),
            thumb: Live::new([0.; 4]),
            bubble: Live::new([0.; 2]),
        }
    }

    /// Moves the fill and the thumb to `fraction`, from any thread, without running the view.
    pub fn set(&self, fraction: f32) {
        self.update(|geometry| geometry.fraction = fraction.clamp(0., 1.));
    }

    /// The fraction the scrubber shows now.
    pub fn fraction(&self) -> f32 {
        self.read().fraction
    }

    /// The fraction under a point along the track, in the pressable area's own points, as a
    /// drag or a hover reports it.
    pub fn fraction_at(&self, x: f32, y: f32) -> f32 {
        let geometry = self.read();
        at(&geometry, x, y)
    }

    fn read(&self) -> Geometry {
        *self.geometry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn update(&self, change: impl FnOnce(&mut Geometry)) {
        let geometry = {
            let mut geometry = self.geometry.lock().unwrap_or_else(PoisonError::into_inner);
            change(&mut geometry);
            *geometry
        };
        self.apply(geometry);
    }

    /// Publishes the rects `geometry` draws. The first measured layout lands at once, so a
    /// scrubber does not sweep in from nothing as it mounts.
    fn apply(&self, geometry: Geometry) {
        if geometry.size[0] <= 0. || geometry.size[1] <= 0. {
            return;
        }
        let rects = rects(&geometry);
        let publish = |live: &Live<[f32; 4]>, value: [f32; 4]| match geometry.measured {
            true => live.set(value),
            false => live.snap(value),
        };
        publish(&self.track, rects.track);
        publish(&self.fill, rects.fill);
        publish(&self.thumb, rects.thumb);
        match geometry.measured {
            true => self.bubble.set(rects.bubble),
            false => self.bubble.snap(rects.bubble),
        };
        if !geometry.measured {
            self.geometry
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .measured = true;
        }
    }
}

/// The rects one frame draws, in the pressable area's points.
struct Rects {
    track: [f32; 4],
    fill: [f32; 4],
    thumb: [f32; 4],
    bubble: [f32; 2],
}

fn rects(g: &Geometry) -> Rects {
    let [w, h] = g.size;
    let line = match g.hovered && g.enabled {
        true => (g.line * GROW_TRACK).round(),
        false => g.line,
    };
    let pin = match (g.enabled, g.hovered) {
        (false, _) => 0.,
        (true, true) => (g.pin * GROW_THUMB).round(),
        (true, false) => g.pin,
    };
    let inset = match g.enabled {
        true => g.pin / 2.,
        false => 0.,
    };
    let along = match g.vertical {
        true => h,
        false => w,
    };
    let travel = (along - inset * 2.).max(0.);
    let reach = inset + g.fraction * travel;
    let bubble = inset + g.bubble.unwrap_or(0.) * travel;
    match g.vertical {
        false => {
            let top = (h - line) / 2.;
            Rects {
                track: [0., top, w, line],
                fill: [0., top, reach, line],
                thumb: [reach - pin / 2., (h - pin) / 2., pin, pin],
                bubble: [bubble - g.bubble_width / 2., 0.],
            }
        }
        true => {
            let left = (w - line) / 2.;
            Rects {
                track: [left, 0., line, h],
                fill: [left, h - reach, line, reach],
                thumb: [(w - pin) / 2., h - reach - pin / 2., pin, pin],
                bubble: [0., h - bubble - g.pin / 2.],
            }
        }
    }
}

/// The fraction under `(x, y)`: along the track from its start, or up from its foot when it
/// stands upright. The thumb's centre lands under the pointer.
fn at(g: &Geometry, x: f32, y: f32) -> f32 {
    let pin = g.pin;
    let (along, length) = match g.vertical {
        true => (g.size[1] - y, g.size[1]),
        false => (x, g.size[0]),
    };
    let travel = length - pin;
    if travel <= 0. {
        return 0.;
    }
    ((along - pin / 2.) / travel).clamp(0., 1.)
}

/// A scrubber. Its root is the pressable slider itself, keyed after the state's owner's choice.
pub struct Scrubber<S> {
    key: String,
    state: ScrubberState,
    fraction: Option<f32>,
    colors: Option<(Hsla, Hsla, Hsla)>,
    enabled: bool,
    vertical: bool,
    name: Option<String>,
    value: Option<String>,
    bubble: Option<(f32, String)>,
    lift: f32,
    on_move: Option<Slide<S>>,
    on_release: Option<Release<S>>,
    on_hover: Option<Hover<S>>,
}

impl<S: 'static> Scrubber<S> {
    /// A scrubber showing `fraction`, for a value the view knows, such as the volume.
    pub fn new(key: impl Into<String>, state: &ScrubberState, fraction: f32) -> Self {
        Self {
            fraction: Some(fraction.clamp(0., 1.)),
            ..Self::live(key, state)
        }
    }

    /// A scrubber showing whatever its state was last [`set`](ScrubberState::set) to, for a
    /// value that moves every frame, such as playback's position.
    pub fn live(key: impl Into<String>, state: &ScrubberState) -> Self {
        Self {
            key: key.into(),
            state: state.clone(),
            fraction: None,
            colors: None,
            enabled: true,
            vertical: false,
            name: None,
            value: None,
            bubble: None,
            lift: LIFT,
            on_move: None,
            on_release: None,
            on_hover: None,
        }
    }

    /// A bubble over the track at `fraction` saying `text`, such as the time a seek lands on.
    pub fn bubble(mut self, fraction: f32, text: impl Into<String>) -> Self {
        self.bubble = Some((fraction.clamp(0., 1.), text.into()));
        self
    }

    /// How far the bubble stands off the track; a negative lift puts it under the track.
    pub fn lift(mut self, lift: f32) -> Self {
        self.lift = lift;
        self
    }

    /// How far a bubble standing `lift` off the track reaches past it.
    pub fn bubble_reach(lift: f32, theme: &Theme) -> f32 {
        lift + (theme.metrics.pad * BUBBLE).round()
    }

    /// The filled part, the empty part and the thumb, the theme's progress colours unless set.
    pub fn colors(mut self, filled: Hsla, empty: Hsla, thumb: Hsla) -> Self {
        self.colors = Some((filled, empty, thumb));
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }

    /// Names the slider for screen readers after what it sets, such as the volume.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The position as a screen reader should read it, such as the elapsed time.
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Runs with the fraction under the pointer as it presses and drags, and with one step either
    /// way from the keyboard or a screen reader.
    pub fn on_move(mut self, f: impl Fn(&mut S, f32, &mut Tx) + 'static) -> Self {
        self.on_move = Some(Rc::new(f));
        self
    }

    /// Runs when a drag lets go, and after each keyboard step, for an owner that commits a seek
    /// only then.
    pub fn on_release(mut self, f: impl Fn(&mut S, &mut Tx) + 'static) -> Self {
        self.on_release = Some(Rc::new(f));
        self
    }

    /// Runs with the fraction under the pointer while it rests over the track, and with `None`
    /// when it leaves, for an owner that shows where a press would land in a bubble.
    pub fn on_hover(mut self, f: impl Fn(&mut S, Option<f32>, &mut Tx) + 'static) -> Self {
        self.on_hover = Some(Rc::new(f));
        self
    }

    pub fn build(self, theme: &Theme) -> El {
        let pad = theme.metrics.pad;
        let line = (pad * TRACK).round();
        let pin = (pad * THUMB).round();
        let reach = (pad * HIT).round();
        let bubble_width = (pad * BUBBLE).round();
        let (filled, empty, thumb) =
            self.colors
                .unwrap_or((theme.progress_bar, theme.muted, theme.progress_bar));
        let state = self.state.clone();
        let interactive = self.enabled && self.on_move.is_some();
        state.update(|g| {
            g.vertical = self.vertical;
            g.line = line;
            g.pin = pin;
            g.enabled = self.enabled;
            g.bubble = self.bubble.as_ref().map(|(at, _)| *at);
            g.bubble_width = bubble_width;
            if let Some(fraction) = self.fraction {
                g.fraction = fraction;
            }
            if !interactive {
                g.hovered = false;
            }
        });
        let fraction = state.fraction();

        let measured = state.clone();
        let mut scrubber = row()
            .key(self.key)
            .role("slider")
            .range_value(f64::from(fraction), 0., 1.)
            .on_layout_tx(move |_: &mut S, rect: [f32; 4], tx: &mut Tx| {
                tx.live_only();
                measured.update(|g| g.size = [rect[2], rect[3]]);
            })
            .child(
                el().key("track")
                    .absolute(0., 0.)
                    .live_rect(state.track.smooth(CATCH_UP))
                    .radius(ROUND)
                    .fill(empty.color())
                    .a11y_hidden(),
            )
            .child(
                el().key("fill")
                    .absolute(0., 0.)
                    .live_rect(state.fill.smooth(CATCH_UP))
                    .radius(ROUND)
                    .fill(filled.color())
                    .a11y_hidden(),
            );
        scrubber = match self.vertical {
            true => scrubber.w(reach).h_full(),
            false => scrubber.w_full().h(reach),
        };
        if self.enabled {
            scrubber = scrubber.child(
                el().key("thumb")
                    .absolute(0., 0.)
                    .live_rect(state.thumb.smooth(CATCH_UP))
                    .radius(ROUND)
                    .fill(thumb.color())
                    .a11y_hidden(),
            );
        }
        if let Some((_, words)) = self.bubble {
            let label = text(words)
                .font(TextStyle::new(theme.text(Text::Tiny), Weight::NORMAL))
                .color(theme.popover_foreground.color());
            let chip = row()
                .px(rems(theme, 0.375))
                .radius(theme.radius)
                .fill(theme.popover.color())
                .border(1., theme.border.color())
                .child(label);
            let bubble = match self.vertical {
                false => {
                    let above = (reach + line) / 2. + self.lift;
                    let placed = match self.lift >= 0. {
                        true => row().pin(Some(0.), None, None, Some(above)),
                        false => {
                            row().pin(Some(0.), Some((reach + line) / 2. - self.lift), None, None)
                        }
                    };
                    placed.w(bubble_width).justify_center()
                }
                true => {
                    let side = (reach + line) / 2. + self.lift.abs();
                    match self.lift >= 0. {
                        true => row().pin(None, Some(0.), Some(side), None),
                        false => row().pin(Some(side), Some(0.), None, None),
                    }
                    .h(pin)
                    .center_y()
                }
            };
            scrubber = scrubber.child(
                bubble
                    .key("bubble")
                    .live_offset(state.bubble.smooth(CATCH_UP))
                    .child(chip)
                    .a11y_hidden(),
            );
        }
        if let Some(name) = self.name {
            scrubber = scrubber.label(name);
        }
        if let Some(value) = self.value {
            scrubber = scrubber.a11y_value(value);
        }
        if !interactive {
            return scrubber.disabled(!self.enabled);
        }

        let hover_state = state.clone();
        let hover = self.on_hover.clone();
        scrubber = scrubber
            .focusable(true)
            .cursor(Cursor::Pointer)
            .range_step(f64::from(STEP))
            .on_hover_change(move |s: &mut S, hovered: bool, tx: &mut Tx| {
                hover_state.update(|g| g.hovered = hovered);
                match (&hover, hovered) {
                    (Some(hover), false) => hover(s, None, tx),
                    _ => tx.live_only(),
                }
            });
        if let Some(hover) = self.on_hover {
            let pointed = state.clone();
            scrubber =
                scrubber.on_pointer_move_local(move |s: &mut S, at: PointerEvent, tx: &mut Tx| {
                    hover(s, Some(pointed.fraction_at(at.x, at.y)), tx)
                });
        }
        let on_move = self.on_move.clone();
        let on_release = self.on_release.clone();
        let dragged = state.clone();
        scrubber = scrubber.on_drag(move |s: &mut S, drag: &mut Drag, tx: &mut Tx| {
            tx.instant();
            let fraction = dragged.fraction_at(drag.x, drag.y);
            match drag.phase {
                Phase::Down | Phase::Move => {
                    dragged.set(fraction);
                    if let Some(on_move) = &on_move {
                        on_move(s, fraction, tx);
                    }
                }
                Phase::Up => {
                    if let Some(on_release) = &on_release {
                        on_release(s, tx);
                    }
                }
            }
        });
        let on_move = self.on_move;
        let on_release = self.on_release;
        let stepped = state;
        scrubber.on_step(move |s: &mut S, steps: i32, tx: &mut Tx| {
            let fraction = (stepped.fraction() + steps as f32 * STEP).clamp(0., 1.);
            stepped.set(fraction);
            if let Some(on_move) = &on_move {
                on_move(s, fraction, tx);
            }
            if let Some(on_release) = &on_release {
                on_release(s, tx);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: f32 = 0.;
    const WIDTH: f32 = 300.;
    const PIN: f32 = 12.;

    fn measured() -> Geometry {
        Geometry {
            size: [WIDTH, 24.],
            line: 4.,
            pin: PIN,
            enabled: true,
            ..Geometry::default()
        }
    }

    #[test]
    fn the_thumb_lands_under_the_pointer() {
        let geometry = measured();
        let travel = WIDTH - PIN;
        for step in 0..=100 {
            let fraction = step as f32 / 100.;
            let center = ORIGIN + PIN / 2. + travel * fraction;
            let read = at(&geometry, center, 0.);
            assert!((read - fraction).abs() < 1e-4, "{fraction} read as {read}");
            let drawn = rects(&Geometry {
                fraction,
                ..geometry
            });
            let middle = drawn.thumb[0] + drawn.thumb[2] / 2.;
            assert!(
                (middle - center).abs() < 1e-3,
                "{fraction} drawn at {middle}"
            );
        }
    }

    #[test]
    fn the_ends_clamp() {
        let geometry = measured();
        assert_eq!(at(&geometry, -50., 0.), 0.);
        assert_eq!(at(&geometry, 0., 0.), 0.);
        assert_eq!(at(&geometry, WIDTH, 0.), 1.);
        assert_eq!(at(&geometry, 9999., 0.), 1.);
    }

    #[test]
    fn an_unmeasured_track_reads_zero() {
        assert_eq!(at(&Geometry::default(), 50., 0.), 0.);
    }

    #[test]
    fn hovering_thickens_the_track() {
        let rest = rects(&measured());
        let hovered = rects(&Geometry {
            hovered: true,
            ..measured()
        });
        assert!(hovered.track[3] > rest.track[3]);
        assert!(hovered.thumb[2] > rest.thumb[2]);
    }
}
