//! The active queue exposed to consumers, independent of where playback happens.

use gpui::{App, Entity};
use music::Track;

use crate::{Devices, Elsewhere, Origin, Queue, QueueKind, QueuePosition, QueueSections};

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

    pub fn origin(&self, cx: &App) -> Option<Origin> {
        match &self.source {
            Source::Local(queue) => queue.read(cx).origin().cloned(),
            Source::Remote(devices) => devices.read(cx).origin(cx),
        }
    }

    /// Why the track at `position` is in the queue, when the source knows. A queue steered
    /// on another device only says so for its upcoming tracks.
    pub fn kind(&self, position: QueuePosition, cx: &App) -> Option<QueueKind> {
        match &self.source {
            Source::Local(queue) => queue.read(cx).kind(position),
            Source::Remote(_) => self.sections.kind(position),
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
