use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{App, Context, Entity};
use music::connect::QueuedTrack;
use music::{ArtistRef, Track};
use serde::{Deserialize, Serialize};

use crate::{AppSettings, Origin, Outcome, Session, SessionEvent, Toast, Toasts};

mod view;
pub use view::{QueuePosition, QueueSections, QueueSlot, QueueView};

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
    /// Legacy boundary, read only to migrate queues saved before per-song provenance.
    #[serde(skip_serializing)]
    pub(crate) manual: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stub {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) artists: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) credited: Vec<ArtistRef>,
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
    /// Provenance survives saving and replaying history.
    #[serde(skip_serializing_if = "QueueKind::is_context")]
    pub(crate) kind: QueueKind,
}

/// One occurrence with identity, collection and provenance independent of position.
/// Source and active orders share its immutable song metadata.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    uid: String,
    track: Rc<Track>,
    origin: Option<Origin>,
    kind: QueueKind,
}

/// Where fetched tracks go in the queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueuePlacement {
    /// Right after the current track.
    Next,
    /// After the tracks queued by hand, ahead of the rest of the source.
    End,
    /// After everything, once the source has run out.
    Last,
    /// After this many upcoming tracks; a gap past the end appends.
    Gap(usize),
}

impl QueuePlacement {
    /// The toast key for a placement worth announcing.
    pub(crate) fn toast(self, source: &str) -> Option<String> {
        match self {
            Self::Next => Some(format!("toast-next-{source}")),
            Self::End => Some(format!("toast-queued-{source}")),
            Self::Last => Some(format!("toast-last-{source}")),
            Self::Gap(_) => None,
        }
    }
}

/// How much of this app's queue a queue window from another device stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// An edit to the queue this app reported, which showed the last `past` and the first
    /// `upcoming` provider songs. Whatever the report did not show was never seen, so it stays.
    Reported { past: usize, upcoming: usize },
    /// Another device's own queue taking over, which carries only the start of what it plays.
    /// What the collection loaded for it holds beyond that window stays after it.
    Handed,
}

/// Why a song is in play order, independent of where it currently sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueKind {
    #[default]
    Context,
    Queued,
    Suggested,
}

impl QueueKind {
    fn is_context(&self) -> bool {
        *self == Self::Context
    }

    pub fn heading(self) -> &'static str {
        match self {
            Self::Context => "queue-up-next",
            Self::Queued => "queue-next-in-queue",
            Self::Suggested => "queue-similar",
        }
    }
}

impl Entry {
    fn new(track: Track, origin: Option<Origin>, kind: QueueKind) -> Self {
        Self::with_uid(queue_uid(), track, origin, kind)
    }

