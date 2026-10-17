use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, Context, Entity};
use music::{ArtistRef, Track};
use serde::{Deserialize, Serialize};

use crate::{AppSettings, Origin, Outcome, Session, SessionEvent, Toast, Toasts};

const PAST_LIMIT: usize = 500;
const KEPT_PAST: usize = 20;
const KEPT_UPCOMING: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Resume {
    pub(crate) provider: String,
    pub(crate) position: f32,
    /// The one origin older versions saved for the whole queue, read only to restore such a queue.
    #[serde(skip_serializing)]
    pub(crate) origin: Option<Origin>,
    /// The collections the saved tracks were queued from. Each stub points into this list.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) origins: Vec<Origin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current: Option<Stub>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) past: Vec<Stub>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) upcoming: Vec<Stub>,
    /// How many of `upcoming` the user queued by hand. They play before the rest.
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) manual: usize,
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stub {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) artists: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) credited: Vec<Named>,
    pub(crate) album: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) album_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cover: Option<String>,
    pub(crate) seconds: f32,
    pub(crate) explicit: bool,
    /// Which of the resume's `origins` the track was queued from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Named {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
}

/// A queued track and the collection it was queued from. A track the user added on its own has
/// no origin.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    track: Track,
    origin: Option<Origin>,
}

impl Entry {
    fn id(&self) -> Option<&str> {
        self.track.id.as_deref()
    }
}

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

/// The upcoming tracks Clear took, each run kept apart so Undo can put it back in its own.
struct Cleared {
    manual: Vec<Entry>,
    rest: Vec<Entry>,
    /// `Queue::rebuilt` when the tracks were taken.
    rebuilt: u64,
}

/// A track removed from the queue on its own, and where it sat.
struct Taken {
    entry: Entry,
    spot: Spot,
}

/// Where a removed track sat, counted from the start of its run.
#[derive(Clone, Copy)]
enum Spot {
    Manual(usize),
    Source(usize),
    /// A suggestion, with `Queue::suggested` at the time, since one from a batch replaced since
    /// is not put back.
    Similar(usize, u64),
}

fn stub(track: &Track) -> Option<Stub> {
    Some(Stub {
        id: track.id.clone()?,
        name: track.name.clone(),
        artists: track.artists.clone(),
        credited: track
            .artist_refs
            .iter()
            .map(|artist| Named {
                name: artist.name.clone(),
                id: artist.id.clone(),
            })
            .collect(),
        album: track.album.clone(),
        album_id: track.album_id.clone(),
        cover: track.cover.clone(),
        seconds: track.duration.as_secs_f32(),
        explicit: track.explicit,
        from: None,
    })
}

fn hydrate(stub: Stub) -> Track {
    Track {
        id: Some(stub.id),
        name: stub.name,
        playable: true,
        artists: stub.artists,
        artist_refs: stub
            .credited
            .into_iter()
            .map(|named| ArtistRef {
                name: named.name,
                id: named.id,
            })
            .collect(),
        album: stub.album,
        album_id: stub.album_id,
        cover: stub.cover,
        duration: Duration::from_secs_f32(stub.seconds.max(0.)),
        added_at: None,
        added_by: None,
        playcount: None,
        popularity: 0,
        explicit: stub.explicit,
        track_number: 0,
        disc_number: 0,
        tags: Vec::new(),
        languages: Vec::new(),
        credits: Vec::new(),
    }
}

/// Snapshots the queue for the next launch. The first `manual` upcoming tracks are the ones the
/// user queued by hand, and the count written out only counts those that survive as stubs. Each
/// distinct origin is written once and the stubs point at it.
fn record<'a>(
    provider: &str,
    past: &[Entry],
    current: Option<&Entry>,
    upcoming: impl Iterator<Item = &'a Entry>,
    manual: usize,
) -> Resume {
    let mut origins: Vec<Origin> = Vec::new();
    let mut save = |entry: &Entry| {
        let mut stub = stub(&entry.track)?;
        stub.from = entry.origin.as_ref().map(|origin| {
            match origins.iter().position(|known| known == origin) {
                Some(index) => index,
                None => {
                    origins.push(origin.clone());
                    origins.len() - 1
                }
            }
        });
        Some(stub)
    };
    let past = past[past.len().saturating_sub(KEPT_PAST)..]
        .iter()
        .filter_map(&mut save)
        .collect();
    let current = current.and_then(&mut save);
    let mut kept_manual = 0;
    let upcoming = upcoming
        .take(KEPT_UPCOMING)
        .enumerate()
        .filter_map(|(index, entry)| {
            let stub = save(entry)?;
            kept_manual += usize::from(index < manual);
            Some(stub)
        })
        .collect();
    Resume {
        provider: provider.to_owned(),
        position: 0.,
        origin: None,
        origins,
        current,
        past,
        upcoming,
        manual: kept_manual,
    }
}

fn local(track: &Track) -> bool {
    track.id.as_deref().is_some_and(music::is_local_id)
}

