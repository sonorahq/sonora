use std::cell::Cell;
use std::f32::consts::TAU;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant;

use gpui::prelude::*;
use gpui::{
    App, Bounds, Div, EntityId, Hsla, PathBuilder, Pixels, Point, SharedString, Window, canvas,
    div, point, px,
};
use ui::{Artwork, Levels, Theme, VisualizerStyle};

/// One turn of the record, in seconds. A real 33⅓ single spins in 1.8, which
/// at this size reads as frantic; the stage settles for a slow, visible turn.
const SPIN: f32 = 18.;
/// The record's diameter, as a share of the square the stage is given. The
/// rest of the square is the room the ring, the halo and the particles live in,
/// and it has to end inside the square: the raster layer clips at its edge.
const DISC: f32 = 0.78;
/// The cover's diameter, as a share of the record's: seven of cover to three of
/// vinyl, so the art reads as the label and the record as its rim.
const LABEL: f32 = 0.7;
/// Where the grooves run, as shares of the record's radius: outside the cover,
/// which is all of the record the eye can see.
const GROOVE_IN: f32 = 0.72;
const GROOVE_OUT: f32 = 0.985;
/// The ring: how far its bars start off the rim, and how far a full band
/// reaches, as shares of the stage's side.
const RING_GAP: f32 = 0.025;
const RING_REACH: f32 = 0.075;
/// The lowest band stands this tall, so a silent track still shows a hairline.
const RING_FLOOR: f32 = 0.015;
/// How wide the halo's reach is past the rim, as a share of the side. It has
/// to fade out before the stage's own edge or the layer clips it mid-glow.
const HALO_REACH: f32 = 0.10;
/// How many concentric discs fake the halo's radial falloff.
const HALO: usize = 12;
/// How many grooves the record wears.
const GROOVES: usize = 20;
/// How bright a particle gets at the top of its drift, and how many shades that
/// brightness is rounded into. Every particle of one shade shares a path, so
/// the field costs one draw per shade and not one per particle.
const STAR: f32 = 0.52;
const TONES: usize = 8;
/// The sheen: one pass's angular width in radians, how bright it gets, and how
/// many cells a pass is subdivided into so its edges fade.
const SHEEN_SPAN: f32 = 0.72;
const SHEEN: f32 = 0.05;
const SHEEN_CELLS: usize = 10;
/// Short bright arcs on the grooves, rotating with the sheen. On a texture of
/// perfect circles they are the only cue that the record itself is turning.
/// They ride between the cover and the rim, the only vinyl there is to see.
const MARKERS: [f32; 3] = [0.76, 0.84, 0.92];
const MARKER_SPAN: f32 = 0.26;
/// How many straight segments stand in for a smooth circle.
const SEGMENTS: usize = 72;
/// The sleeve: the cover's edge and the record's diameter as shares of the
/// stage's side, how far the record's centre sits past the cover's right edge,
/// where the cover starts, and the label's diameter as a share of the
/// record's. Together they leave the record peeking out to the right with
/// about half of it showing, the way a record slid out of its sleeve sits in
/// the hand.
const SLEEVE: f32 = 0.64;
const SLEEVE_DISC: f32 = 0.52;
const SLEEVE_CX: f32 = 0.68;
const SLEEVE_X: f32 = 0.025;
const SLEEVE_LABEL: f32 = 0.52;
/// The sleeve's grooves, as shares of the record's radius: they run outside
/// the label and stop short of the rim.
const SLEEVE_GROOVES: usize = 6;
const SLEEVE_GROOVE_IN: f32 = 0.58;
const SLEEVE_GROOVE_OUT: f32 = 0.94;

