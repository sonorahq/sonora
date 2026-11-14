//! How a play order is addressed and laid out for display.

use std::sync::Arc;

use super::QueueKind;

/// A song's position in play order, also used to address local queue actions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueuePosition {
    Past(usize),
    Current,
    Upcoming(usize),
}

impl QueuePosition {
    pub fn upcoming(self) -> Option<usize> {
        match self {
            Self::Upcoming(index) => Some(index),
            _ => None,
        }
    }
}

/// A section heading or song in the queue's display order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum QueueSlot {
    Header(&'static str),
    Track(QueuePosition),
}

/// The display rows, with a heading wherever upcoming provenance changes.
#[derive(Clone)]
pub struct QueueSections {
    pub past: usize,
    pub current: bool,
    pub upcoming: usize,
    slots: Arc<[QueueSlot]>,
    kinds: Arc<[QueueKind]>,
}

impl QueueSections {
    pub fn new(past: usize, current: bool, kinds: impl IntoIterator<Item = QueueKind>) -> Self {
        let kinds: Vec<_> = kinds.into_iter().collect();
        let mut slots = Vec::new();
        if past > 0 {
            slots.push(QueueSlot::Header("queue-history"));
            slots.extend((0..past).map(|at| QueueSlot::Track(QueuePosition::Past(at))));
        }
        if current {
            slots.push(QueueSlot::Header("queue-now-playing"));
            slots.push(QueueSlot::Track(QueuePosition::Current));
        }
        let mut previous = None;
        for (at, kind) in kinds.iter().enumerate() {
            if previous != Some(*kind) {
                slots.push(QueueSlot::Header(kind.heading()));
                previous = Some(*kind);
            }
            slots.push(QueueSlot::Track(QueuePosition::Upcoming(at)));
        }
        Self {
            past,
            current,
            upcoming: kinds.len(),
            slots: slots.into(),
            kinds: kinds.into(),
        }
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current
            .then_some(self.past + usize::from(self.past > 0) + 1)
    }

    pub fn slot(&self, index: usize) -> Option<QueueSlot> {
        self.slots.get(index).copied()
    }

    pub fn kind(&self, position: QueuePosition) -> Option<QueueKind> {
        let QueuePosition::Upcoming(index) = position else {
            return None;
        };
        self.kinds.get(index).copied()
    }
}