    fn with_uid(uid: String, track: Track, origin: Option<Origin>, kind: QueueKind) -> Self {
        Self {
            uid,
            track: Rc::new(track),
            origin,
            kind,
        }
    }

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

/// A track removed from the queue on its own, and where it sat.
struct Taken {
    entry: Entry,
    index: usize,
    batch: u64,
}

fn stub(track: &Track) -> Option<Stub> {
    Some(Stub {
        id: track.id.clone()?,
        name: track.name.clone(),
        artists: track.artists.clone(),
        credited: track.artist_refs.clone(),
        album: track.album.clone(),
        album_id: track.album_id.clone(),
        cover: track.cover.clone(),
        seconds: track.duration.as_secs_f32(),
        explicit: track.explicit,
        from: None,
        kind: QueueKind::Context,
    })
}

fn hydrate(stub: Stub) -> Track {
    Track {
        id: Some(stub.id),
        name: stub.name,
        playable: true,
        artists: stub.artists,
        artist_refs: stub.credited,
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

/// Snapshots provenance and collection metadata for the next launch, omitting suggestions.
fn record<'a>(
    provider: &str,
    past: &[Entry],
    current: Option<&Entry>,
    upcoming: impl Iterator<Item = &'a Entry>,
) -> Resume {
    let mut origins: Vec<Origin> = Vec::new();
    let mut save = |entry: &Entry| {
        let mut stub = stub(&entry.track)?;
        stub.kind = entry.kind;
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
    let upcoming = upcoming
        .filter(|entry| entry.kind != QueueKind::Suggested)
        .take(KEPT_UPCOMING)
        .filter_map(save)
        .collect();
    Resume {
        provider: provider.to_owned(),
        position: 0.,
        origin: None,
        origins,
        current,
        past,
        upcoming,
        manual: 0,
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
    let mut entries = unknown(upcoming, source);
    entries.extend(
        source
            .iter()
            .filter(|entry| current.is_none_or(|playing| entry.uid != playing.uid))
            .cloned(),
    );

    fastrand::shuffle(&mut entries);
    *upcoming = entries.into();
}

fn restore(upcoming: &mut VecDeque<Entry>, source: &[Entry], current: Option<&Entry>) {
    let extra = unknown(upcoming, source);
    let at = current.and_then(|playing| source.iter().position(|entry| entry.uid == playing.uid));
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
    preserve: impl Fn(&T) -> bool,
) -> bool {
    let Some(selected) = upcoming.remove(index) else {
        return false;
    };

    past.extend(current.replace(selected));
    let mut kept = VecDeque::new();
    for entry in upcoming.drain(..index) {
        match preserve(&entry) {
            true => kept.push_back(entry),
            false => past.push(entry),
        }
    }
    kept.append(upcoming);
    *upcoming = kept;
    true
}

fn trim<T>(past: &mut Vec<T>, limit: usize) {
    past.drain(..past.len().saturating_sub(limit));
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
    kind: QueueKind,
) -> impl Iterator<Item = Entry> {
    tracks
        .into_iter()
        .map(move |track| Entry::new(track, origin.clone(), kind))
}

/// The collection order a play order keeps for reset and unshuffle.
fn source_of(entries: impl IntoIterator<Item = Entry>) -> Vec<Entry> {
    entries
        .into_iter()
        .filter(|entry| entry.kind == QueueKind::Context)
        .collect()
}

/// The entries `upcoming` holds that are not occurrences from `source`, such as a song added by
/// hand that the collection also has.
fn unknown(upcoming: &mut VecDeque<Entry>, source: &[Entry]) -> Vec<Entry> {
    let known: HashSet<&str> = source.iter().map(|entry| entry.uid.as_str()).collect();
    upcoming
        .drain(..)
        .filter(|entry| !known.contains(entry.uid.as_str()))
        .collect()
}

/// One play order around the current song. Entries retain their provenance when moved;
/// `source` keeps only the original collection order for reset and unshuffle.
pub struct Queue {
    past: Vec<Entry>,
    current: Option<Entry>,
    upcoming: VecDeque<Entry>,
    source: Vec<Entry>,
    /// Derived runs are rebuilt on edits, so playback-clock redraws never scan the playlist.
    pub(crate) sections: QueueSections,
    shuffle: bool,
    revision: u64,
    /// The toast counting tracks removed one by one while it may still be up.
    removed: Option<Reported<Vec<Taken>>>,
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
            SessionEvent::SignedIn
            | SessionEvent::Reconnected
            | SessionEvent::WentOffline
            | SessionEvent::LocalChanged => {}
        })
        .detach();

        let shuffle = settings.read(cx).shuffle();

        Self {
            past: Vec::new(),
            current: None,
            upcoming: VecDeque::new(),
            source: Vec::new(),
            sections: QueueSections::new(0, false, []),
            shuffle,
            revision: 0,
            removed: None,
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
        self.settings
            .update(cx, |settings, cx| settings.set_shuffle(on, cx));
        self.reorder();
        self.changed(cx);
    }

    /// Rebuilds collection order while keeping additions and recommendations out of the source.
    fn reorder(&mut self) {
        let mut suggested = VecDeque::new();
        let mut manual = VecDeque::new();
        self.upcoming.retain(|entry| match entry.kind {
            QueueKind::Context => true,
            QueueKind::Queued => {
                manual.push_back(entry.clone());
                false
            }
            QueueKind::Suggested => {
                suggested.push_back(entry.clone());
                false
            }
        });
        match self.shuffle {
            true => scramble(&mut self.upcoming, &self.source, self.current.as_ref()),
            false => restore(&mut self.upcoming, &self.source, self.current.as_ref()),
        }
        manual.append(&mut self.upcoming);
        self.upcoming = manual;
        self.upcoming.append(&mut suggested);
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        trim(&mut self.past, PAST_LIMIT);
        self.sections = QueueSections::new(
            self.past.len(),
            self.current.is_some(),
            self.upcoming.iter().map(|entry| entry.kind),
        );
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
                    self.upcoming.iter(),
                )
            });
        self.settings
            .update(cx, |settings, cx| settings.set_resume(resume, cx));
    }

    /// Restores a saved queue. A queue saved by an older version has one origin for all of it,
    /// which goes to every track except the ones queued by hand.
    pub(crate) fn revive(&mut self, resume: Resume, cx: &mut Context<Self>) -> Option<Track> {
        let origins = resume.origins;
        let entry = |stub: Stub| {
            let kind = stub.kind;
            let origin = stub.from.and_then(|index| origins.get(index).cloned());
            Entry::new(hydrate(stub), origin, kind)
        };
        self.past = resume.past.into_iter().map(entry).collect();
        self.current = resume.current.map(entry);
        self.upcoming = resume.upcoming.into_iter().map(entry).collect();
        for entry in self.upcoming.iter_mut().take(resume.manual) {
            entry.kind = QueueKind::Queued;
        }
        self.suggested += 1;
        if let Some(origin) = resume.origin {
            self.past
                .iter_mut()
                .chain(self.current.as_mut())
                .chain(self.upcoming.iter_mut())
                .filter(|entry| entry.kind == QueueKind::Context)
                .for_each(|entry| entry.origin = Some(origin.clone()));
        }
        self.source = source_of(
            self.past
                .iter()
                .chain(self.current.as_ref())
                .chain(self.upcoming.iter())
                .cloned(),
        );
        self.changed(cx);
        self.current().cloned()
    }

    /// Drops the recommendations, saying whether there were any.
    fn drop_suggested(&mut self) -> bool {
        let before = self.upcoming.len();
        self.upcoming
            .retain(|entry| entry.kind != QueueKind::Suggested);
        before != self.upcoming.len()
    }

    fn purge(&mut self, cx: &mut Context<Self>) {
        let suggested = self.drop_suggested();
        self.suggested += 1;
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
        self.past.iter().map(|entry| entry.track.as_ref())
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.as_ref().map(|entry| entry.track.as_ref())
    }

    /// The collection the current track was queued from. `None` while nothing plays or when the
    /// user added the current track on its own.
    pub fn origin(&self) -> Option<&Origin> {
        self.current.as_ref()?.origin.as_ref()
    }

    /// The occurrence identity of the song playing, independent of its queue position.
    pub(crate) fn current_uid(&self) -> Option<&str> {
        self.current.as_ref().map(|entry| entry.uid.as_str())
    }

    pub(crate) fn connect_current(&self) -> Option<QueuedTrack> {
        connect_entry(self.current.as_ref()?)
    }

    /// Recent provider tracks with identities that survive a move from current to past.
    pub(crate) fn connect_previous(&self) -> impl DoubleEndedIterator<Item = QueuedTrack> + '_ {
        self.past.iter().filter_map(connect_entry)
    }

    /// The upcoming play order reported to other clients. Local files and suggestions stay out,
    /// since the suggestions are this app's own and change whenever the radio refills.
    pub(crate) fn connect_upcoming(&self) -> impl Iterator<Item = QueuedTrack> + '_ {
        self.upcoming
            .iter()
            .filter(|entry| entry.kind != QueueKind::Suggested)
            .filter_map(connect_entry)
    }

    /// Every track the queue holds, for reading a remote queue without asking the provider
    /// again for what is already here.
    pub(crate) fn tracks(&self) -> impl Iterator<Item = &Track> {
        self.past
            .iter()
            .chain(self.current.as_ref())
            .chain(self.upcoming.iter())
            .map(|entry| entry.track.as_ref())
    }

    /// The underlying collection, retained while a hand-queued track is playing.
    pub(crate) fn context_origin(&self) -> Option<&Origin> {
        self.source.iter().find_map(|entry| entry.origin.as_ref())
    }

    /// Where the current track sits in the collection the queue was started from. `None` when it
    /// is not in that collection, as for a track queued by hand.
    pub fn place(&self) -> Option<usize> {
        let current = self.current.as_ref()?;
        self.source
            .iter()
            .position(|entry| entry.uid == current.uid)
    }

    /// The full upcoming play order, including recommendations.
    pub fn upcoming(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.upcoming.iter().map(|entry| entry.track.as_ref())
    }

    /// How many songs are left to play before the recommendations.
    pub fn ahead(&self) -> usize {
        self.upcoming
            .iter()
            .filter(|entry| entry.kind != QueueKind::Suggested)
            .count()
    }

    pub fn similar(&self) -> impl Iterator<Item = &Track> {
        self.upcoming
            .iter()
            .filter(|entry| entry.kind == QueueKind::Suggested)
            .map(|entry| entry.track.as_ref())
    }

    pub fn kind(&self, position: QueuePosition) -> Option<QueueKind> {
        self.entry(position).map(|entry| entry.kind)
    }

    fn entry(&self, position: QueuePosition) -> Option<&Entry> {
        match position {
            QueuePosition::Past(at) => self.past.get(at),
            QueuePosition::Current => self.current.as_ref(),
            QueuePosition::Upcoming(at) => self.upcoming.get(at),
        }
    }

    /// Resolves an occurrence identity without depending on a previously rendered index.
    pub fn position(&self, uid: &str) -> Option<QueuePosition> {
        if self.current_uid() == Some(uid) {
            return Some(QueuePosition::Current);
        }
        self.past
            .iter()
            .position(|entry| entry.uid == uid)
            .map(QueuePosition::Past)
            .or_else(|| {
                self.upcoming
                    .iter()
                    .position(|entry| entry.uid == uid)
                    .map(QueuePosition::Upcoming)
            })
    }

    pub fn uid(&self, position: QueuePosition) -> Option<&str> {
        self.entry(position).map(|entry| entry.uid.as_str())
    }

    /// Replaces the suggestions with `tracks` from the radio `origin`.
    pub fn suggest(&mut self, tracks: Vec<Track>, origin: Origin, cx: &mut Context<Self>) {
        self.drop_suggested();
        self.suggested += 1;
        self.upcoming
            .extend(tagged(tracks, Some(origin), QueueKind::Suggested));
        self.changed(cx);
    }

    /// Adds suggestions after the ones already there, for a station topped up before the queue
    /// has played through it.
    pub fn extend_similar(&mut self, tracks: Vec<Track>, origin: Origin, cx: &mut Context<Self>) {
        if tracks.is_empty() {
            return;
        }
        self.upcoming
            .extend(tagged(tracks, Some(origin), QueueKind::Suggested));
        self.changed(cx);
    }

    pub fn clear_similar(&mut self, cx: &mut Context<Self>) {
        if !self.drop_suggested() {
            return;
        }
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
        if sift(
            &mut self.past,
            &mut self.current,
            &mut self.upcoming,
            &mut self.source,
            |entry| !matches(entry),
        ) {
            self.changed(cx);
        }
        current_removed
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
        self.upcoming
            .front()
            .is_some_and(|entry| entry.kind == QueueKind::Suggested)
    }

    pub fn has_previous(&self) -> bool {
        !self.past.is_empty()
    }

    /// Drops everything queued, keeping the suggestions, and posts a toast whose Undo puts it
    /// back. Every clear gets a toast and an Undo of its own.
    pub fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        let mut removed = Vec::new();
        let mut kept = 0;
        self.upcoming.retain(|entry| {
            if entry.kind == QueueKind::Suggested {
                kept += 1;
                return true;
            }
            removed.push(Taken {
                entry: entry.clone(),
                index: kept,
                batch: self.suggested,
            });
            false
        });
        if removed.is_empty() {
            return;
        }
        self.changed(cx);
        let snapshot = Rc::new(RefCell::new(Some(removed)));
        let toast = Self::undo_toast(
            Toast::new(Outcome::Plain, "toast-queue-cleared"),
            snapshot,
            cx,
        );
        Toasts::post(toast, cx);
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

        let index = self.place().unwrap_or_default();
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
        self.fill(
            tagged(tracks, origin, QueueKind::Context).collect(),
            index,
            cx,
        )
    }

    /// Rebuilds play order while retaining only collection entries for reset and unshuffle.
    fn fill(&mut self, entries: Vec<Entry>, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        if index >= entries.len() {
            return None;
        }

        self.source = source_of(entries.iter().cloned());
        let mut past = entries;
        self.upcoming = past.split_off(index + 1).into();
        self.suggested += 1;
        self.current = past.pop();
        if self.shuffle {
            self.reorder();
            past.clear();
        }
        self.past = past;
        self.changed(cx);
        self.current().cloned()
    }

    /// Applies a remote queue window while keeping the current entry and playback position.
    /// `reach` says how much of this queue the window stands for, so what lies beyond it stays,
    /// and so do the suggestions, which are never reported.
    pub(crate) fn replace_remote(
        &mut self,
        past: Vec<(Track, bool)>,
        current: bool,
        upcoming: Vec<(Track, bool)>,
        reach: Reach,
        cx: &mut Context<Self>,
    ) {
        let origin = self.origin().or_else(|| self.context_origin()).cloned();
        let old_past = std::mem::take(&mut self.past);
        let (suggested, old_upcoming): (Vec<_>, Vec<_>) = self
            .upcoming
            .drain(..)
            .partition(|entry| entry.kind == QueueKind::Suggested);
        let (earlier, left_past, mut known, left_upcoming, beyond) = match reach {
            Reach::Reported {
                past: shown_past,
                upcoming: shown_upcoming,
            } => {
                let mut seen_past = old_past;
                let earlier = seen_past
                    .drain(..seen_start(&seen_past, shown_past))
                    .collect();
                let mut seen_upcoming = old_upcoming;
                let beyond = seen_upcoming.split_off(seen_end(&seen_upcoming, shown_upcoming));
                // what the report left out, such as a local file, stays where it was
                let (left_past, seen_past): (Vec<_>, Vec<_>) =
                    seen_past.into_iter().partition(|entry| !reported(entry));
                let (left_upcoming, seen_upcoming): (Vec<_>, Vec<_>) = seen_upcoming
                    .into_iter()
                    .partition(|entry| !reported(entry));
                let known: Vec<_> = seen_past.into_iter().chain(seen_upcoming).collect();
                (earlier, left_past, known, left_upcoming, beyond)
            }
            // the collection loaded for a handover may hold the window's history ahead, as it
            // does when it was shuffled, so both halves are looked for across all of it
            Reach::Handed => {
                let split = old_past.len();
                let pool: Vec<_> = old_past.into_iter().chain(old_upcoming).collect();
                let named = named(&pool, past.iter().chain(&upcoming).map(|(track, _)| track));
                let mut known = Vec::new();
                let mut earlier = Vec::new();
                let mut beyond = Vec::new();
                for (index, (entry, named)) in pool.into_iter().zip(named).enumerate() {
                    match (named, index < split) {
                        (true, _) => known.push(entry),
                        (false, true) => earlier.push(entry),
                        (false, false) => beyond.push(entry),
                    }
                }
                (earlier, Vec::new(), known, Vec::new(), beyond)
            }
        };
        let mut identity =
            |track: &Track| match known.iter().position(|entry| entry.track.id == track.id) {
                Some(index) => known.remove(index).uid,
                None => queue_uid(),
            };
        let past_len = past.len();
        let mut entries: Vec<_> = past
            .into_iter()
            .chain(upcoming)
            .map(|(track, queued)| {
                let kind = match queued {
                    true => QueueKind::Queued,
                    false => QueueKind::Context,
                };
                Entry::with_uid(
                    identity(&track),
                    track,
                    (!queued).then(|| origin.clone()).flatten(),
                    kind,
                )
            })
            .collect();
        let window: Vec<_> = entries.split_off(past_len);
        self.past = earlier
            .into_iter()
            .chain(left_past)
            .chain(entries)
            .collect();
        self.upcoming = window
            .into_iter()
            .chain(left_upcoming)
            .chain(beyond)
            .chain(suggested)
            .collect();
        if let Some(entry) = self.current.as_mut() {
            entry.kind = match current {
                true => QueueKind::Queued,
                false => QueueKind::Context,
            };
        }
        let queued: HashSet<_> = self
            .past
            .iter()
            .chain(&self.current)
            .chain(&self.upcoming)
            .filter(|entry| entry.kind == QueueKind::Queued)
            .map(|entry| &entry.uid)
            .collect();
        self.source.retain(|entry| !queued.contains(&entry.uid));
        self.suggested += 1;
        self.changed(cx);
    }

    /// Inserts hand-queued tracks at `placement`, keeping their order and provenance.
    pub fn insert(
        &mut self,
        tracks: impl IntoIterator<Item = Track>,
        origin: Option<Origin>,
        placement: QueuePlacement,
        cx: &mut Context<Self>,
    ) {
        let (at, kind) = match placement {
            QueuePlacement::Next => (0, QueueKind::Queued),
            QueuePlacement::End => (self.manual_end(), QueueKind::Queued),
            // what plays once the collection has run out is not part of the hand-queued run
            QueuePlacement::Last => (self.context_end(), QueueKind::Context),
            QueuePlacement::Gap(gap) => {
                // a song dropped among the suggestions lands at the end of the queue instead
                let at = gap.min(self.context_end());
                let kind = match joins_manual(self.upcoming.iter(), at) {
                    true => QueueKind::Queued,
                    false => QueueKind::Context,
                };
                (at, kind)
            }
        };
        self.insert_entries(at, tagged(tracks, origin, kind));
        self.changed(cx);
    }

    /// The gap right after the hand-queued run, where Add to queue puts a song. Hand-queued
    /// songs only ever sit in that run, which songs replayed from the history may precede.
    fn manual_end(&self) -> usize {
        self.upcoming
            .iter()
            .rposition(|entry| entry.kind == QueueKind::Queued)
            .map_or(0, |at| at + 1)
    }

    /// Adds the next stretch of the station the queue was started from, both to the source and
    /// after what is queued.
    pub(crate) fn extend_context(
        &mut self,
        tracks: Vec<Track>,
        origin: Origin,
        cx: &mut Context<Self>,
    ) {
        let at = self.context_end();
        let entries: Vec<Entry> = tagged(tracks, Some(origin), QueueKind::Context).collect();
        self.source.extend(entries.iter().cloned());
        self.insert_entries(at, entries);
        self.changed(cx);
    }

    /// Inserts `entries` at `at`, keeping their order.
    fn insert_entries(&mut self, at: usize, entries: impl IntoIterator<Item = Entry>) {
        for (offset, entry) in entries.into_iter().enumerate() {
            self.upcoming.insert(at + offset, entry);
        }
    }

    /// The gap before trailing recommendations, even after earlier recommendations were replayed.
    fn context_end(&self) -> usize {
        self.upcoming
            .iter()
            .rposition(|entry| entry.kind != QueueKind::Suggested)
            .map_or(0, |at| at + 1)
    }

    /// Moves a song to `gap`. Landing inside or right after the hand-queued run joins it and
    /// leaving that run leaves it, and a suggestion dragged ahead of the queue's end becomes part
    /// of the queue, so the next radio refill keeps it.
    pub fn move_upcoming_to_gap(&mut self, from: usize, gap: usize, cx: &mut Context<Self>) {
        // only a suggestion moves among the suggestions, anything else stops at the queue's end
        let gap = match self.upcoming.get(from).map(|entry| entry.kind) {
            Some(QueueKind::Suggested) | None => gap,
            Some(_) => gap.min(self.context_end()),
        };
        let to = gap_target(from, gap, self.upcoming.len());
        if !move_item(&mut self.upcoming, from, to) {
            return;
        }
        let others = self
            .upcoming
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != to)
            .map(|(_, entry)| entry);
        let joins = joins_manual(others, to);
        let in_body = self
            .upcoming
            .iter()
            .skip(to + 1)
            .any(|entry| entry.kind != QueueKind::Suggested);
        if let Some(entry) = self.upcoming.get_mut(to) {
            entry.kind = match (joins, entry.kind) {
                (true, _) => QueueKind::Queued,
                (false, QueueKind::Queued) => QueueKind::Context,
                (false, QueueKind::Suggested) if in_body => QueueKind::Context,
                (false, kind) => kind,
            };
            // a hand-queued song is no longer the collection's, or a reshuffle would bring it
            // back from the collection a second time
            if entry.kind == QueueKind::Queued {
                let uid = entry.uid.clone();
                self.source.retain(|kept| kept.uid != uid);
            }
        }
        self.changed(cx);
    }

    /// Removes an upcoming song or suggestion, retaining its position for Undo.
    pub fn remove(&mut self, position: QueuePosition, cx: &mut Context<Self>) {
        let QueuePosition::Upcoming(index) = position else {
            return;
        };
        let Some(entry) = self.upcoming.remove(index) else {
            return;
        };
        self.changed(cx);
        self.report_removal(
            Taken {
                entry,
                index,
                batch: self.suggested,
            },
            cx,
        );
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
        // A reset or shuffle may have restored the same occurrence already. Identities
        // distinguish those from separate additions of the same song.
        let restoring: HashSet<_> = removed.iter().map(|taken| &taken.entry.uid).collect();
        self.upcoming
            .retain(|entry| !restoring.contains(&entry.uid));
        for Taken {
            entry,
            index,
            batch,
        } in removed.into_iter().rev()
        {
            if (entry.kind == QueueKind::Suggested && batch != self.suggested)
                || self.current_uid() == Some(&entry.uid)
                || self.past.iter().any(|played| played.uid == entry.uid)
            {
                continue;
            }
            self.upcoming.insert(index.min(self.upcoming.len()), entry);
        }
        self.changed(cx);
    }

    pub fn next(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        let next = self.upcoming.pop_front()?;
        if let Some(played) = self.current.replace(next) {
            self.past.push(played);
        }
        self.changed(cx);
        self.current().cloned()
    }

    /// Starts the queue over from its first song, for repeat all. What was queued by hand joins
    /// the collection it played in, so the next loop has no stale hand-queued run.
    pub fn rewind(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.drop_suggested();
        let mut entries = std::mem::take(&mut self.past);
        entries.extend(self.current.take());
        entries.extend(self.upcoming.drain(..));
        for entry in &mut entries {
            entry.kind = QueueKind::Context;
        }
        self.fill(entries, 0, cx)
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        let index = self.past.iter().rposition(|entry| entry.track.playable)?;
        self.select(QueuePosition::Past(index), cx)
    }

    /// Selects a song in play order. Replaying history queues the interrupted songs first;
    /// selecting the collection preserves manual additions, while selecting a suggestion skips them.
    pub fn select(&mut self, position: QueuePosition, cx: &mut Context<Self>) -> Option<Track> {
        match position {
            QueuePosition::Past(index) => {
                if !select_past(&mut self.past, &mut self.current, &mut self.upcoming, index) {
                    return None;
                }
            }
            QueuePosition::Upcoming(index) => {
                let preserve = self.upcoming.get(index)?.kind == QueueKind::Context;
                select_upcoming(
                    &mut self.past,
                    &mut self.current,
                    &mut self.upcoming,
                    index,
                    |entry| preserve && entry.kind == QueueKind::Queued,
                );
            }
            _ => return None,
        }
        self.changed(cx);
        self.current().cloned()
    }
}

