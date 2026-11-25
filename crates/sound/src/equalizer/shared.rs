//! Carries the curve from its one writer to every output. The writer publishes into fixed
//! atomics behind a sequence counter, and a reader copies them out once per frame, trying again
//! when a write was under way, so the audio thread never waits, allocates or frees.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};

use super::{Curve, Filter, Kind, MAX_CHANNELS, SLOTS, Tone, Width};

/// How a curve change should reach the ear. A glide is a permission rather than an order, since
/// the reader replaces any move too big to glide, while a replace is never glided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Glide,
    Replace,
}

/// The one handle that changes the curve. It is not `Clone`, so there is exactly one writer,
/// which the sequence counter relies on.
pub struct EqualizerWriter {
    shared: Arc<Shared>,
    enabled: bool,
    curve: Curve,
    tone: Tone,
    /// The curve the outputs play, `curve` under `tone`.
    played: Curve,
    /// The filter each slot holds. An unchanged filter keeps its slot, so deleting, adding or
    /// reordering rows starts no transition in any other slot, and slot order is not heard.
    held: [Option<Filter>; SLOTS],
}

/// The audio thread's view of the curve, shared by every output. Cloning shares the same curve.
#[derive(Clone)]
pub struct Equalizer(Arc<Shared>);

/// What the reader copies out in one go.
#[derive(Clone, Copy)]
pub(crate) struct Snapshot {
    pub enabled: bool,
    /// Bumped by every replacing edit, so a reader that missed the edit under later glides still
    /// replaces.
    pub replaces: u32,
    /// The resolved preamp in decibels.
    pub preamp_db: f64,
    /// One per slot, an empty slot holding a filter that is off.
    pub filters: [Filter; SLOTS],
}

struct Shared {
    /// Odd while a write is under way.
    seq: AtomicU32,
    enabled: AtomicBool,
    replaces: AtomicU32,
    preamp: AtomicU64,
    slots: [Slot; SLOTS],
    /// Set by an output that met more channels than it has state for and passed them through.
    unprocessed: AtomicBool,
}

/// One filter's fields as atomics. `word` packs the kind, the width unit and whether it is on.
struct Slot {
    word: AtomicU64,
    frequency: AtomicU64,
    gain: AtomicU64,
    width: AtomicU64,
}

const ENABLED_BIT: u64 = 1 << 8;
const OCTAVES_BIT: u64 = 1 << 9;

impl EqualizerWriter {
    pub fn new(enabled: bool, curve: Curve) -> Self {
        let shared = Arc::new(Shared {
            seq: AtomicU32::new(0),
            enabled: AtomicBool::new(false),
            replaces: AtomicU32::new(0),
            preamp: AtomicU64::new(0f64.to_bits()),
            slots: std::array::from_fn(|_| Slot::default()),
            unprocessed: AtomicBool::new(false),
        });
        let mut writer = Self {
            shared,
            enabled,
            played: curve.toned(Tone::default()),
            curve,
            tone: Tone::default(),
            held: [None; SLOTS],
        };
        writer.place();
        writer.publish(Edit::Replace);
        writer
    }