fn sift<T>(
    past: &mut Vec<T>,
    current: &mut Option<T>,
    upcoming: &mut VecDeque<T>,
    source: &mut Vec<T>,
    keep: impl Fn(&T) -> bool,
) -> bool {
    let tally = |past: &Vec<T>, current: &Option<T>, upcoming: &VecDeque<T>, source: &Vec<T>| {
        past.len() + usize::from(current.is_some()) + upcoming.len() + source.len()
    };

    let before = tally(past, current, upcoming, source);
    past.retain(&keep);
    if current.as_ref().is_some_and(|item| !keep(item)) {
        *current = None;
    }
    upcoming.retain(&keep);
    source.retain(&keep);
    before != tally(past, current, upcoming, source)
}

fn scramble(upcoming: &mut VecDeque<Entry>, source: &[Entry], current: Option<&Entry>) {
    let known: HashSet<&str> = source.iter().filter_map(Entry::id).collect();
    let mut entries: Vec<Entry> = upcoming
        .drain(..)
        .filter(|entry| !entry.id().is_some_and(|id| known.contains(id)))
        .collect();

    let mut playing = current.and_then(Entry::id);
    entries.extend(
        source
            .iter()
            .filter(|entry| match playing == entry.id() {
                true => {
                    playing = None;
                    false
                }
                false => true,
            })
            .cloned(),
    );

    fastrand::shuffle(&mut entries);
    *upcoming = entries.into();
}

fn restore(upcoming: &mut VecDeque<Entry>, source: &[Entry], current: Option<&Entry>) {
    let known: HashSet<&str> = source.iter().filter_map(Entry::id).collect();
    let extra: Vec<Entry> = upcoming
        .drain(..)
        .filter(|entry| !entry.id().is_some_and(|id| known.contains(id)))
        .collect();

    let at = current
        .and_then(Entry::id)
        .and_then(|id| source.iter().position(|entry| entry.id() == Some(id)));
    let tail = match at {
        Some(at) => &source[at + 1..],
        None => source,
    };

    *upcoming = tail.iter().cloned().chain(extra).collect();
}

fn move_item<T>(items: &mut VecDeque<T>, from: usize, to: usize) -> bool {
    if from >= items.len() || to >= items.len() || from == to {
        return false;
    }
    let Some(item) = items.remove(from) else {
        return false;
    };
    items.insert(to, item);
    true
}

fn select_past<T>(
    past: &mut Vec<T>,
    current: &mut Option<T>,
    upcoming: &mut VecDeque<T>,
    index: usize,
) -> bool {
    if index >= past.len() {
        return false;
    }

    let mut replay = VecDeque::from(past.split_off(index));
    let selected = replay.pop_front().expect("past index was checked");
    if let Some(playing) = current.replace(selected) {
        replay.push_back(playing);
    }
    replay.append(upcoming);
    *upcoming = replay;
    true
}

fn select_upcoming<T>(
    past: &mut Vec<T>,
    current: &mut Option<T>,
    upcoming: &mut VecDeque<T>,
    index: usize,
) -> bool {
    let Some(selected) = upcoming.remove(index) else {
        return false;
    };

    past.extend(current.replace(selected));
    past.extend(upcoming.drain(..index));
    true
}

fn trim<T>(past: &mut Vec<T>, limit: usize) {
    let over = past.len().saturating_sub(limit);
    if over > 0 {
        past.drain(..over);
    }
}

fn in_order<'a, T: PartialEq + 'a>(sequence: impl Iterator<Item = &'a T>, source: &[T]) -> bool {
    sequence.eq(source.iter())
}

pub(crate) fn gap_target(from: usize, gap: usize, len: usize) -> usize {
    let gap = gap.min(len);
    if gap > from { gap - 1 } else { gap }
}

/// Pairs each track with the collection it was queued from.
fn tagged(
    tracks: impl IntoIterator<Item = Track>,
    origin: Option<Origin>,
) -> impl Iterator<Item = Entry> {
    tracks.into_iter().map(move |track| Entry {
        track,
        origin: origin.clone(),
    })
}

/// The play order around the current track. `upcoming` is one list in three runs: the first
/// `manual` tracks are what the user queued by hand, then the rest of the source, then the last
/// `similar` tracks are radio suggestions. Add to queue and Play next land in the first run, so
/// they play before the album or playlist continues. Every entry keeps the collection it was
/// queued from, so the current one tells what the queue is playing from.
pub struct Queue {
    past: Vec<Entry>,
    current: Option<Entry>,
    upcoming: VecDeque<Entry>,
    source: Vec<Entry>,
    manual: usize,
    similar: usize,
    shuffle: bool,
    revision: u64,
    /// The toast counting tracks removed one by one while it may still be up.
    removed: Option<Reported<Vec<Taken>>>,
    /// Counts the times `upcoming` was rebuilt from the source, which can bring back tracks an
    /// Undo is holding.
    rebuilt: u64,
    /// Counts the times the suggestions were replaced or dropped.
    suggested: u64,
    session: Entity<Session>,
    settings: Entity<AppSettings>,
}