/// Whether a song put at `at` joins the hand-queued run in `upcoming`: inside it or right after
/// it, or at the very top when there is no run.
fn joins_manual<'a>(upcoming: impl Iterator<Item = &'a Entry>, at: usize) -> bool {
    let run = upcoming
        .enumerate()
        .filter(|(_, entry)| entry.kind == QueueKind::Queued)
        .fold(None, |run, (index, _)| match run {
            None => Some((index, index)),
            Some((first, _)) => Some((first, index)),
        });
    match run {
        Some((first, last)) => (first..=last + 1).contains(&at),
        None => at == 0,
    }
}

/// Whether a report to other devices carries `entry`, which it does for a provider song.
fn reported(entry: &Entry) -> bool {
    entry.id().is_some_and(|id| !music::is_local_id(id))
}

/// Where the last `shown` reported songs of `past` begin, the part of the history a report
/// carried.
fn seen_start(past: &[Entry], shown: usize) -> usize {
    let mut left = shown;
    for (index, entry) in past.iter().enumerate().rev() {
        if left == 0 {
            return index + 1;
        }
        left -= usize::from(reported(entry));
    }
    0
}

/// Where the first `shown` reported songs of `upcoming` end, the part of the queue a report
/// carried.
fn seen_end(upcoming: &[Entry], shown: usize) -> usize {
    let mut left = shown;
    for (index, entry) in upcoming.iter().enumerate() {
        if left == 0 {
            return index;
        }
        left -= usize::from(reported(entry));
    }
    upcoming.len()
}