    pub fn reader(&self) -> Equalizer {
        Equalizer(self.shared.clone())
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    pub fn tone(&self) -> Tone {
        self.tone
    }

    /// The curve the outputs play: the sound's own under the tone.
    pub fn played(&self) -> &Curve {
        &self.played
    }

    /// The preamp the played curve resolves to in decibels, as published to the outputs, so an
    /// automatic one is not worked out again.
    pub fn preamp_db(&self) -> f64 {
        f64::from_bits(self.shared.preamp.load(Ordering::Relaxed))
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        self.publish(Edit::Replace);
    }

    pub fn set_curve(&mut self, curve: Curve, edit: Edit) {
        if curve == self.curve {
            return;
        }
        self.curve = curve;
        self.played = self.curve.toned(self.tone);
        self.place();
        self.publish(edit);
    }

    /// Lays `tone` over the curve. A glide moves its shelves smoothly like a dragged filter.
    pub fn set_tone(&mut self, tone: Tone, edit: Edit) {
        if tone == self.tone {
            return;
        }
        self.tone = tone;
        self.played = self.curve.toned(tone);
        self.place();
        self.publish(edit);
    }

    /// Gives every filter of the played curve a slot. An unchanged filter stays where it is, so a
    /// filter edited in place keeps its slot and can glide.
    fn place(&mut self) {
        let mut placed = [None; SLOTS];
        let mut moved = Vec::new();
        for filter in self.played.filters() {
            let kept =
                (0..SLOTS).find(|&slot| placed[slot].is_none() && self.held[slot] == Some(*filter));
            match kept {
                Some(slot) => placed[slot] = Some(*filter),
                None => moved.push(*filter),
            }
        }
        let mut free: Vec<usize> = (0..SLOTS).filter(|&slot| placed[slot].is_none()).collect();
        free.sort_by_key(|&slot| self.held[slot].is_none());
        for (slot, filter) in free.into_iter().zip(moved) {
            placed[slot] = Some(filter);
        }
        self.held = placed;
    }

    fn publish(&self, edit: Edit) {
        let preamp = self.played.preamp_db();
        if self.shared.unprocessed.swap(false, Ordering::Relaxed) {
            log::warn!(
                "equalizer: a stream with more than {MAX_CHANNELS} channels played unequalized"
            );
        }
        let shared = &self.shared;
        shared.seq.fetch_add(1, Ordering::Relaxed);
        fence(Ordering::Release);
        shared.enabled.store(self.enabled, Ordering::Relaxed);
        shared.preamp.store(preamp.to_bits(), Ordering::Relaxed);
        for (slot, filter) in shared.slots.iter().zip(&self.held) {
            slot.store(filter.as_ref());
        }
        if edit == Edit::Replace {
            shared.replaces.fetch_add(1, Ordering::Relaxed);
        }
        shared.seq.fetch_add(1, Ordering::Release);
    }
}

impl Equalizer {
    /// The latest snapshot when it is newer than `seen`, else nothing. A write under way or
    /// one that lands during the copy yields nothing too, and the next frame tries again.
    pub(crate) fn poll(&self, seen: u32) -> Option<(u32, Snapshot)> {
        let shared = &self.0;
        let start = shared.seq.load(Ordering::Acquire);
        if start == seen || start & 1 == 1 {
            return None;
        }
        let snapshot = Snapshot {
            enabled: shared.enabled.load(Ordering::Relaxed),
            replaces: shared.replaces.load(Ordering::Relaxed),
            preamp_db: f64::from_bits(shared.preamp.load(Ordering::Relaxed)),
            filters: std::array::from_fn(|i| shared.slots[i].load()),
        };
        fence(Ordering::Acquire);
        (shared.seq.load(Ordering::Relaxed) == start).then_some((start, snapshot))
    }

    pub(crate) fn flag_unprocessed(&self) {
        self.0.unprocessed.store(true, Ordering::Relaxed);
    }
}

impl std::fmt::Debug for Equalizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Equalizer")
            .field("enabled", &self.0.enabled.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Slot {
    /// Stores `filter`, or an empty slot as a filter that is off.
    fn store(&self, filter: Option<&Filter>) {
        let Some(filter) = filter else {
            self.word.store(0, Ordering::Relaxed);
            return;
        };
        let mut word = u64::from(filter.kind.code());
        if filter.enabled {
            word |= ENABLED_BIT;
        }
        if matches!(filter.width, Width::Octaves(_)) {
            word |= OCTAVES_BIT;
        }
        self.word.store(word, Ordering::Relaxed);
        self.frequency
            .store(filter.frequency.to_bits(), Ordering::Relaxed);
        self.gain.store(filter.gain.to_bits(), Ordering::Relaxed);
        self.width
            .store(filter.width.value().to_bits(), Ordering::Relaxed);
    }

    /// The filter as stored. It was validated before it was stored, so it is not checked again.
    fn load(&self) -> Filter {
        let word = self.word.load(Ordering::Relaxed);
        let width = f64::from_bits(self.width.load(Ordering::Relaxed));
        Filter {
            kind: Kind::of(word as u8),
            frequency: f64::from_bits(self.frequency.load(Ordering::Relaxed)),
            gain: f64::from_bits(self.gain.load(Ordering::Relaxed)),
            width: match word & OCTAVES_BIT != 0 {
                true => Width::Octaves(width),
                false => Width::Q(width),
            },
            enabled: word & ENABLED_BIT != 0,
        }
    }
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            word: AtomicU64::new(0),
            frequency: AtomicU64::new(1_000f64.to_bits()),
            gain: AtomicU64::new(0f64.to_bits()),
            width: AtomicU64::new(1f64.to_bits()),
        }
    }
}
