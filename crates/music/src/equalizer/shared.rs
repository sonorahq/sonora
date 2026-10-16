//! Carries the curve from its one writer to every output. The writer publishes into fixed
//! atomics behind a sequence counter, and a reader copies them out once per frame, trying again
//! when a write was under way, so the audio thread never waits, allocates or frees.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::sync::{Arc, Mutex, PoisonError, Weak};

use super::{Curve, Filter, Kind, MAX_CHANNELS, MAX_FILTERS, Width};
use crate::spectrum::Analyzer;

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
    /// The filter each slot holds. An unchanged filter keeps its slot, so deleting, adding or
    /// reordering rows starts no transition in any other slot, and slot order is not heard.
    held: [Option<Filter>; MAX_FILTERS],
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
    pub filters: [Filter; MAX_FILTERS],
}

struct Shared {
    /// Odd while a write is under way.
    seq: AtomicU32,
    enabled: AtomicBool,
    replaces: AtomicU32,
    preamp: AtomicU64,
    slots: [Slot; MAX_FILTERS],
    /// Set by an output that met more channels than it has state for and passed them through.
    unprocessed: AtomicBool,
    /// The rate each live output runs at, so the UI can draw the response at the one that plays.
    /// Only the non-audio side locks it.
    rates: Mutex<Vec<Weak<AtomicU32>>>,
    /// What every output's equalizer hears ahead of its preamp and filters.
    heard: Analyzer,
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
            rates: Mutex::new(Vec::new()),
            heard: Analyzer::default(),
        });
        let mut writer = Self {
            shared,
            enabled,
            curve,
            held: [None; MAX_FILTERS],
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

    /// The preamp the curve resolves to in decibels, as published to the outputs, so an
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
        self.place();
        self.publish(edit);
    }

    /// The spectrum of the music on its way into the equalizer, on every output.
    pub fn heard(&self) -> &Analyzer {
        &self.shared.heard
    }

    /// The highest rate any live output plays at, which is the one to draw the response at.
    pub fn rate(&self) -> Option<u32> {
        let mut rates = self
            .shared
            .rates
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        rates.retain(|rate| rate.strong_count() > 0);
        rates
            .iter()
            .filter_map(Weak::upgrade)
            .map(|rate| rate.load(Ordering::Relaxed))
            .filter(|rate| *rate > 0)
            .max()
    }

    /// Gives every filter of the curve a slot. An unchanged filter stays where it is, so a
    /// filter edited in place keeps its slot and can glide.
    fn place(&mut self) {
        let mut placed = [None; MAX_FILTERS];
        let mut moved = Vec::new();
        for filter in self.curve.filters() {
            let kept = (0..MAX_FILTERS)
                .find(|&slot| placed[slot].is_none() && self.held[slot] == Some(*filter));
            match kept {
                Some(slot) => placed[slot] = Some(*filter),
                None => moved.push(*filter),
            }
        }
        let mut free: Vec<usize> = (0..MAX_FILTERS)
            .filter(|&slot| placed[slot].is_none())
            .collect();
        free.sort_by_key(|&slot| self.held[slot].is_none());
        for (slot, filter) in free.into_iter().zip(moved) {
            placed[slot] = Some(filter);
        }
        self.held = placed;
    }

    fn publish(&self, edit: Edit) {
        let preamp = self.curve.preamp_db();
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

    /// A place for one output to publish its rate for as long as it holds it. Called when an
    /// output is built, never from the audio thread.
    pub(crate) fn rate_slot(&self) -> Arc<AtomicU32> {
        let slot = Arc::new(AtomicU32::new(0));
        let mut rates = self.0.rates.lock().unwrap_or_else(PoisonError::into_inner);
        rates.retain(|rate| rate.strong_count() > 0);
        rates.push(Arc::downgrade(&slot));
        slot
    }

    pub(crate) fn heard(&self) -> &Analyzer {
        &self.0.heard
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