/// Which of `entries` a remote window names, in their order. A song the window names twice
/// takes two occurrences.
fn named<'a>(entries: &[Entry], window: impl Iterator<Item = &'a Track>) -> Vec<bool> {
    let mut wanted: HashMap<&str, usize> = HashMap::new();
    for id in window.filter_map(|track| track.id.as_deref()) {
        *wanted.entry(id).or_default() += 1;
    }
    entries
        .iter()
        .map(|entry| {
            let Some(count) = entry.id().and_then(|id| wanted.get_mut(id)) else {
                return false;
            };
            let named = *count > 0;
            *count = count.saturating_sub(1);
            named
        })
        .collect()
}

/// Makes a fresh identity for a newly queued occurrence, never for a change of position.
fn queue_uid() -> String {
    format!("sonora-{:016x}", fastrand::u64(..))
}

/// Excludes local files from the provider's queue report while retaining occurrence identities.
fn connect_entry(entry: &Entry) -> Option<QueuedTrack> {
    let id = entry.id().filter(|id| !music::is_local_id(id))?;
    Some(QueuedTrack {
        id: id.to_owned(),
        uid: entry.uid.clone(),
        queued: entry.kind == QueueKind::Queued,
        added_by: entry
            .track
            .added_by
            .as_ref()
            .map(|person| person.id.clone()),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::rc::Rc;
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
            uid: format!("test-{id}"),
            track: Rc::new(track(id)),
            origin: None,
            kind: super::QueueKind::Context,
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
        let mut past = vec![entry("1"), entry("2"), entry("3")];
        let mut current = Some(entry("4"));
        let mut queued = entry("5");
        queued.kind = super::QueueKind::Queued;
        let mut upcoming = VecDeque::from([queued, entry("6")]);

        assert!(select_past(&mut past, &mut current, &mut upcoming, 1));
        assert_eq!(past, [entry("1")]);
        assert_eq!(current, Some(entry("2")));
        assert_eq!(ids(&upcoming), ["3", "4", "5", "6"]);
        assert_eq!(
            upcoming.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
            [
                super::QueueKind::Context,
                super::QueueKind::Context,
                super::QueueKind::Queued,
                super::QueueKind::Context
            ]
        );
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

        assert!(select_upcoming(
            &mut past,
            &mut current,
            &mut upcoming,
            1,
            |_| false
        ));
        assert_eq!(past, [1, 2, 3]);
        assert_eq!(current, Some(4));
        assert_eq!(upcoming, [5]);

        // Selecting collection playback keeps explicit additions available afterwards.
        let mut past = vec![1];
        let mut current = Some(2);
        let mut upcoming = VecDeque::from([3, 4, 5, 6]);
        assert!(select_upcoming(
            &mut past,
            &mut current,
            &mut upcoming,
            2,
            |entry| *entry == 3
        ));
        assert_eq!(past, [1, 2, 4]);
        assert_eq!(current, Some(5));
        assert_eq!(upcoming, [3, 6]);
    }

    #[test]
    fn selecting_invalid_upcoming_track_keeps_the_queue() {
        let mut past = vec![1];
        let mut current = Some(2);
        let mut upcoming = VecDeque::from([3, 4]);

        assert!(!select_upcoming(
            &mut past,
            &mut current,
            &mut upcoming,
            2,
            |_| false
        ));
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
        let mut upcoming = listing(300);
        upcoming[1].kind = super::QueueKind::Queued;
        let current = entry("now");

        let resume = record("spotify", &past, Some(&current), upcoming.iter());

        assert_eq!(resume.provider, "spotify");
        assert_eq!(resume.position, 0.);
        assert_eq!(resume.current.map(|stub| stub.id), Some("now".to_owned()));
        assert_eq!(resume.past.len(), 20);
        assert_eq!(resume.past.first().map(|stub| stub.id.as_str()), Some("40"));
        assert_eq!(resume.upcoming.len(), 200);
        assert_eq!(resume.upcoming[0].kind, super::QueueKind::Context);
        assert_eq!(resume.upcoming[1].kind, super::QueueKind::Queued);
        assert_eq!(resume.manual, 0);
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