/// Where the stage's clock started, so the angle carries over between visits
/// to fullscreen instead of resetting to the same pose every time.
pub(crate) fn spin() -> f32 {
    static STARTED: OnceLock<Instant> = OnceLock::new();
    STARTED.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// The frames the stage's own motion needs: the turn of the record, the sweep
/// of the sheen, the drift of the particles. None of it depends on sound being
/// made, and once playback stops nothing else in fullscreen asks for frames at
/// all — the spectrum drive parks and the view goes quiet — so a stage that
/// rode along with playback would freeze mid-turn the moment it was paused.
/// This asks for frames on the stage's behalf, from inside the frame it was
/// last given, and wakes the view to redraw. Call it every render; it only
/// ever has one loop running, and it stops the moment `live` goes false.
/// How many frames the loop may run on without the view having rendered. A
/// render refreshes the allowance, so while the stage is on screen the loop
/// never runs dry; once the view stops asking — it was closed, or the stage
/// was switched off — it winds down on its own instead of spinning forever on
/// a view nobody is watching.
const GRACE: u32 = 3;

#[derive(Clone)]
pub(crate) struct Drive {
    armed: Rc<Cell<bool>>,
    beats: Rc<Cell<u32>>,
}

impl Default for Drive {
    fn default() -> Self {
        Self {
            armed: Rc::new(Cell::new(false)),
            beats: Rc::new(Cell::new(0)),
        }
    }
}

impl Drive {
    pub(crate) fn run(&self, watch: EntityId, live: bool, window: &mut Window) {
        self.beats.set(match live {
            true => GRACE,
            false => 0,
        });
        if !live || self.armed.replace(true) {
            return;
        }
        let drive = self.clone();
        window.on_next_frame(move |window, cx| drive.step(watch, window, cx));
    }

    fn step(&self, watch: EntityId, window: &mut Window, cx: &mut App) {
        let beats = self.beats.get();
        if beats == 0 {
            self.armed.set(false);
            return;
        }
        self.beats.set(beats - 1);
        cx.notify(watch);
        let drive = self.clone();
        window.on_next_frame(move |window, cx| drive.step(watch, window, cx));
    }
}

/// How long the particle field takes to fade in or out, as a time constant:
/// it covers about two thirds of the way in this long, and is all but there
/// after three times as much. Not a share per frame, so it lasts the same on
/// any display. Raise it for a slower drift in and out.
const FADE: f32 = 1.6;

/// What the clock hands back for one frame.
pub(crate) struct Pose {
    /// Seconds the music has been playing: the record's turn.
    pub(crate) turn: f32,
    /// How much of the particle field is there, from none of it to all of it.
    /// It eases out once the music stops and back in once it starts — the
    /// drift belongs to the sound, not to the window being open.
    pub(crate) presence: f32,
}

/// How long the record has been turning, and nothing else: the clock only
/// banks time while sound is being made, so pausing parks it mid-turn — no
/// rewind, no coasting — and resuming picks the turn up from where it stood.
/// Call it once per render; it remembers when it was last called and adds the
/// gap only when `playing`. The cap keeps a long stall — a suspended window,
/// a hitch in the loop — from lurching the record forward.
#[derive(Default)]
pub(crate) struct Clock {
    played: Cell<f32>,
    presence: Cell<f32>,
    last: Cell<Option<Instant>>,
}

impl Clock {
    pub(crate) fn tick(&self, playing: bool) -> Pose {
        let now = Instant::now();
        if let Some(last) = self.last.replace(Some(now)) {
            let gone = now.duration_since(last).as_secs_f32().min(0.1);
            if playing {
                self.played.set(self.played.get() + gone);
            }
            let target = match playing {
                true => 1.,
                false => 0.,
            };
            let rate = 1. - (-gone / FADE).exp();
            let presence = self.presence.get();
            self.presence.set(presence + (target - presence) * rate);
        }
        Pose {
            turn: self.played.get(),
            presence: self.presence.get(),
        }
    }

    /// Whether anything on the stage still moves: the record turning, or the
    /// field still fading. Once a paused stage has gone still there is nothing
    /// left to draw, and the frame loop may wind down.
    pub(crate) fn moving(&self, playing: bool) -> bool {
        playing || self.presence.get() > 0.01
    }
}

/// What the stage is asked to show: which way the cover is staged, the
/// spectrum the ring reads and the shape it reads them in, how much drifts off
/// the rim, how far the record has turned and for how long, and the theme the
/// stage dresses in. Two clocks on purpose: the record and everything riding
/// it turn only while sound plays, while the particles keep to the wall clock
/// whether or not it does.
pub(crate) struct Stage {
    pub(crate) levels: Levels,
    pub(crate) style: VisualizerStyle,
    pub(crate) particles: usize,
    /// The way the cover is staged: bare, starry, on a record, in its sleeve.
    pub(crate) layout: ui::StageStyle,
    /// Seconds on the wall clock, driving the particles' drift.
    pub(crate) elapsed: f32,
    /// Seconds the music has been playing, driving the record's turn.
    pub(crate) turn: f32,
    /// How much of the particle field is present, easing with the music.
    pub(crate) presence: f32,
    pub(crate) theme: Theme,
}

/// Everything on the stage that moves, and by how much. The record turns on
/// the music's own clock; the field is as present as the music has left it;
/// the particles drift on the wall clock, whatever the music is doing, and
/// fade with the field.
#[derive(Clone, Copy)]
struct Motion {
    /// Seconds the music has been playing.
    turn: f32,
    /// How much of the particle field is there, from none to all of it.
    presence: f32,
    /// Seconds since the stage was first drawn.
    elapsed: f32,
}

/// The whole stage, sized to `side`. The sleeve stages the cover on bare
/// paint with the record behind it; the rest halo the cover and read the
/// spectrum, staging the record with everything that sweeps across it only
/// when a record is asked for.
pub(crate) fn stage(
    side: Pixels,
    label: Option<impl Into<SharedString>>,
    waiting: bool,
    stage: Stage,
) -> Div {
    let Stage {
        levels,
        style,
        particles,
        layout,
        elapsed,
        turn,
        presence,
        theme,
    } = stage;
    let paint = palette(&theme);
    let side = side.as_f32();
    // Resolved once so both the sleeve's cover and its record label can carry
    // the same art.
    let label: Option<SharedString> = label.map(Into::into);

    // The sleeve is a different composition: the cover square with the record
    // peeking out behind it, no field around either.
    if layout == ui::StageStyle::Sleeve {
        return sleeve(side, label, waiting, turn, &paint, theme.radius * 2.);
    }
    let vinyl = layout.turned();

    div()
        .relative()
        .size(px(side))
        .child(
            canvas(move |_, _, _| {}, {
                let levels = levels.clone();
                move |bounds, _, window, _| field(bounds, &levels, style, vinyl, &paint, window)
            })
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child({
                    // Seven of cover to three of vinyl, with the record or
                    // without it: the stage's size is not the record's to give.
                    let art = Artwork::new(label).size(px(side * DISC * LABEL));
                    // The cover rides the record: the renderer cannot turn an
                    // image, so the turn is cut from its pixels and handed back
                    // as a frame of its own, once per degree.
                    match vinyl {
                        true => art.spin(turn / SPIN).circle().soft(waiting),
                        false => art.circle().soft(waiting),
                    }
                }),
        )
        .child(
            canvas(move |_, _, _| {}, {
                let motion = Motion {
                    turn,
                    presence,
                    elapsed,
                };
                move |bounds, _, window, _| shine(bounds, motion, particles, vinyl, &paint, window)
            })
            .absolute()
            .inset_0(),
        )
}

/// The sleeve layout: the cover slid most of the way out of its sleeve — that
/// is, off the record behind it — so the record peeks out to the right with
/// the cover's own art turning on it as its label. The cover itself stays
/// square and still; the turn belongs to the record and its label.
fn sleeve(
    side: f32,
    label: Option<SharedString>,
    waiting: bool,
    turn: f32,
    paint: &Palette,
    radius: Pixels,
) -> Div {
    let cover = side * SLEEVE;
    let disc = side * SLEEVE_DISC;
    let center_x = side * SLEEVE_CX;
    let center_y = side / 2.;
    let label_side = disc * SLEEVE_LABEL;

    div()
        .relative()
        .size(px(side))
        .child(
            canvas(move |_, _, _| {}, {
                let paint = *paint;
                move |bounds, _, window, _| platter(bounds, turn, &paint, window)
            })
            .absolute()
            .inset_0(),
        )
        .child(
            div()
                .absolute()
                .left(px(center_x - label_side / 2.))
                .top(px(center_y - label_side / 2.))
                // The cover's own art rides the record as its label, turning
                // with it the way a record label does.
                .child(
                    Artwork::new(label.clone())
                        .size(px(label_side))
                        .spin(turn / SPIN)
                        .circle()
                        .soft(waiting),
                ),
        )
        .child(
            div()
                .absolute()
                .left(px(side * SLEEVE_X))
                .top(px((side - cover) / 2.))
                .child(
                    Artwork::new(label)
                        .size(px(cover))
                        .corner_radius(radius)
                        .soft(waiting),
                ),
        )
}

/// Every colour the stage paints with, resolved once from the theme.
#[derive(Clone, Copy)]
struct Palette {
    halo: Hsla,
    disc: Hsla,
    groove: Hsla,
    groove_dark: Hsla,
    rim: Hsla,
    ring: Hsla,
    star: Hsla,
    sheen: Hsla,
    marker: Hsla,
    label_edge: Hsla,
    /// The sleeve's record: a pastel pressing in the theme's own hue.
    record: Hsla,
    record_groove: Hsla,
    record_edge: Hsla,
    hole: Hsla,
}

/// The record dresses in the theme's own hue at a whisper: a dark disc with a
/// trace of it, lighter and darker grooves over it, and everything that glows
/// — halo, ring, particles — in the hue itself. On a light theme the disc cuts
/// the hue deeper and the glows darken, so the stage sits with the adaptive
/// theme instead of washing out against a bright background.
fn palette(theme: &Theme) -> Palette {
    let hue = theme.primary;
    let dark = theme.background.l < 0.5;
    let neutral = |l: f32, a: f32| Hsla {
        h: hue.h,
        s: 0.08,
        l,
        a,
    };

    let disc = Hsla {
        h: hue.h,
        s: match dark {
            true => (hue.s * 0.25).clamp(0.02, 0.28),
            false => (hue.s * 0.5).clamp(0.08, 0.4),
        },
        l: match dark {
            true => 0.115,
            false => 0.24,
        },
        a: 1.,
    };
    let glow = Hsla {
        l: match dark {
            true => hue.l.clamp(0.42, 0.72),
            false => hue.l.clamp(0.32, 0.52),
        },
        ..hue
    };
    // The sleeve's record: a pastel pressing in the theme's own hue, lighter
    // than the vinyl the starry stage paints so it reads against bare paint.
    let record = Hsla {
        s: (hue.s * 0.9).clamp(0.12, 0.5),
        l: match dark {
            true => 0.72,
            false => 0.58,
        },
        ..hue
    };

    Palette {
        halo: Hsla {
            s: (hue.s * 0.9).clamp(0.15, 0.7),
            l: match dark {
                true => (hue.l * 0.95).clamp(0.25, 0.6),
                false => hue.l.clamp(0.35, 0.55),
            },
            ..hue
        },
        disc,
        groove: Hsla {
            s: disc.s * 0.4,
            l: disc.l + 0.075,
            a: 0.5,
            ..disc
        },
        groove_dark: Hsla {
            s: disc.s * 0.4,
            l: disc.l - 0.065,
            a: 0.55,
            ..disc
        },
        rim: Hsla {
            l: disc.l + 0.2,
            a: 0.45,
            ..disc
        },
        ring: glow,
        // The particles carry the theme's own colour, not a neutral one: what
        // drifts off the rim should read as part of the same scheme as the disc
        // and the ring. On a light theme they take the hue deeper so they stay
        // held against the background instead of bleaching out.
        star: Hsla {
            s: hue.s.clamp(0.25, 0.8),
            l: match dark {
                true => (hue.l + 0.3).clamp(0.55, 0.82),
                false => (hue.l - 0.18).clamp(0.3, 0.5),
            },
            ..hue
        },
        sheen: neutral(0.9, SHEEN),
        marker: neutral(0.8, 0.06),
        label_edge: neutral(
            match dark {
                true => 0.04,
                false => 0.08,
            },
            0.35,
        ),
        record,
        record_groove: Hsla {
            l: record.l + 0.06,
            a: 0.7,
            ..record
        },
        record_edge: Hsla {
            l: record.l - 0.10,
            a: 0.6,
            ..record
        },
        hole: neutral(
            match dark {
                true => 0.10,
                false => 0.22,
            },
            1.,
        ),
    }
}

/// The field under the label: halo, spectrum ring and, when the record is
/// staged, its face and grooves.
fn field(
    bounds: Bounds<Pixels>,
    levels: &Levels,
    style: VisualizerStyle,
    vinyl: bool,
    paint: &Palette,
    window: &mut Window,
) {
    let Some(center) = center(bounds) else {
        return;
    };
    let side = bounds.size.width.min(bounds.size.height).as_f32();
    let radius = side * DISC / 2.;

    // The halo: concentric discs whose opacity falls off quadratically, the
    // same trick the ambient field uses for its blobs. It is what makes the
    // stage read as diffuse rather than as a disc on flat paint.
    for step in 0..HALO {
        let u = step as f32 / HALO as f32;
        let reach = radius + side * HALO_REACH * u;
        let alpha = (1. - u) * (1. - u) * 0.10;
        let mut builder = PathBuilder::fill();
        circle(&mut builder, center, reach);
        match builder.build() {
            Ok(path) => window.paint_path(path, paint.halo.opacity(alpha)),
            Err(error) => log::warn!("starry: cannot build the halo: {error}"),
        }
    }

    if style.shown() {
        ring(center, side, radius, levels, style, paint, window);
    }
    if vinyl {
        record(center, radius, paint, window);
    }
}

/// The ring around the rim, in whatever shape the visualizer is set to: bars
/// stand in a mirrored circle, the wave runs one smooth loop, and `Both` puts
/// the bars inside the wave, cut to its height.
fn ring(
    center: Point<Pixels>,
    side: f32,
    radius: f32,
    levels: &Levels,
    style: VisualizerStyle,
    paint: &Palette,
    window: &mut Window,
) {
    let bands = levels.mixed();
    if bands.is_empty() {
        return;
    }
    let inner = radius + side * RING_GAP;
    let reach = side * RING_REACH;

    match style {
        VisualizerStyle::None => {}
        VisualizerStyle::Bars => bars(center, inner, reach, &bands, |_| f32::MAX, paint, window),
        VisualizerStyle::Wave => wave(center, inner, reach, &bands, paint, window),
        VisualizerStyle::Both => {
            wave(center, inner, reach, &bands, paint, window);
            // Under the wave: a bar standing proud of it is cut back to the
            // wave's own height at that angle, so the two read as one shape
            // with the wave as its edge.
            bars(
                center,
                inner,
                reach,
                &bands,
                |angle| wave_radius(inner, reach, &bands, angle),
                paint,
                window,
            );
        }
    }
}

/// The wave's own radius at `angle`, walked along the same cubics `wave` draws
/// with. The bars stand between the wave's samples, where a spline dips below
/// the points it passes through, so their own heights are not enough to keep
/// them in — they have to be cut to the curve itself.
fn wave_radius(inner: f32, reach: f32, bands: &[f32], angle: f32) -> f32 {
    let count = bands.len();
    let start = -TAU / 4.;
    let step = TAU / count as f32;
    let at = |index: i64| {
        let band = bands[index.rem_euclid(count as i64) as usize];
        let level = RING_FLOOR + band.clamp(0., 1.) * (1. - RING_FLOOR);
        let turned = start + step * (index as f32 + 0.5);
        let radius = inner + reach * level;
        (radius * turned.cos(), radius * turned.sin())
    };

    let walked = (angle - start) / step;
    let segment = walked.floor();
    let t = (walked - segment).clamp(0., 1.);
    let index = segment as i64;
    let (x0, y0) = at(index);
    let (x1, y1) = at(index + 1);
    let (bx, by) = at(index - 1);
    let (ax, ay) = at(index + 2);

    let (c1x, c1y) = (x0 + (x1 - bx) / 6., y0 + (y1 - by) / 6.);
    let (c2x, c2y) = (x1 - (ax - x0) / 6., y1 - (ay - y0) / 6.);
    let u = 1. - t;
    let x = x0 * (u * u * u) + c1x * (3. * u * u * t) + c2x * (3. * u * t * t) + x1 * (t * t * t);
    let y = y0 * (u * u * u) + c1y * (3. * u * u * t) + c2y * (3. * u * t * t) + y1 * (t * t * t);
    (x * x + y * y).sqrt()
}

/// The bars: the mixed bands walked twice around the rim, once and then
/// mirrored, so the circle reads as symmetric. All bars share one stroked
/// path, one subpath each. `ceiling` is the furthest a bar may reach at a
/// given angle, which is what keeps them under the wave when both are drawn.
fn bars(
    center: Point<Pixels>,
    inner: f32,
    reach: f32,
    bands: &[f32],
    ceiling: impl Fn(f32) -> f32,
    paint: &Palette,
    window: &mut Window,
) {
    let count = bands.len();
    let posts = 2 * count;
    let width = (TAU * inner / posts as f32 * 0.5).max(1.2);
    let mut builder = PathBuilder::stroke(px(width));
    for post in 0..posts {
        let band = match post < count {
            true => bands[post],
            false => bands[posts - 1 - post],
        };
        let level = RING_FLOOR + band.clamp(0., 1.) * (1. - RING_FLOOR);
        let angle = -TAU / 4. + TAU * (post as f32 + 0.5) / posts as f32;
        // A hair under the ceiling, so the wave's stroke reads as the edge and
        // the bars as what fills it.
        let tip = (inner + reach * level).min(ceiling(angle) - 0.5);
        builder.move_to(on(center, inner, angle));
        builder.line_to(on(center, tip, angle));
    }
    match builder.build() {
        Ok(path) => window.paint_path(path, paint.ring.opacity(0.55)),
        Err(error) => log::warn!("starry: cannot build the ring: {error}"),
    }
}

/// The wave: one smooth loop through every band, a closed Catmull-Rom spline —
/// the same spline the bottom wave runs — bent around the rim.
fn wave(
    center: Point<Pixels>,
    inner: f32,
    reach: f32,
    bands: &[f32],
    paint: &Palette,
    window: &mut Window,
) {
    let mut points = Vec::with_capacity(bands.len());
    for (index, band) in bands.iter().enumerate() {
        let level = RING_FLOOR + band.clamp(0., 1.) * (1. - RING_FLOOR);
        let angle = -TAU / 4. + TAU * (index as f32 + 0.5) / bands.len() as f32;
        points.push(on(center, inner + reach * level, angle));
    }
    let mut builder = PathBuilder::stroke(px(1.5));
    wave_ring(&mut builder, &points);
    match builder.build() {
        Ok(path) => window.paint_path(path, paint.ring.opacity(0.55)),
        Err(error) => log::warn!("starry: cannot build the wave: {error}"),
    }
}

/// Appends a closed Catmull-Rom spline through `points`, as cubics.
fn wave_ring(builder: &mut PathBuilder, points: &[Point<Pixels>]) {
    let count = points.len();
    if count < 2 {
        return;
    }
    builder.move_to(points[0]);
    for index in 0..count {
        let from = points[index];
        let to = points[(index + 1) % count];
        let before = points[(index + count - 1) % count];
        let after = points[(index + 2) % count];
        builder.cubic_bezier_to(
            to,
            point(
                from.x + (to.x - before.x) / 6.,
                from.y + (to.y - before.y) / 6.,
            ),
            point(
                to.x - (after.x - from.x) / 6.,
                to.y - (after.y - from.y) / 6.,
            ),
        );
    }
    builder.close();
}

/// The record itself: face, grooves and rim. The grooves alternate two tones
/// so the surface has texture even where the sheen isn't passing.
fn record(center: Point<Pixels>, radius: f32, paint: &Palette, window: &mut Window) {
    let mut face = PathBuilder::fill();
    circle(&mut face, center, radius);
    match face.build() {
        Ok(path) => window.paint_path(path, paint.disc),
        Err(error) => log::warn!("starry: cannot build the record: {error}"),
    }

    let mut light = PathBuilder::stroke(px(1.));
    let mut dark = PathBuilder::stroke(px(1.));
    for groove in 0..GROOVES {
        let t = GROOVE_IN + (GROOVE_OUT - GROOVE_IN) * groove as f32 / (GROOVES - 1) as f32;
        match groove % 2 == 0 {
            true => circle(&mut light, center, radius * t),
            false => circle(&mut dark, center, radius * t),
        }
    }
    circle(&mut light, center, radius * 0.995);
    match light.build() {
        Ok(path) => window.paint_path(path, paint.groove),
        Err(error) => log::warn!("starry: cannot build the grooves: {error}"),
    }
    match dark.build() {
        Ok(path) => window.paint_path(path, paint.groove_dark),
        Err(error) => log::warn!("starry: cannot build the grooves: {error}"),
    }

    let mut rim = PathBuilder::stroke(px(1.5));
    circle(&mut rim, center, radius - 0.75);
    match rim.build() {
        Ok(path) => window.paint_path(path, paint.rim),
        Err(error) => log::warn!("starry: cannot build the rim: {error}"),
    }
}

/// The sleeve's record, as a pastel pressing: face, grooves, a sheen riding
/// the turn, and the spindle hole at its centre. The label — the cover's own
/// art, turning — is an element laid over this, not painted here.
fn platter(bounds: Bounds<Pixels>, turn: f32, paint: &Palette, window: &mut Window) {
    let side = bounds.size.width.min(bounds.size.height).as_f32();
    // Painted paths read in window coordinates, so the record's centre is the
    // canvas origin plus its share of the stage's side.
    let center = bounds.origin + point(px(side * SLEEVE_CX), px(side / 2.));
    let radius = side * SLEEVE_DISC / 2.;
    let angle = TAU * turn / SPIN;

    let mut face = PathBuilder::fill();
    circle(&mut face, center, radius);
    match face.build() {
        Ok(path) => window.paint_path(path, paint.record),
        Err(error) => log::warn!("starry: cannot build the platter: {error}"),
    }

    let mut grooves = PathBuilder::stroke(px(1.));
    for groove in 0..SLEEVE_GROOVES {
        let t = SLEEVE_GROOVE_IN
            + (SLEEVE_GROOVE_OUT - SLEEVE_GROOVE_IN) * groove as f32 / (SLEEVE_GROOVES - 1) as f32;
        circle(&mut grooves, center, radius * t);
    }
    match grooves.build() {
        Ok(path) => window.paint_path(path, paint.record_groove),
        Err(error) => log::warn!("starry: cannot build the platter's grooves: {error}"),
    }

    let mut rim = PathBuilder::stroke(px(1.5));
    circle(&mut rim, center, radius - 0.75);
    match rim.build() {
        Ok(path) => window.paint_path(path, paint.record_edge),
        Err(error) => log::warn!("starry: cannot build the platter's rim: {error}"),
    }

    // A short sheen riding the rim with the turn, the only cue on an even
    // pastel face that the record is moving at all.
    let mut sheen = PathBuilder::stroke(px(2.));
    arc(
        &mut sheen,
        center,
        radius * 0.97,
        angle + 0.4,
        angle + 0.4 + MARKER_SPAN,
    );
    match sheen.build() {
        Ok(path) => window.paint_path(path, paint.record_groove),
        Err(error) => log::warn!("starry: cannot build the platter's sheen: {error}"),
    }

    let mut pin = PathBuilder::fill();
    circle(&mut pin, center, (side * 0.008).max(2.5));
    match pin.build() {
        Ok(path) => window.paint_path(path, paint.hole),
        Err(error) => log::warn!("starry: cannot build the spindle hole: {error}"),
    }
    let mut pin_rim = PathBuilder::stroke(px(1.));
    circle(&mut pin_rim, center, (side * 0.008).max(2.5) + 1.2);
    match pin_rim.build() {
        Ok(path) => window.paint_path(path, paint.record_edge),
        Err(error) => log::warn!("starry: cannot build the spindle hole's rim: {error}"),
    }
}

/// Everything that sweeps over the label: the sheen and its groove markers
/// turning only while the music does, the particles as present as the music
/// has left them, and a hairline at the cover's edge so the two read as
/// separate surfaces. With y pointing down, a growing angle already turns
/// clockwise on screen.
fn shine(
    bounds: Bounds<Pixels>,
    motion: Motion,
    particles: usize,
    vinyl: bool,
    paint: &Palette,
    window: &mut Window,
) {
    let Motion {
        turn,
        presence,
        elapsed,
    } = motion;
    let Some(center) = center(bounds) else {
        return;
    };
    let side = bounds.size.width.min(bounds.size.height).as_f32();
    let radius = side * DISC / 2.;

    if vinyl {
        let angle = TAU * turn / SPIN;

        // Two sheen passes, opposite each other, each a fan of cells whose
        // opacity rises to a peak in the middle and falls off at both edges.
        // The fan is what rotates; the cells only keep its ends soft.
        for pass in 0..2 {
            let base = angle + pass as f32 * std::f32::consts::PI;
            for cell in 0..SHEEN_CELLS {
                let from = base + SHEEN_SPAN * cell as f32 / SHEEN_CELLS as f32;
                let to = base + SHEEN_SPAN * (cell + 1) as f32 / SHEEN_CELLS as f32;
                let alpha =
                    SHEEN * (std::f32::consts::PI * (cell as f32 + 0.5) / SHEEN_CELLS as f32).sin();
                let mut builder = PathBuilder::fill();
                // From the cover's edge outwards: the sheen sweeps the vinyl,
                // not the art it carries.
                wedge(
                    &mut builder,
                    center,
                    radius * LABEL,
                    radius * 0.985,
                    from,
                    to,
                );
                match builder.build() {
                    Ok(path) => window.paint_path(path, paint.sheen.opacity(alpha)),
                    Err(error) => log::warn!("starry: cannot build the sheen: {error}"),
                }
            }
        }

        // Three short bright arcs riding the grooves at the sheen's angle.
        for marker in MARKERS {
            let mut builder = PathBuilder::stroke(px(1.2));
            arc(
                &mut builder,
                center,
                radius * marker,
                angle + 0.4,
                angle + 0.4 + MARKER_SPAN,
            );
            match builder.build() {
                Ok(path) => window.paint_path(path, paint.marker),
                Err(error) => log::warn!("starry: cannot build a marker: {error}"),
            }
        }
    }

    // The particles: each on its own cycle from the rim outward, fading in and
    // back out over the trip, scattered by a hash so the field is the same
    // every frame without a random source. They are gathered by how bright they
    // are and one path is drawn per shade, so a thousand of them cost a
    // handful of draws rather than a thousand. The whole field carries the
    // music's presence: it fades out as the music stops and back in as it
    // starts, and a field faded to nothing costs nothing at all.
    if presence > 0.01 {
        let mut shades: Vec<PathBuilder> = (0..TONES).map(|_| PathBuilder::fill()).collect();
        let mut counts = [0usize; TONES];
        for seed in 0..particles as u32 {
            let first = scatter(seed);
            let second = scatter(seed.wrapping_add(0x9E1));
            let third = scatter(seed.wrapping_add(0x37D));
            let period = 6. + first * 6.;
            let journey = (elapsed / period + second).fract();
            let distance = radius * (1.03 + 0.23 * journey.powf(0.8));
            // A slow orbit in the record's own direction, a little different
            // per particle, so the field circles rather than spins rigidly.
            let heading = -TAU / 4. + TAU * third + elapsed * TAU / SPIN * (0.06 + first * 0.12);
            let alpha = (std::f32::consts::PI * journey).sin() * (0.22 + second * 0.30) * presence;
            let size = side * 0.0025 + third * side * 0.003;
            let shade = ((alpha / STAR * TONES as f32) as usize).min(TONES - 1);
            counts[shade] += 1;
            disc(&mut shades[shade], on(center, distance, heading), size);
        }
        for (shade, builder) in shades.into_iter().enumerate() {
            if counts[shade] == 0 {
                continue;
            }
            let alpha = (shade as f32 + 0.5) / TONES as f32 * STAR;
            match builder.build() {
                Ok(path) => window.paint_path(path, paint.star.opacity(alpha)),
                Err(error) => log::warn!("starry: cannot build the particles: {error}"),
            }
        }
    }

    // A hairline pressing the label's edge into the record so the two read as
    // separate surfaces. There is no spindle: the cover covers seven tenths of
    // the record and sits over the hole, the way a real label does.
    if vinyl {
        let mut edge = PathBuilder::stroke(px(1.));
        circle(&mut edge, center, radius * LABEL + 0.5);
        match edge.build() {
            Ok(path) => window.paint_path(path, paint.label_edge),
            Err(error) => log::warn!("starry: cannot build the label edge: {error}"),
        }
    }
}

/// The stage's centre, or `None` while the canvas is still empty.
fn center(bounds: Bounds<Pixels>) -> Option<Point<Pixels>> {
    let side = bounds.size.width.min(bounds.size.height);
    match side <= px(1.) {
        true => None,
        false => Some(point(
            bounds.origin.x + bounds.size.width / 2.,
            bounds.origin.y + bounds.size.height / 2.,
        )),
    }
}

/// The point at `angle` around `center`, at `radius` in logical pixels.
fn on(center: Point<Pixels>, radius: f32, angle: f32) -> Point<Pixels> {
    point(
        center.x + px(radius * angle.cos()),
        center.y + px(radius * angle.sin()),
    )
}

/// Appends a full circle as a closed polygon of `SEGMENTS` straight segments.
fn circle(builder: &mut PathBuilder, center: Point<Pixels>, radius: f32) {
    let step = TAU / SEGMENTS as f32;
    for segment in 0..=SEGMENTS {
        let at = on(center, radius, segment as f32 * step);
        match segment {
            0 => builder.move_to(at),
            _ => builder.line_to(at),
        }
    }
    builder.close();
}

/// Appends a disc whose own size decides how many segments it takes. A particle
/// is a few pixels across and does not need the seventy-two a record does, and
/// a thousand of them share a path.
fn disc(builder: &mut PathBuilder, center: Point<Pixels>, radius: f32) {
    let cells = ((radius * 0.8).ceil() as usize).clamp(6, SEGMENTS);
    let step = TAU / cells as f32;
    for cell in 0..=cells {
        let at = on(center, radius, cell as f32 * step);
        match cell {
            0 => builder.move_to(at),
            _ => builder.line_to(at),
        }
    }
    builder.close();
}

/// Appends a curved run from `from` to `to`, subdivided finely enough that the
/// chords never read as corners at these radii.
fn arc(builder: &mut PathBuilder, center: Point<Pixels>, radius: f32, from: f32, to: f32) {
    let cells = ((to - from).abs() / 0.06).ceil().max(3.) as usize;
    for cell in 0..=cells {
        let at = on(
            center,
            radius,
            from + (to - from) * cell as f32 / cells as f32,
        );
        match cell {
            0 => builder.move_to(at),
            _ => builder.line_to(at),
        }
    }
}

/// Appends an annular sector — the sheen's cell — between two radii and two
/// angles, both ends curved.
fn wedge(
    builder: &mut PathBuilder,
    center: Point<Pixels>,
    inner: f32,
    outer: f32,
    from: f32,
    to: f32,
) {
    let cells = ((to - from).abs() / 0.06).ceil().max(3.) as usize;
    builder.move_to(on(center, inner, from));
    builder.line_to(on(center, outer, from));
    for cell in 1..=cells {
        builder.line_to(on(
            center,
            outer,
            from + (to - from) * cell as f32 / cells as f32,
        ));
    }
    builder.line_to(on(center, inner, to));
    builder.line_to(on(center, inner, from));
    builder.close();
}

/// A stable pseudo-random fraction of the seed, in 0..1. The constants are the
/// usual integer-mixing suspects; only that the output is spread evenly and
/// repeatable matters here.
fn scatter(seed: u32) -> f32 {
    let mut mixed = seed.wrapping_mul(0x9E37_79B9).wrapping_add(0x85EB_CA6B);
    mixed ^= mixed >> 13;
    mixed = mixed.wrapping_mul(0xC2B2_AE35);
    mixed ^= mixed >> 16;
    (mixed >> 8) as f32 / 16_777_216.
}
