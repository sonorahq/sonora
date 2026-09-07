use std::collections::{HashSet, VecDeque};
use std::time::Duration;

use gpui::{Context, Entity};
use music::{ArtistRef, Track};
use serde::{Deserialize, Serialize};

use crate::{AppSettings, Session, SessionEvent};

const PAST_LIMIT: usize = 500;
const KEPT_PAST: usize = 20;
const KEPT_UPCOMING: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Resume {
    pub(crate) provider: String,
    pub(crate) position: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) origin: Option<crate::Origin>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) current: Option<Stub>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) past: Vec<Stub>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) upcoming: Vec<Stub>,
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
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Named {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
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

fn record<'a>(
    provider: &str,
    past: &[Track],
    current: Option<&Track>,
    upcoming: impl Iterator<Item = &'a Track>,
) -> Resume {
    Resume {
        provider: provider.to_owned(),
        position: 0.,
        origin: None,
        current: current.and_then(stub),
        past: past[past.len().saturating_sub(KEPT_PAST)..]
            .iter()
            .filter_map(stub)
            .collect(),
        upcoming: upcoming.take(KEPT_UPCOMING).filter_map(stub).collect(),
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

fn scramble(upcoming: &mut VecDeque<Track>, source: &[Track], current: Option<&Track>) {
    let known: HashSet<&str> = source
        .iter()
        .filter_map(|track| track.id.as_deref())
        .collect();
    let mut tracks: Vec<Track> = upcoming
        .drain(..)
        .filter(|track| !track.id.as_deref().is_some_and(|id| known.contains(id)))
        .collect();

    let mut playing = current.and_then(|current| current.id.clone());
    tracks.extend(
        source
            .iter()
            .filter(|track| match playing.as_deref() == track.id.as_deref() {
                true => {
                    playing = None;
                    false
                }
                false => true,
            })
            .cloned(),
    );

    fastrand::shuffle(&mut tracks);
    *upcoming = tracks.into();
}

fn restore(upcoming: &mut VecDeque<Track>, source: &[Track], current: Option<&Track>) {
    let known: HashSet<&str> = source
        .iter()
        .filter_map(|track| track.id.as_deref())
        .collect();
    let extra: Vec<Track> = upcoming
        .drain(..)
        .filter(|track| !track.id.as_deref().is_some_and(|id| known.contains(id)))
        .collect();

    let at = current
        .and_then(|current| current.id.as_deref())
        .and_then(|id| {
            source
                .iter()
                .position(|track| track.id.as_deref() == Some(id))
        });
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

pub struct Queue {
    past: Vec<Track>,
    current: Option<Track>,
    upcoming: VecDeque<Track>,
    source: Vec<Track>,
    similar: usize,
    shuffle: bool,
    revision: u64,
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
            similar: 0,
            shuffle,
            revision: 0,
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
        let mut suggested = self.upcoming.split_off(self.queued());
        match on {
            true => scramble(&mut self.upcoming, &self.source, self.current.as_ref()),
            false => restore(&mut self.upcoming, &self.source, self.current.as_ref()),
        }
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
                )
            });
        self.settings
            .update(cx, |settings, cx| settings.set_resume(resume, cx));
    }

    pub(crate) fn revive(&mut self, resume: Resume, cx: &mut Context<Self>) -> Option<Track> {
        self.past = resume.past.into_iter().map(hydrate).collect();
        self.current = resume.current.map(hydrate);
        self.upcoming = resume.upcoming.into_iter().map(hydrate).collect();
        self.similar = 0;
        self.source = self
            .past
            .iter()
            .chain(self.current.as_ref())
            .chain(self.upcoming.iter())
            .cloned()
            .collect();
        self.changed(cx);
        self.current.clone()
    }

    fn purge(&mut self, cx: &mut Context<Self>) {
        let suggested = self.similar > 0;
        self.upcoming.truncate(self.queued());
        self.similar = 0;
        let sifted = sift(
            &mut self.past,
            &mut self.current,
            &mut self.upcoming,
            &mut self.source,
            local,
        );
        if suggested || sifted {
            self.changed(cx);
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn past(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.past.iter()
    }

    pub fn current(&self) -> Option<&Track> {
        self.current.as_ref()
    }

    fn queued(&self) -> usize {
        self.upcoming.len() - self.similar
    }

    pub fn upcoming(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.upcoming.range(..self.queued())
    }

    pub fn similar(&self) -> impl ExactSizeIterator<Item = &Track> {
        self.upcoming.range(self.queued()..)
    }

    pub fn suggest(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        self.upcoming.truncate(self.queued());
        self.similar = tracks.len();
        self.upcoming.extend(tracks);
        self.changed(cx);
    }

    pub fn clear_similar(&mut self, cx: &mut Context<Self>) {
        if self.similar == 0 {
            return;
        }
        self.upcoming.truncate(self.queued());
        self.similar = 0;
        self.changed(cx);
    }

    pub fn ids(&self) -> HashSet<String> {
        self.past
            .iter()
            .chain(self.current.as_ref())
            .chain(self.upcoming.iter())
            .filter_map(|track| track.id.clone())
            .collect()
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

    pub fn has_previous(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn clear_upcoming(&mut self, cx: &mut Context<Self>) {
        if self.queued() == 0 {
            return;
        }
        self.upcoming.drain(..self.queued());
        self.changed(cx);
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.past.clear();
        self.current = None;
        self.upcoming.clear();
        self.source.clear();
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
            .and_then(|current| self.source.iter().position(|track| track == current))
            .unwrap_or_default();
        let source = self.source.clone();
        self.start(source, index, cx)
    }

    pub fn start(
        &mut self,
        tracks: Vec<Track>,
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<Track> {
        if index >= tracks.len() {
            return None;
        }

        self.source = tracks.clone();
        let mut past = tracks;
        self.upcoming = past.split_off(index + 1).into();
        self.similar = 0;
        self.current = past.pop();
        if self.shuffle {
            scramble(&mut self.upcoming, &self.source, self.current.as_ref());
            past.clear();
        }
        self.past = past;
        self.changed(cx);
        self.current.clone()
    }

    pub fn append(&mut self, track: Track, cx: &mut Context<Self>) {
        self.append_all([track], cx);
    }

    pub fn append_all(&mut self, tracks: impl IntoIterator<Item = Track>, cx: &mut Context<Self>) {
        self.insert_upcoming(self.queued(), tracks, cx);
    }

    pub(crate) fn extend_context(&mut self, tracks: Vec<Track>, cx: &mut Context<Self>) {
        let at = self.queued();
        self.source.extend(tracks.iter().cloned());
        for (offset, track) in tracks.into_iter().enumerate() {
            self.upcoming.insert(at + offset, track);
        }
        self.changed(cx);
    }

    pub fn insert_upcoming(
        &mut self,
        gap: usize,
        tracks: impl IntoIterator<Item = Track>,
        cx: &mut Context<Self>,
    ) {
        let at = gap.min(self.queued());
        for (offset, track) in tracks.into_iter().enumerate() {
            self.upcoming.insert(at + offset, track);
        }
        self.changed(cx);
    }

    pub fn prepend(&mut self, track: Track, cx: &mut Context<Self>) {
        self.prepend_all([track], cx);
    }

    pub fn prepend_all(&mut self, tracks: impl IntoIterator<Item = Track>, cx: &mut Context<Self>) {
        let mut upcoming = tracks.into_iter().collect::<VecDeque<_>>();
        upcoming.append(&mut self.upcoming);
        self.upcoming = upcoming;
        self.changed(cx);
    }

    pub fn move_upcoming(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if move_item(&mut self.upcoming, from, to) {
            self.changed(cx);
        }
    }

    pub fn move_upcoming_to_gap(&mut self, from: usize, gap: usize, cx: &mut Context<Self>) {
        let to = gap_target(from, gap, self.queued());
        self.move_upcoming(from, to, cx);
    }

    pub fn remove_upcoming(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.queued() && self.upcoming.remove(index).is_some() {
            self.changed(cx);
        }
    }

    pub fn remove_similar(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.similar {
            return;
        }
        if self.upcoming.remove(self.queued() + index).is_some() {
            self.similar -= 1;
            self.changed(cx);
        }
    }

    pub fn next(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.similar -= usize::from(self.queued() == 0 && self.similar > 0);
        let next = self.upcoming.pop_front()?;
        if let Some(played) = self.current.replace(next) {
            self.past.push(played);
        }
        self.changed(cx);
        self.current.clone()
    }

    pub fn rewind(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        self.upcoming.truncate(self.queued());
        self.similar = 0;
        let mut tracks = std::mem::take(&mut self.past);
        tracks.extend(self.current.take());
        tracks.extend(self.upcoming.drain(..));
        self.start(tracks, 0, cx)
    }

    pub fn previous(&mut self, cx: &mut Context<Self>) -> Option<Track> {
        let index = self.past.iter().rposition(|track| track.playable)?;
        if !select_past(&mut self.past, &mut self.current, &mut self.upcoming, index) {
            return None;
        }
        self.changed(cx);
        self.current.clone()
    }

    pub fn play_past(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        if !select_past(&mut self.past, &mut self.current, &mut self.upcoming, index) {
            return None;
        }
        self.changed(cx);
        self.current.clone()
    }

    pub fn play_upcoming(&mut self, index: usize, cx: &mut Context<Self>) -> Option<Track> {
        if index >= self.queued()
            || !select_upcoming(&mut self.past, &mut self.current, &mut self.upcoming, index)
        {
            return None;
        }
        self.changed(cx);
        self.current.clone()
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
        self.similar -= index + 1;
        self.changed(cx);
        self.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use music::{ArtistRef, Track};

    use super::{
        gap_target, hydrate, in_order, local, move_item, record, restore, scramble, select_past,
        select_upcoming, sift, stub, trim,
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

    fn ids(tracks: &VecDeque<Track>) -> Vec<String> {
        tracks
            .iter()
            .map(|track| track.name.clone())
            .collect::<Vec<_>>()
    }

    fn listing(count: usize) -> Vec<Track> {
        (0..count).map(|index| track(&index.to_string())).collect()
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
        let mut upcoming: VecDeque<Track> = source.iter().cloned().collect();

        scramble(&mut upcoming, &source, None);
        restore(&mut upcoming, &source, None);

        assert_eq!(ids(&upcoming), ids(&source.iter().cloned().collect()));
    }

    #[test]
    fn restoring_puts_unknown_tracks_last() {
        let source = vec![track("a"), track("b"), track("c")];
        let mut upcoming = VecDeque::from(vec![
            track("queued-one"),
            track("c"),
            track("queued-two"),
            track("a"),
        ]);

        restore(&mut upcoming, &source, None);

        assert_eq!(ids(&upcoming), ["a", "b", "c", "queued-one", "queued-two"]);
    }

    #[test]
    fn restoring_keeps_both_copies_of_a_repeated_track() {
        let source = vec![track("a"), track("b"), track("a")];
        let mut upcoming = VecDeque::from(vec![track("a"), track("a"), track("b")]);

        restore(&mut upcoming, &source, None);

        assert_eq!(ids(&upcoming), ["a", "b", "a"]);
    }

    #[test]
    fn restoring_continues_after_the_current_track() {
        let source = listing(12);
        let mut upcoming: VecDeque<Track> = source.iter().cloned().collect();

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
        let current = track("now");

        let resume = record("spotify", &past, Some(&current), upcoming.iter());

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
