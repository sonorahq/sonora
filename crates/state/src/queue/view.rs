//! The active queue exposed to consumers, independent of where playback happens.

use gpui::{App, Entity};
use music::Track;

use std::sync::Arc;

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

/// A cheap handle to the active play order. Only requested tracks are copied from the local
/// queue or remote metadata cache, never the whole playlist.
#[derive(Clone)]
pub struct QueueView {
    source: Source,
    pub sections: QueueSections,
    pub playing: bool,
}

#[derive(Clone)]
enum Source {
    Local(Entity<Queue>),
    Remote(Entity<Devices>),
}

impl QueueView {
    pub(crate) fn new(
        queue: Entity<Queue>,
        remote: Option<(Entity<Devices>, Elsewhere)>,
        playing: bool,
        cx: &App,
    ) -> Self {
        let (source, sections, playing) = match remote {
            Some((devices, remote)) => {
                let sections = devices.read(cx).queue_sections();
                (Source::Remote(devices), sections, remote.state.playing)
            }
            None => {
                let sections = queue.read(cx).sections.clone();
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
    pub fn revision(&self, cx: &App) -> u64 {
        match &self.source {
            Source::Local(queue) => queue.read(cx).revision(),
            Source::Remote(_) => 0,
        }
    }

    pub fn origin<'a>(&'a self, cx: &'a App) -> Option<&'a Origin> {
        match &self.source {
            Source::Local(queue) => queue.read(cx).origin(),
            Source::Remote(_) => None,
        }
    }

    pub fn track(&self, position: QueuePosition, cx: &App) -> Option<Track> {
        match &self.source {
            Source::Local(queue) => {
                let queue = queue.read(cx);
                match position {
                    QueuePosition::Past(at) => queue.past().nth(at),
                    QueuePosition::Current => queue.current(),
                    QueuePosition::Upcoming(at) => queue.upcoming().nth(at),
                }
                .cloned()
            }
            Source::Remote(devices) => devices.read(cx).queue_tracks(position),
        }
    }
}
