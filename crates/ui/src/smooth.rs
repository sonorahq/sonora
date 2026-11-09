//! Smooth scrolling for a notched mouse wheel. Rune moves a viewport by a whole wheel step at
//! once for line deltas (trackpads and free-spinning wheels already scroll point by point), so
//! each notch here becomes a glide toward where the wheel has asked to be, and notches that come
//! while one is gliding add to its target and keep its speed, as browsers scroll.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use rune::{El, ScrollPos, Spring, Tx, WheelEvent};

/// The spring a notch glides on: quick enough that a wheel spun hard keeps up, soft enough that
/// one notch reads as motion rather than a jump.
const GLIDE: Spring = Spring::new(0.26, 0.0);
/// How long after a notch the next one adds to its target rather than starting from where the
/// viewport is drawn.
const CHAIN: Duration = Duration::from_millis(260);

/// Where one viewport is and where its wheel glide is heading.
#[derive(Clone, Copy, Default)]
struct Track {
    at: [f32; 2],
    /// How far the viewport can scroll, once it has reported; Rune clamps a request itself,
    /// so before that the target is left unclamped.
    max: Option<[f32; 2]>,
    heading: Option<([f32; 2], Instant)>,
}

thread_local! {
    static TRACKS: RefCell<HashMap<String, Track>> = RefCell::new(HashMap::new());
    /// Glides asked for by notches since the last frame, started from the app's poll: a scroll
    /// request made inside a wheel handler doesn't reach Rune's viewports.
    static PENDING: RefCell<Vec<(String, [f32; 2])>> = const { RefCell::new(Vec::new()) };
}

/// Glides a scroll viewport on a notched wheel: `.smooth::<S>(key)` right after `.scroll(..)`,
/// with the viewport's own key.
pub trait Smooth {
    fn smooth<S: 'static>(self, key: &str) -> El;
}

impl Smooth for El {
    fn smooth<S: 'static>(self, key: &str) -> El {
        smooth::<S>(self, key)
    }
}

/// Makes a scroll viewport keyed `key` glide on a notched wheel. Call it on the element that
/// carries `.scroll(..)` (or `.scroll_x()`), with that element's key.
pub fn smooth<S: 'static>(viewport: El, key: &str) -> El {
    let followed = key.to_owned();
    let wheeled = key.to_owned();
    viewport
        .chain_scroll_offset(move |at: ScrollPos| {
            TRACKS.with_borrow_mut(|tracks| {
                let track = tracks.entry(followed.clone()).or_default();
                track.at = at.offset;
                track.max = Some(at.max);
            });
        })
        .on_wheel(move |_: &mut S, event: WheelEvent, tx: &mut Tx| glide(&wheeled, event, tx))
        .pixel_snap(false)
}

/// Turns one notch into a glide; leaves point-precise scrolling to Rune.
fn glide(key: &str, event: WheelEvent, tx: &mut Tx) -> bool {
    if event.precise || event.mods.cmd || event.mods.ctrl {
        return false;
    }
    let target = TRACKS.with_borrow_mut(|tracks| {
        let track = tracks.entry(key.to_owned()).or_default();
        let now = Instant::now();
        let from = match track.heading {
            Some((heading, at)) if now.duration_since(at) < CHAIN => heading,
            _ => track.at,
        };
        let max = track.max.unwrap_or([f32::MAX; 2]);
        let target = [
            (from[0] - event.dx).clamp(0., max[0].max(0.)),
            (from[1] - event.dy).clamp(0., max[1].max(0.)),
        ];
        track.heading = Some((target, now));
        target
    });
    PENDING.with_borrow_mut(|pending| {
        pending.retain(|(asked, _)| asked != key);
        pending.push((key.to_owned(), target));
    });
    tx.wake_after(Duration::ZERO);
    tx.live_only();
    true
}

/// Starts the glides the wheel asked for since the last frame. Call it from the app's `poll`,
/// which runs before every frame.
pub fn glide_wheels(tx: &mut Tx) {
    for (key, target) in PENDING.with_borrow_mut(std::mem::take) {
        tx.scroll_to_with(key, target[0], target[1], GLIDE);
    }
}
