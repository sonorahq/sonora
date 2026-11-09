//! The active queue exposed to consumers, independent of where playback happens.

use std::sync::Arc;

use music::Track;

use crate::{Devices, Elsewhere, Origin, Queue, QueueKind};

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

/// A cheap view of the active play order, borrowed from the store for as long as it is read.
/// Only requested tracks are copied from the local queue or remote metadata cache, never the
/// whole playlist.
#[derive(Clone)]
pub struct QueueView<'a> {
    source: Source<'a>,
    pub sections: QueueSections,
    pub playing: bool,
}

#[derive(Clone, Copy)]
enum Source<'a> {
    Local(&'a Queue),
    Remote(&'a Devices),
}

impl<'a> QueueView<'a> {
    /// The local queue, or the steered device's when `remote` names one, with whether the
    /// queue it shows is playing.
    pub(crate) fn new(
        queue: &'a Queue,
        devices: &'a Devices,
        remote: Option<Elsewhere>,
        playing: bool,
    ) -> Self {
        let (source, sections, playing) = match remote {
            Some(remote) => {
                let sections = devices.queue_sections();
                (Source::Remote(devices), sections, remote.state.playing)
            }
            None => {
                let sections = queue.sections.clone();
                (Source::Local(queue), sections, playing)
            }
        };
        Self {
            source,
            sections,
            playing,
        }
    }

    pub fn editable(&self) -> bool {
        matches!(self.source, Source::Local(_))
    }

    /// Remote queues are read-only; zero is never used to authorize an edit there.
    pub fn revision(&self) -> u64 {
        match self.source {
            Source::Local(queue) => queue.revision(),
            Source::Remote(_) => 0,
        }
    }

    pub fn origin(&self) -> Option<&'a Origin> {
        match self.source {
            Source::Local(queue) => queue.origin(),
            Source::Remote(_) => None,
        }
    }

    pub fn track(&self, position: QueuePosition) -> Option<Track> {
        match self.source {
            Source::Local(queue) => match position {
                QueuePosition::Past(at) => queue.past().nth(at),
                QueuePosition::Current => queue.current(),
                QueuePosition::Upcoming(at) => queue.upcoming().nth(at),
            }
            .cloned(),
            Source::Remote(devices) => devices.queue_tracks(position),
        }
    }
}
