use std::cell::RefCell;
use std::ops::Deref;
use std::rc::{Rc, Weak};

use gpui::{App, Context, Entity};
use logic::queue::{Order, Removed};
use music::Track;

use crate::{AppSettings, Origin, Outcome, Session, SessionEvent, Toast, Toasts};

mod view;
pub use logic::queue::{QueueKind, QueuePlacement, QueuePosition, QueueSections, QueueSlot, Stub};
pub(crate) use logic::queue::{Reach, gap_target};
pub use view::QueueView;

/// A queue as it is saved for the next launch.
pub type Resume = logic::queue::Resume<Origin>;

/// A track removed from the queue on its own, and where it sat.
type Taken = Removed<Origin>;

/// What an Undo on a queue toast puts back. The toast's handler holds the only strong
/// reference, so the tracks are let go once the toast is, and taking them empties it.
type Snapshot<T> = Rc<RefCell<Option<T>>>;

/// A toast the queue posted with an Undo, and a weak handle on what that Undo puts back.
struct Reported<T> {
    toast: usize,
    undo: Weak<RefCell<Option<T>>>,
}

impl<T> Reported<T> {
    /// What the toast's Undo puts back, while the toast is still up and its Undo unused, so a
    /// change made meanwhile can join it.
    fn live(&self, cx: &mut App) -> Option<Snapshot<T>> {
        let shown = Toasts::entity(cx)
            .read(cx)
            .shown()
            .iter()
            .any(|toast| toast.id == self.toast);
        let snapshot = self.undo.upgrade().filter(|_| shown)?;
        let unused = snapshot.borrow().is_some();
        unused.then_some(snapshot)
    }
}

/// The play order as the app holds it. [`Order`] decides what plays when, and this entity adds
/// what an edit owes the rest of the app: a redraw, the queue saved for the next launch and the
/// Undo toasts. Reading goes straight through to the order.
pub struct Queue {
    order: Order<Origin>,
    /// The toast counting tracks removed one by one while it may still be up.
    removed: Option<Reported<Vec<Taken>>>,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
}

impl Deref for Queue {
    type Target = Order<Origin>;

    fn deref(&self) -> &Self::Target {
        &self.order
    }
}