impl Queue {
    pub fn new(
        session: Entity<Session>,
        settings: Entity<AppSettings>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedOut => this.purge(cx),
            SessionEvent::SignedIn | SessionEvent::Reconnected | SessionEvent::LocalChanged => {}
        })
        .detach();

        let shuffle = settings.read(cx).shuffle();

        Self {
            past: Vec::new(),
            current: None,
            upcoming: VecDeque::new(),
            source: Vec::new(),
            manual: 0,
            similar: 0,
            shuffle,
            revision: 0,
            removed: None,
            rebuilt: 0,
            suggested: 0,
            session,
            settings,
        }
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn set_shuffle(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.shuffle == on {
            return;
        }
        self.shuffle = on;
        self.rebuilt += 1;
        self.settings
            .update(cx, |settings, cx| settings.set_shuffle(on, cx));
        let mut suggested = self.upcoming.split_off(self.queued());
        let mut manual: VecDeque<Entry> = self.upcoming.drain(..self.manual).collect();
        match on {
            true => scramble(&mut self.upcoming, &self.source, self.current.as_ref()),
            false => restore(&mut self.upcoming, &self.source, self.current.as_ref()),
        }
        manual.append(&mut self.upcoming);
        self.upcoming = manual;
        self.upcoming.append(&mut suggested);
        self.changed(cx);
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        self.set_shuffle(!self.shuffle, cx);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        trim(&mut self.past, PAST_LIMIT);
        self.revision = self.revision.wrapping_add(1);
        self.remember(cx);
        cx.notify();
    }

    fn blank(&self) -> bool {
        self.past.is_empty() && self.current.is_none() && self.upcoming.is_empty()
    }

    fn remember(&mut self, cx: &mut Context<Self>) {
        let resume = self
            .session
            .read(cx)
            .provider_slug()
            .filter(|_| !self.blank())
            .map(|slug| {
                record(
                    slug,
                    &self.past,
                    self.current.as_ref(),
                    self.upcoming.range(..self.queued()),
                    self.manual,
                )
            });
        self.settings
            .update(cx, |settings, cx| settings.set_resume(resume, cx));
    }

    /// Restores a saved queue. A queue saved by an older version has one origin for all of it,
    /// which goes to every track except the ones queued by hand.
    pub(crate) fn revive(&mut self, resume: Resume, cx: &mut Context<Self>) -> Option<Track> {
        let origins = resume.origins;
        let entry = |stub: Stub| Entry {
            origin: stub.from.and_then(|index| origins.get(index).cloned()),
            track: hydrate(stub),
        };
        self.past = resume.past.into_iter().map(&entry).collect();
        self.current = resume.current.map(&entry);
        self.upcoming = resume.upcoming.into_iter().map(&entry).collect();
        self.manual = resume.manual.min(self.upcoming.len());
        self.similar = 0;
        self.suggested += 1;
        if let Some(origin) = resume.origin {
            self.past
                .iter_mut()
                .chain(self.current.as_mut())
                .chain(self.upcoming.range_mut(self.manual..))
                .for_each(|entry| entry.origin = Some(origin.clone()));
        }
        self.source = self
            .past
            .iter()
            .chain(self.current.as_ref())
            .chain(self.upcoming.range(self.manual..))
            .cloned()
            .collect();
        self.changed(cx);
        self.current().cloned()
    }

    fn purge(&mut self, cx: &mut Context<Self>) {
        let suggested = self.similar > 0;
        self.upcoming.truncate(self.queued());
        self.similar = 0;
        self.suggested += 1;
        self.manual = self
            .upcoming
            .range(..self.manual)
            .filter(|entry| local(&entry.track))
            .count();
        let sifted = sift(
            &mut self.past,
            &mut self.current,
            &mut self.upcoming,
            &mut self.source,
            |entry| local(&entry.track),
        );
        if suggested || sifted {
            self.changed(cx);
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn past(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.past.iter().map(|entry| &entry.track)
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.as_ref().map(|entry| &entry.track)
    }

    /// The collection the current track was queued from. `None` while nothing plays or when the
    /// user added the current track on its own.
    pub fn origin(&self) -> Option<&Origin> {
        self.current.as_ref()?.origin.as_ref()
    }

    /// Where the current track sits in the collection the queue was started from. `None` when it
    /// is not in that collection, as for a track queued by hand.
    pub fn place(&self) -> Option<usize> {
        let current = self.current.as_ref()?;
        self.source.iter().position(|entry| entry == current)
    }

    fn queued(&self) -> usize {
        self.upcoming.len() - self.similar
    }

    /// Everything queued to play, hand-queued tracks first, without the suggestions.
    pub fn upcoming(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.upcoming
            .range(..self.queued())
            .map(|entry| &entry.track)
    }

    /// The tracks the user queued by hand. They open `upcoming`.
    pub fn manual(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.upcoming.range(..self.manual).map(|entry| &entry.track)
    }

    pub fn similar(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.upcoming
            .range(self.queued()..)
            .map(|entry| &entry.track)
    }

    /// Replaces the suggestions with `tracks` from the radio `origin`.
    pub fn suggest(&mut self, tracks: Vec<Track>, origin: Origin, cx: &mut Context<Self>) {
        self.upcoming.truncate(self.queued());
        self.similar = tracks.len();
        self.suggested += 1;
        self.upcoming.extend(tagged(tracks, Some(origin)));
        self.changed(cx);
    }

    /// Adds suggestions after the ones already there, for a station topped up before the queue
    /// has played through it.
    pub fn extend_similar(&mut self, tracks: Vec<Track>, origin: Origin, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        self.similar += tracks.len();
        self.upcoming.extend(tagged(tracks, Some(origin)));
        self.changed(cx);
    }

    pub fn clear_similar(&mut self, cx: &mut Context<Self>) {
        if self.similar == 0 {
            return;
        }
        self.upcoming.truncate(self.queued());
        self.similar = 0;
        self.suggested += 1;
        self.changed(cx);
    }

    pub fn ids(&self) -> HashSet<String> {
        self.past
            .iter()
            .chain(self.current.as_ref())
            .chain(self.upcoming.iter())
            .filter_map(|entry| entry.track.id.clone())
            .collect()
    }

    pub fn remove_tracks(&mut self, ids: &[String], cx: &mut Context<Self>) -> bool {
        if ids.is_empty() {
            return false;
        }
        let matches = |entry: &Entry| entry.track.id.as_ref().is_some_and(|id| ids.contains(id));
        let current_removed = self.current.as_ref().is_some_and(matches);
        let before = self.past.len() + usize::from(self.current.is_some()) + self.upcoming.len();
        self.past.retain(|entry| !matches(entry));
        if current_removed {
            self.current = None;
        }
        self.manual -= self
            .upcoming
            .range(..self.manual)
            .filter(|entry| matches(entry))
            .count();
        self.upcoming.retain(|entry| !matches(entry));
        self.source.retain(|entry| !matches(entry));
        let after = self.past.len() + usize::from(self.current.is_some()) + self.upcoming.len();
        if current_removed || before != after {
            self.changed(cx);
        }
        current_removed
    }

    pub fn len(&self) -> usize {
        self.upcoming.len()
    }

    pub fn is_empty(&self) -> bool {
        self.current.is_none() && self.upcoming.is_empty()
    }

    pub fn has_next(&self) -> bool {
        !self.upcoming.is_empty()
    }

    /// Whether the track `next` would hand back is a radio suggestion rather than one the queue
    /// was started with.
    pub fn next_is_suggested(&self) -> bool {
        self.queued() == 0 && self.similar > 0
    }

    pub fn has_previous(&self) -> bool {
        !self.past.is_empty()
    }

    /// Drops everything queued, keeping the suggestions, and posts a toast whose Undo puts it
    /// back. Every clear gets a toast and an Undo of its own.
    pub fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        if self.queued() == 0 {
            return;
        }
        let mut rest: Vec<Entry> = self.upcoming.drain(..self.queued()).collect();
        let manual: Vec<Entry> = rest.drain(..self.manual).collect();
        self.manual = 0;
        self.changed(cx);

        let snapshot = Rc::new(RefCell::new(Some(Cleared {
            manual,
            rest,
            rebuilt: self.rebuilt,
        })));
        let toast = Self::cleared_toast(snapshot, cx);
        Toasts::post(toast, cx);
    }

    /// The Queue cleared toast, whose Undo hands `snapshot` back to the queue once.
    fn cleared_toast(snapshot: Snapshot<Cleared>, cx: &mut Context<Self>) -> Toast {
        let queue = cx.entity().downgrade();
        Toast::new(Outcome::Plain, "toast-queue-cleared").undo(move |cx| {
            let Some(cleared) = snapshot.borrow_mut().take() else {
                return;
            };
            queue
                .update(cx, |queue, cx| queue.unclear(cleared, cx))
                .ok();
        })
    }

    /// Puts back what Clear took, each run ahead of what was queued into it since, in the order
    /// it had. A rebuild from the source since may have brought some of those tracks back
    /// already, and those copies are dropped so nothing plays twice.
    fn unclear(&mut self, cleared: Cleared, cx: &mut Context<Self>) {
        let suggested = self.upcoming.split_off(self.queued());
        let mut since_rest: Vec<Entry> = self.upcoming.drain(..).collect();
        let since_manual: Vec<Entry> = since_rest.drain(..self.manual).collect();
        if cleared.rebuilt != self.rebuilt {
            drop_copies(&mut since_rest, &cleared.rest);
        }
        self.manual = cleared.manual.len() + since_manual.len();
        self.upcoming = cleared
            .manual
            .into_iter()
            .chain(since_manual)
            .chain(cleared.rest)
            .chain(since_rest)
            .chain(suggested)
            .collect();
        self.changed(cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.past.clear();
        self.current = None;
        self.upcoming.clear();
        self.source.clear();
        self.manual = 0;
        self.similar = 0;
        self.changed(cx);
    }

    pub fn reordered(&self) -> bool {
        let sequence = self
            .past
            .iter()
            .chain(self.current.as_ref())
            .chain(self.upcoming.iter());

        !self.source.is_empty() && !in_order(sequence, &self.source)
    }

    pub fn reset(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        if self.source.is_empty() {
            return None;
        }

        let index = self
            .current
            .as_ref()
            .and_then(|current| {
                self.source
                    .iter()
                    .position(|entry| entry.track == current.track)
            })
            .unwrap_or_default();
        let source = self.source.clone();
        self.fill(source, index, cx)
    }

    /// Makes `tracks` the queue, all of them queued from `origin`, and plays the one at `index`.
    pub fn start(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) -> Option<Track> {
        self.fill(tagged(tracks, origin).collect(), index, cx)
    }

    /// Makes `entries` the source and the queue, keeping the origin each one has.
    fn fill(&mut self, entries: Vec<Entry>, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        if index >= entries.len() {
            return None;
        }

        self.source = entries.clone();
        let mut past = entries;
        self.upcoming = past.split_off(index + 1).into();
        self.manual = 0;
        self.similar = 0;
        self.rebuilt += 1;
        self.suggested += 1;
        self.current = past.pop();
        if self.shuffle {
            scramble(&mut self.upcoming, &self.source, self.current.as_ref());
            past.clear();
        }
        self.past = past;
        self.changed(cx);
        self.current().cloned()
    }

    /// Queues tracks after the ones already queued by hand, ahead of the rest of the source.
    pub fn append_all(
        &mut self,
        tracks: impl IntoIterator<Item = Track>,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        self.insert_upcoming(self.manual, tracks, origin, cx);
    }

    /// Queues tracks after everything else, so they play once the source has run out.
    pub fn append_last_all(
        &mut self,
        tracks: impl IntoIterator<Item = Track>,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        let at = self.queued();
        for (offset, entry) in tagged(tracks, origin).enumerate() {
            self.upcoming.insert(at + offset, entry);
        }
        self.changed(cx);
    }

    /// Adds the next stretch of the station the queue was started from, both to the source and
    /// after what is queued.
    pub(crate) fn extend_context(
        &mut self,
        tracks: Vec<Track>,
        origin: Origin,
        cx: &mut Context<Self>,
    ) {
        let at = self.queued();
        let entries: Vec<Entry> = tagged(tracks, Some(origin)).collect();
        self.source.extend(entries.iter().cloned());
        for (offset, entry) in entries.into_iter().enumerate() {
            self.upcoming.insert(at + offset, entry);
        }
        self.changed(cx);
    }

    /// Inserts tracks `gap` places into the upcoming list. A gap inside or right after the
    /// hand-queued run joins that run, so a drop at the very top always plays next.
    pub fn insert_upcoming(
        &mut self,
        gap: usize,
        tracks: impl IntoIterator<Item = Track>,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        let at = gap.min(self.queued());
        let mut count = 0;
        for (offset, entry) in tagged(tracks, origin).enumerate() {
            self.upcoming.insert(at + offset, entry);
            count += 1;
        }
        if at <= self.manual {
            self.manual += count;
        }
        self.changed(cx);
    }

    /// Queues tracks to play right after the current one, ahead of anything queued before.
    pub fn prepend_all(
        &mut self,
        tracks: impl IntoIterator<Item = Track>,
        origin: Option<Origin>,
        cx: &mut Context<Self>,
    ) {
        let mut upcoming = tagged(tracks, origin).collect::<VecDeque<_>>();
        self.manual += upcoming.len();
        upcoming.append(&mut self.upcoming);
        self.upcoming = upcoming;
        self.changed(cx);
    }

    /// Moves an upcoming track. Landing inside or right after the hand-queued run joins it, and
    /// leaving that run for the source's tracks leaves it.
    pub fn move_upcoming(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let from_manual = from < self.manual;
        if !move_item(&mut self.upcoming, from, to) {
            return;
        }
        self.manual -= usize::from(from_manual);
        self.manual += usize::from(to <= self.manual);
        self.changed(cx);
    }

    pub fn move_upcoming_to_gap(&mut self, from: usize, gap: usize, cx: &mut Context<Self>) {
        let to = gap_target(from, gap, self.queued());
        self.move_upcoming(from, to, cx);
    }

    /// Removes one queued track and reports it on the toast that counts removals, whose Undo
    /// puts it back.
    pub fn remove_upcoming(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.queued() {
            return;
        }
        let Some(entry) = self.upcoming.remove(index) else {
            return;
        };
        let spot = match index < self.manual {
            true => Spot::Manual(index),
            false => Spot::Source(index - self.manual),
        };
        self.manual -= usize::from(index < self.manual);
        self.changed(cx);
        self.report_removal(Taken { entry, spot }, cx);
    }

    /// Removes one suggestion and reports it the way `remove_upcoming` does.
    pub fn remove_similar(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.similar {
            return;
        }
        let Some(entry) = self.upcoming.remove(self.queued() + index) else {
            return;
        };
        self.similar -= 1;
        self.changed(cx);
        let spot = Spot::Similar(index, self.suggested);
        self.report_removal(Taken { entry, spot }, cx);
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
            Some([only]) => Toast::new(Outcome::Plain, "toast-queue-removed-track")
                .name(only.entry.track.name.clone()),
            removed => Toast::new(Outcome::Plain, "toast-queue-removed-tracks")
                .count(removed.map_or(0, <[Taken]>::len)),
        };
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

    /// Puts removed tracks back where they sat, newest first, so each lands where it was before
    /// the ones after it were taken. Spots are clamped to what each run holds now, and
    /// suggestions from a batch replaced since are left out.
    fn put_back(&mut self, removed: Vec<Taken>, cx: &mut Context<Self>) {
        for Taken { entry, spot } in removed.into_iter().rev() {
            match spot {
                Spot::Manual(index) => {
                    self.upcoming.insert(index.min(self.manual), entry);
                    self.manual += 1;
                }
                Spot::Source(index) => {
                    let at = (self.manual + index).min(self.queued());
                    self.upcoming.insert(at, entry);
                }
                Spot::Similar(index, batch) if batch == self.suggested => {
                    let at = self.queued() + index.min(self.similar);
                    self.upcoming.insert(at, entry);
                    self.similar += 1;
                }
                Spot::Similar(..) => {}
            }
        }
        self.changed(cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.similar -= usize::from(self.queued() == 0 && self.similar > 0);
        let next = self.upcoming.pop_front()?;
        self.manual = self.manual.saturating_sub(1);
        if let Some(played) = self.current.replace(next) {
            self.past.push(played);
        }
        self.changed(cx);
        self.current().cloned()
    }

    pub fn rewind(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.upcoming.truncate(self.queued());
        self.similar = 0;
        let mut entries = std::mem::take(&mut self.past);
        entries.extend(self.current.take());
        entries.extend(self.upcoming.drain(..));
        self.fill(entries, 0, cx)
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        let index = self.past.iter().rposition(|entry| entry.track.playable)?;
        self.play_past(index, cx)
    }

    /// Replays a track from the history. What was playing and anything after it in the history
    /// line up ahead of the hand-queued run, so Next returns to where the listener was.
    pub fn play_past(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        let before = self.upcoming.len();
        if !select_past(&mut self.past, &mut self.current, &mut self.upcoming, index) {
            return None;
        }
        self.manual += self.upcoming.len() - before;
        self.changed(cx);
        self.current().cloned()
    }

    /// Jumps to an upcoming track. Hand-queued tracks it skips go to the history, but picking a
    /// track from the source keeps the hand-queued run for afterwards.
    pub fn play_upcoming(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        if index >= self.queued() {
            return None;
        }
        let selected = self.upcoming.remove(index)?;
        self.past.extend(self.current.replace(selected));
        match index < self.manual {
            true => {
                self.past.extend(self.upcoming.drain(..index));
                self.manual -= index + 1;
            }
            false => {
                self.past.extend(self.upcoming.drain(self.manual..index));
            }
        }
        self.changed(cx);
        self.current().cloned()
    }

    pub fn play_similar(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        if index >= self.similar {
            return None;
        }
        let target = self.queued() + index;
        if !select_upcoming(
            &mut self.past,
            &mut self.current,
            &mut self.upcoming,
            target,
        ) {
            return None;
        }
        self.manual = 0;
        self.similar -= index + 1;
        self.changed(cx);
        self.current().cloned()
    }
}

/// Drops from `entries` one copy of each track in `restored`, matched by id, for when the
/// queue brought back on its own some of what an Undo is about to restore.
fn drop_copies(entries: &mut Vec<Entry>, restored: &[Entry]) {
    let mut owed: HashMap<&str, usize> = HashMap::new();
    for id in restored.iter().filter_map(Entry::id) {
        *owed.entry(id).or_default() += 1;
    }
    entries.retain(|entry| {
        let Some(owed) = entry.id().and_then(|id| owed.get_mut(id)) else {
            return true;
        };
        let keep = *owed == 0;
        *owed = owed.saturating_sub(1);
        keep
    });
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use music::{ArtistRef, Track};

    use super::{
        Entry, gap_target, hydrate, in_order, local, move_item, record, restore, scramble,
        select_past, select_upcoming, sift, stub, trim,
    };

    fn track(id: &str) -> Track {
        Track {
            id: Some(id.to_owned()),
            name: id.to_owned(),
            playable: true,
            artists: String::new(),
            artist_refs: Vec::new(),
            album: String::new(),
            album_id: None,
            cover: None,
            duration: Duration::from_secs(180),
            added_at: None,
            added_by: None,
            playcount: None,
            popularity: 0,
            explicit: false,
            track_number: 0,
            disc_number: 0,
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        }
    }

    fn entry(id: &str) -> Entry {
        Entry {
            track: track(id),
            origin: None,
        }
    }

    fn ids(entries: &VecDeque<Entry>) -> Vec<String> {
        entries
            .iter()
            .map(|entry| entry.track.name.clone())
            .collect::<Vec<_>>()
    }

    fn listing(count: usize) -> Vec<Entry> {
        (0..count).map(|index| entry(&index.to_string())).collect()
    }

    #[test]
    fn scrambling_keeps_every_track() {
        let source = listing(20);
        let mut upcoming = VecDeque::new();

        scramble(&mut upcoming, &source, None);

        let mut seen = ids(&upcoming);
        let mut expected = ids(&source.iter().cloned().collect());
        seen.sort();
        expected.sort();
        assert_eq!(seen, expected);
    }

    #[test]
    fn restoring_returns_the_source_order() {
        let source = listing(20);
        let mut upcoming: VecDeque<Entry> = source.iter().cloned().collect();

        scramble(&mut upcoming, &source, None);
        restore(&mut upcoming, &source, None);

        assert_eq!(ids(&upcoming), ids(&source.iter().cloned().collect()));
    }

    #[test]
    fn restoring_puts_unknown_tracks_last() {
        let source = vec![entry("a"), entry("b"), entry("c")];
        let mut upcoming = VecDeque::from(vec![
            entry("queued-one"),
            entry("c"),
            entry("queued-two"),
            entry("a"),
        ]);

        restore(&mut upcoming, &source, None);

        assert_eq!(ids(&upcoming), ["a", "b", "c", "queued-one", "queued-two"]);
    }

    #[test]
    fn restoring_keeps_both_copies_of_a_repeated_track() {
        let source = vec![entry("a"), entry("b"), entry("a")];
        let mut upcoming = VecDeque::from(vec![entry("a"), entry("a"), entry("b")]);

        restore(&mut upcoming, &source, None);

        assert_eq!(ids(&upcoming), ["a", "b", "a"]);
    }

    #[test]
    fn restoring_continues_after_the_current_track() {
        let source = listing(12);
        let mut upcoming: VecDeque<Entry> = source.iter().cloned().collect();

        scramble(&mut upcoming, &source, Some(&source[8]));
        restore(&mut upcoming, &source, Some(&source[8]));

        assert_eq!(ids(&upcoming), ids(&source[9..].iter().cloned().collect()));
    }

    #[test]
    fn spots_a_reordered_queue() {
        let source = [1, 2, 3, 4];

        assert!(in_order([1, 2, 3, 4].iter(), &source));
        assert!(!in_order([1, 3, 2, 4].iter(), &source));
        assert!(!in_order([1, 2, 3].iter(), &source));
        assert!(!in_order([1, 2, 3, 4, 5].iter(), &source));
    }

    #[test]
    fn moves_items_in_both_directions() {
        let mut items = VecDeque::from([1, 2, 3, 4]);

        assert!(move_item(&mut items, 0, 2));
        assert_eq!(items, [2, 3, 1, 4]);
        assert!(move_item(&mut items, 3, 1));
        assert_eq!(items, [2, 4, 3, 1]);
    }

    #[test]
    fn ignores_invalid_moves() {
        let mut items = VecDeque::from([1, 2, 3]);

        assert!(!move_item(&mut items, 1, 1));
        assert!(!move_item(&mut items, 3, 0));
        assert!(!move_item(&mut items, 0, 3));
        assert_eq!(items, [1, 2, 3]);
    }

    #[test]
    fn selecting_past_rebuilds_the_queue_from_that_track() {
        let mut past = vec![1, 2, 3];
        let mut current = Some(4);
        let mut upcoming = VecDeque::from([5, 6]);

        assert!(select_past(&mut past, &mut current, &mut upcoming, 1));
        assert_eq!(past, [1]);
        assert_eq!(current, Some(2));
        assert_eq!(upcoming, [3, 4, 5, 6]);
    }

    #[test]
    fn selecting_invalid_past_track_keeps_the_queue() {
        let mut past = vec![1, 2];
        let mut current = Some(3);
        let mut upcoming = VecDeque::from([4, 5]);

        assert!(!select_past(&mut past, &mut current, &mut upcoming, 2));
        assert_eq!(past, [1, 2]);
        assert_eq!(current, Some(3));
        assert_eq!(upcoming, [4, 5]);
    }

    #[test]
    fn selecting_upcoming_moves_skipped_tracks_to_the_past() {
        let mut past = vec![1];
        let mut current = Some(2);
        let mut upcoming = VecDeque::from([3, 4, 5]);

        assert!(select_upcoming(&mut past, &mut current, &mut upcoming, 1));
        assert_eq!(past, [1, 2, 3]);
        assert_eq!(current, Some(4));
        assert_eq!(upcoming, [5]);
    }

    #[test]
    fn selecting_invalid_upcoming_track_keeps_the_queue() {
        let mut past = vec![1];
        let mut current = Some(2);
        let mut upcoming = VecDeque::from([3, 4]);

        assert!(!select_upcoming(&mut past, &mut current, &mut upcoming, 2));
        assert_eq!(past, [1]);
        assert_eq!(current, Some(2));
        assert_eq!(upcoming, [3, 4]);
    }

    #[test]
    fn converts_gaps_to_insertion_indices() {
        assert_eq!(gap_target(0, 3, 4), 2);
        assert_eq!(gap_target(0, 4, 4), 3);
        assert_eq!(gap_target(3, 1, 4), 1);
        assert_eq!(gap_target(2, 0, 4), 0);
        assert_eq!(gap_target(1, 1, 4), 1);
        assert_eq!(gap_target(1, 2, 4), 1);
        assert_eq!(gap_target(0, 10, 4), 3);
    }

    #[test]
    fn gap_moves_match_visual_positions() {
        let mut items = VecDeque::from([1, 2, 3, 4]);

        let to = gap_target(0, 3, items.len());
        assert!(move_item(&mut items, 0, to));
        assert_eq!(items, [2, 3, 1, 4]);

        let to = gap_target(3, 0, items.len());
        assert!(move_item(&mut items, 3, to));
        assert_eq!(items, [4, 2, 3, 1]);

        let to = gap_target(1, 2, items.len());
        assert!(!move_item(&mut items, 1, to));
        assert_eq!(items, [4, 2, 3, 1]);
    }

    #[test]
    fn a_saved_track_keeps_what_the_player_bar_shows() {
        let mut source = track("abc");
        source.artists = "One, Two".to_owned();
        source.artist_refs = vec![
            ArtistRef {
                name: "One".to_owned(),
                id: Some("a1".to_owned()),
            },
            ArtistRef {
                name: "Two".to_owned(),
                id: None,
            },
        ];
        source.album = "Album".to_owned();
        source.album_id = Some("al".to_owned());
        source.cover = Some("https://cover".to_owned());
        source.explicit = true;

        let saved = stub(&source).expect("a track with an id");
        let json = serde_json::to_string(&saved).expect("a serializable stub");
        let restored = hydrate(serde_json::from_str(&json).expect("a readable stub"));

        assert_eq!(restored.id, source.id);
        assert_eq!(restored.name, source.name);
        assert_eq!(restored.artists, source.artists);
        assert_eq!(restored.artist_refs, source.artist_refs);
        assert_eq!(restored.album, source.album);
        assert_eq!(restored.album_id, source.album_id);
        assert_eq!(restored.cover, source.cover);
        assert_eq!(restored.duration, source.duration);
        assert!(restored.explicit);
        assert!(restored.playable);
    }

    #[test]
    fn a_track_without_an_id_is_never_saved() {
        let mut orphan = track("abc");
        orphan.id = None;

        assert!(stub(&orphan).is_none());
    }

    #[test]
    fn a_record_caps_the_history_and_the_queue() {
        let past = listing(60);
        let upcoming = listing(300);
        let current = entry("now");

        let resume = record("spotify", &past, Some(&current), upcoming.iter(), 0);

        assert_eq!(resume.provider, "spotify");
        assert_eq!(resume.position, 0.);
        assert_eq!(resume.current.map(|stub| stub.id), Some("now".to_owned()));
        assert_eq!(resume.past.len(), 20);
        assert_eq!(resume.past.first().map(|stub| stub.id.as_str()), Some("40"));
        assert_eq!(resume.upcoming.len(), 200);
        assert_eq!(
            resume.upcoming.first().map(|stub| stub.id.as_str()),
            Some("0")
        );
    }

    #[test]
    fn sifting_keeps_only_what_is_wanted() {
        let mut past = vec![1, 2, 3];
        let mut current = Some(4);
        let mut upcoming = VecDeque::from([5, 6]);
        let mut source = vec![1, 2, 3, 4, 5, 6];

        assert!(sift(
            &mut past,
            &mut current,
            &mut upcoming,
            &mut source,
            |item| item % 2 == 0
        ));
        assert_eq!(past, [2]);
        assert_eq!(current, Some(4));
        assert_eq!(upcoming, [6]);
        assert_eq!(source, [2, 4, 6]);
    }

    #[test]
    fn sifting_reports_an_untouched_queue() {
        let mut past = vec![1];
        let mut current = Some(2);
        let mut upcoming = VecDeque::from([3]);
        let mut source = vec![1, 2, 3];

        assert!(!sift(
            &mut past,
            &mut current,
            &mut upcoming,
            &mut source,
            |_| true
        ));
        assert_eq!(past, [1]);
        assert_eq!(current, Some(2));
        assert_eq!(upcoming, [3]);
    }

    #[test]
    fn signing_out_leaves_only_imported_tracks() {
        let imported = format!("{}song.flac", music::LOCAL_TRACK_PREFIX);
        let mut past = vec![track("streamed"), track(&imported)];
        let mut current = Some(track("playing"));
        let mut upcoming = VecDeque::from([track("next"), track(&imported)]);
        let mut source = vec![track("streamed"), track(&imported)];

        assert!(sift(
            &mut past,
            &mut current,
            &mut upcoming,
            &mut source,
            local
        ));
        assert_eq!(past.len(), 1);
        assert!(current.is_none());
        assert_eq!(upcoming.len(), 1);
        assert_eq!(source.len(), 1);
        assert!(past.iter().chain(&source).all(local));
    }

    #[test]
    fn an_imported_track_outlives_the_session() {
        let imported = format!("{}song.flac", music::LOCAL_TRACK_PREFIX);
        let mut past = Vec::new();
        let mut current = Some(track(&imported));
        let mut upcoming = VecDeque::new();
        let mut source = Vec::new();

        assert!(!sift(
            &mut past,
            &mut current,
            &mut upcoming,
            &mut source,
            local
        ));
        assert!(current.is_some());
    }

    #[test]
    fn trimming_drops_the_oldest_history() {
        let mut past = vec![1, 2, 3, 4, 5];

        trim(&mut past, 3);
        assert_eq!(past, [3, 4, 5]);

        trim(&mut past, 3);
        assert_eq!(past, [3, 4, 5]);

        trim(&mut past, 0);
        assert!(past.is_empty());
    }
}