impl Queue {
    pub fn new(
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedOut => this.edit(cx, Order::purge),
            SessionEvent::SignedIn
            | SessionEvent::Reconnected
            | SessionEvent::WentOffline
            | SessionEvent::LocalChanged => {}
        })
        .detach();

        let shuffle = settings.read(cx).shuffle();

        Self {
            order: Order::new(shuffle),
            removed: None,
            session,
            settings,
        }
    }

    /// Runs `edit` on the order. When it changed anything, the queue is saved and redrawn.
    fn edit<T>(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Order<Origin>) -> T) -> T {
        let before = self.order.revision();
        let answer = edit(&mut self.order);
        if self.order.revision() != before {
            self.remember(cx);
            cx.notify();
        }
        answer
    }

    fn remember(&mut self, cx: &mut Context<Self>) {
        let resume = self
            .session
            .read(cx)
            .provider_slug()
            .and_then(|slug| self.order.record(slug));
        self.settings
            .update(cx, |settings, cx| settings.set_resume(resume, cx));
    }

    pub fn set_shuffle(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.order.shuffle() == on {
            return;
        }
        self.settings
            .update(cx, |settings, cx| settings.set_shuffle(on, cx));
        self.edit(cx, |order| order.set_shuffle(on));
    }

    /// Restores a saved queue. A queue saved by an older version has one origin for all of it,
    /// which goes to every track except the ones queued by hand.
    pub(crate) fn revive(&mut self, resume: Resume, cx: &mut Context<Self>) -> Option<Track> {
        self.edit(cx, |order| order.revive(resume))
    }

    /// Replaces the suggestions with `tracks` from the radio `origin`.
    pub fn suggest(&mut self, tracks: Vec<Track>, origin: Origin, cx: &mut Context<Self>) {
        self.edit(cx, |order| order.suggest(tracks, origin));
    }

    /// Adds suggestions after the ones already there, for a station topped up before the queue
    /// has played through it.
    pub fn extend_similar(&mut self, tracks: Vec<Track>, origin: Origin, cx: &mut Context<Self>) {
        self.edit(cx, |order| order.extend_similar(tracks, origin));
    }

    pub fn clear_similar(&mut self, cx: &mut Context<Self>) {
        self.edit(cx, Order::clear_similar);
    }

    /// Drops every occurrence of `ids` and says whether the current track was one of them.
    pub fn remove_tracks(&mut self, ids: &[String], cx: &mut Context<Self>) -> bool {
        self.edit(cx, |order| order.remove_tracks(ids))
    }

    /// Drops everything queued, keeping the suggestions, and posts a toast whose Undo puts it
    /// back. Every clear gets a toast and an Undo of its own.
    pub fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        let removed = self.edit(cx, Order::clear_upcoming);
        if removed.is_empty() {
            return;
        }
        let snapshot = Rc::new(RefCell::new(Some(removed)));
        let toast = Self::undo_toast(
            Toast::new(Outcome::Plain, "toast-queue-cleared"),
            snapshot,
            cx,
        );
        Toasts::post(toast, cx);
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.edit(cx, Order::reset)
    }

    /// Makes `tracks` the queue, all of them queued from `origin`, and plays the one at `index`.
    pub fn start(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) -> Option<Track> {
        self.edit(cx, |order| order.start(tracks, index, origin))
    }

    /// Applies a remote queue window while keeping the current entry and playback position.
    /// `reach` says how much of this queue the window stands for, so what lies beyond it stays.
    /// The suggestions are reported like any other song, so one the window no longer names is
    /// gone like any other too.
    pub(crate) fn replace_remote(
        &mut self,
        past: Vec<(Track, QueueKind)>,
        current: QueueKind,
        upcoming: Vec<(Track, QueueKind)>,
        reach: Reach,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |order| {
            order.replace_remote(past, current, upcoming, reach)
        });
    }

    /// Makes `tracks` what plays after the current track, as a Sonora Session shares it. Local
    /// files and suggestions are never shared, so the ones here stay after them.
    pub(crate) fn share_upcoming(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.edit(cx, |order| order.share_upcoming(tracks));
    }

    /// Inserts hand-queued tracks at `placement`, keeping their order and provenance.
    pub fn insert(
        &mut self,
        tracks: impl IntoIterator<Item = Track>,
        origin: Option<Origin>,
        placement: QueuePlacement,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |order| order.insert(tracks, origin, placement));
    }

    /// Adds the next stretch of the station the queue was started from, both to the source and
    /// after what is queued.
    pub(crate) fn extend_context(
        &mut self,
        tracks: Vec<Track>,
        origin: Origin,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |order| order.extend_context(tracks, origin));
    }

    /// Moves a song to `gap`. Landing inside or right after the hand-queued run joins it and
    /// leaving that run leaves it, and a suggestion dragged ahead of the queue's end becomes part
    /// of the queue, so the next radio refill keeps it.
    pub fn move_upcoming_to_gap(&mut self, from: usize, gap: usize, cx: &mut Context<Self>) {
        self.edit(cx, |order| order.move_upcoming_to_gap(from, gap));
    }

    /// Removes an upcoming song or suggestion, retaining its position for Undo.
    pub fn remove(&mut self, position: QueuePosition, cx: &mut Context<Self>) {
        let Some(taken) = self.edit(cx, |order| order.remove(position)) else {
            return;
        };
        self.report_removal(taken, cx);
    }

    /// Adds a removed track to the toast that reports removals, or posts a new one once the
    /// last has gone. Every removal restarts its clock.
    fn report_removal(&mut self, taken: Taken, cx: &mut Context<Self>) {
        let live = self
            .removed
            .as_ref()
            .and_then(|reported| Some((reported.toast, reported.live(cx)?)));
        match live {
            Some((id, snapshot)) => {
                if let Some(removed) = snapshot.borrow_mut().as_mut() {
                    removed.push(taken);
                }
                let toast = Self::removal_toast(snapshot, cx);
                Toasts::settle(id, toast, cx);
            }
            None => {
                let snapshot = Rc::new(RefCell::new(Some(vec![taken])));
                let undo = Rc::downgrade(&snapshot);
                let toast = Self::removal_toast(snapshot, cx);
                let toast = Toasts::post(toast, cx);
                self.removed = Some(Reported { toast, undo });
            }
        }
    }

    /// The toast for tracks removed one by one, naming one and counting several. Its Undo hands
    /// `snapshot` back to the queue once.
    fn removal_toast(snapshot: Snapshot<Vec<Taken>>, cx: &mut Context<Self>) -> Toast {
        let toast = match snapshot.borrow().as_deref() {
            Some([only]) => {
                Toast::new(Outcome::Plain, "toast-queue-removed-track").name(only.name().to_owned())
            }
            removed => Toast::new(Outcome::Plain, "toast-queue-removed-tracks")
                .count(removed.map_or(0, <[Taken]>::len)),
        };
        Self::undo_toast(toast, snapshot, cx)
    }

    /// Builds the same one-shot Undo for clearing a queue and removing individual songs.
    fn undo_toast(toast: Toast, snapshot: Snapshot<Vec<Taken>>, cx: &mut Context<Self>) -> Toast {
        let queue = cx.entity().downgrade();
        toast.undo(move |cx| {
            let Some(removed) = snapshot.borrow_mut().take() else {
                return;
            };
            queue
                .update(cx, |queue, cx| queue.put_back(removed, cx))
                .ok();
        })
    }

    /// Restores removals newest first, clamping positions to today's queue. Occurrences already
    /// played and recommendations from a replaced batch are not restored.
    fn put_back(&mut self, removed: Vec<Taken>, cx: &mut Context<Self>) {
        self.edit(cx, |order| order.put_back(removed));
    }

    pub fn next(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.edit(cx, Order::advance)
    }

    /// Starts the queue over from its first song, for repeat all. What was queued by hand joins
    /// the collection it played in, so the next loop has no stale hand-queued run.
    pub fn rewind(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.edit(cx, Order::rewind)
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.edit(cx, Order::previous)
    }

    /// Selects a song in play order. Replaying history queues the interrupted songs first;
    /// selecting the collection preserves manual additions, while selecting a suggestion skips them.
    pub fn select(&mut self, position: QueuePosition, cx: &mut Context<Self>) -> Option<Track> {
        self.edit(cx, |order| order.select(position))
    }
}
