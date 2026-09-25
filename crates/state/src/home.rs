use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Context, Entity, Task};
use music::{GenreItem, GenreSection, HomeFeed, MusicApi, Track};

use crate::playback::PlaybackEvent;
use crate::{
    Io, Library, LibraryPart, LibraryState, Network, Playback, Session, SessionEvent, Shelf, join,
};

const GROUP_SIZE: usize = 10;
const LIMIT: usize = GROUP_SIZE * 3;
/// How long to wait before asking for the feed again after nothing of it arrived, per try;
/// a provider that answers 503 for a moment is usually back by the second one.
const RETRIES: [Duration; 3] = [
    Duration::from_secs(3),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// How many rows Quick picks holds at most: the provider's own recent items first, then its
/// picks up to here.
const PICKS_LIMIT: usize = 30;

/// How long after a play starts to re-read the recently played shelf: the provider folds a
/// reported play in a little after the play began.
const RECENT_DELAY: Duration = Duration::from_secs(90);

pub struct Home {
    library: Entity<Library>,
    session: Entity<Session>,
    io: Io,
    /// What the provider listed as played lately, as it came.
    recent: Rc<Vec<GenreItem>>,
    /// The tracks that follow the recent ones: the provider's Quick picks, or a mix from the
    /// library when the provider has none.
    picks: Rc<Vec<Track>>,
    /// `recent` and then `picks`, up to `PICKS_LIMIT`, which is what the page draws as Quick
    /// picks.
    quick_picks: Rc<Vec<GenreItem>>,
    picks_seed: u64,
    sections: Rc<Vec<GenreSection>>,
    feeding: bool,
    /// Why the last fetch brought nothing, kept until a lot lands.
    error: Option<String>,
    /// How many fetches have ended with no feed at all since the last sign-in.
    failures: usize,
    /// Whether the home page is on screen. While it is, a lot may add to the page but never
    /// change what is already drawn, so nothing jumps under the user's eyes.
    visible: bool,
    /// The last lot that would have changed something on screen, kept whole for the moment
    /// the page is out of sight.
    pending: Option<HomeFeed>,
    task: Option<Task<()>>,
    naming: Option<Task<()>>,
    /// The re-read of the recently played shelf in flight, if one is.
    recent_task: Option<Task<()>>,
    /// The delayed re-read queued after a play started, replaced whenever another starts.
    play_timer: Option<Task<()>>,
}

impl Home {
    pub fn new(
        library: Entity<Library>,
        session: Entity<Session>,
        playback: Entity<Playback>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        let picks_seed = fastrand::u64(..);

        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => this.reload(cx),
            SessionEvent::SignedOut => {
                this.clear();
                cx.notify();
            }
            SessionEvent::LocalChanged => {
                if this.is_local(cx) {
                    this.reload(cx);
                }
            }
            SessionEvent::Reconnected => {}
        })
        .detach();

        cx.observe(&library, |this, _, cx| this.mix(cx)).detach();
        cx.subscribe(&playback, |this, _, event, cx| match event {
            PlaybackEvent::StartedPlayback => this.played(cx),
            PlaybackEvent::EndedPlayback | PlaybackEvent::Paused | PlaybackEvent::Seeked => {}
        })
        .detach();

        let mut home = Self {
            library,
            session,
            io,
            recent: Rc::new(Vec::new()),
            picks: Rc::new(Vec::new()),
            quick_picks: Rc::new(Vec::new()),
            picks_seed,
            sections: Rc::new(Vec::new()),
            feeding: false,
            error: None,
            failures: 0,
            visible: false,
            pending: None,
            task: None,
            naming: None,
            recent_task: None,
            play_timer: None,
        };
        home.feed(cx);
        home
    }

    pub fn sections(&self) -> Rc<Vec<GenreSection>> {
        self.sections.clone()
    }

    pub fn is_feeding(&self) -> bool {
        self.feeding
    }

    /// Why the page is empty, when the feed failed rather than came back empty. Nothing while
    /// a fetch is in flight or the page has anything to draw, Quick picks mixed from the library
    /// included, so the failure only shows where there is nothing else.
    pub fn error(&self) -> Option<&str> {
        match self.feeding || !self.sections.is_empty() || !self.quick_picks.is_empty() {
            true => None,
            false => self.error.as_deref(),
        }
    }

    /// Asks for the feed again after a failure, with the pauses between tries started over.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        self.failures = 0;
        self.task = None;
        self.feed(cx);
    }

    fn client(&self, cx: &App) -> Option<Arc<dyn MusicApi>> {
        let session = self.session.read(cx);
        if session.guest() && session.local_client().is_some() {
            session.local_client()
        } else {
            session.client()
        }
    }

    fn shelf(&self, cx: &App) -> Shelf {
        let session = self.session.read(cx);
        if session.guest() {
            Shelf::Local
        } else {
            Shelf::Streaming
        }
    }

    pub fn is_local(&self, cx: &App) -> bool {
        self.shelf(cx) == Shelf::Local
    }

    fn clear(&mut self) {
        self.task = None;
        self.naming = None;
        self.recent_task = None;
        self.play_timer = None;
        self.recent = Rc::new(Vec::new());
        self.picks = Rc::new(Vec::new());
        self.quick_picks = Rc::new(Vec::new());
        self.sections = Rc::new(Vec::new());
        self.feeding = false;
        self.error = None;
        self.failures = 0;
        self.pending = None;
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.clear();
        self.feed(cx);
        cx.notify();
    }

    /// Re-reads the provider's live recently played shelf into the home page's Recently Played
    /// section. A fetch in flight is never doubled, and a failed, empty or identical answer
    /// leaves the drawn shelf alone.
    pub fn refresh_recent(&mut self, cx: &mut Context<Self>) {
        if self.recent_task.is_some() {
            return;
        }
        let Some(client) = self.client(cx) else {
            return;
        };
        let io = self.io.clone();
        self.recent_task = Some(cx.spawn(async move |this, cx| {
            let live = join(io.spawn(async move { client.recent_resources().await })).await;
            this.update(cx, |this, cx| {
                this.recent_task = None;
                let live = match live {
                    Ok(live) if !live.is_empty() => live,
                    Ok(_) => return,
                    Err(error) => {
                        log::warn!("home: cannot load the recently played shelf: {error:#}");
                        return;
                    }
                };
                let mut sections = this.sections.as_ref().clone();
                match sections
                    .iter()
                    .position(|section| section.title.eq_ignore_ascii_case("recently played"))
                {
                    Some(at) if sections[at].items != live => sections[at].items = live,
                    Some(_) => return,
                    None => sections.insert(
                        1.min(sections.len()),
                        GenreSection {
                            title: "Recently Played".to_owned(),
                            items: live,
                        },
                    ),
                }
                this.sections = Rc::new(sections);
                cx.notify();
            })
            .ok();
        }));
    }

    /// A play just started: re-read the shelf at once, then once more after the provider has
    /// had time to fold the play in. One-shot timers off the play event, never a poll.
    fn played(&mut self, cx: &mut Context<Self>) {
        self.refresh_recent(cx);
        self.play_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RECENT_DELAY).await;
            this.update(cx, |this, cx| this.refresh_recent(cx)).ok();
        }));
    }

    pub fn feed(&mut self, cx: &mut Context<Self>) {
        if self.feeding || !self.sections.is_empty() {
            return;
        }
        let Some(client) = self.client(cx) else {
            return;
        };

        self.feeding = true;
        self.error = None;
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let opened = join(io.spawn(async move { client.home_paged().await })).await;
            let mut feed = match opened {
                Ok(feed) => feed,
                Err(error) => {
                    log::warn!("home: cannot load the feed: {error:#}");
                    this.update(cx, |this, cx| {
                        this.error = Some(crate::blamed(&error, cx));
                        this.fed(cx);
                    })
                    .ok();
                    return;
                }
            };

            // Every lot is the whole feed so far, so each one replaces the last on the page.
            while let Some(lot) = feed.recv().await {
                let landed = this.update(cx, |this, cx| {
                    match lot {
                        Ok(feed) => this.land(feed, cx),
                        Err(error) => {
                            log::warn!("home: cannot load the feed: {error:#}");
                            this.error = Some(crate::blamed(&error, cx));
                        }
                    }
                    cx.notify();
                });
                if landed.is_err() {
                    return;
                }
            }
            this.update(cx, |this, cx| this.fed(cx)).ok();
        }));
    }

    /// Puts a lot on the page. With the page in view only what is missing lands: a shelf
    /// already drawn keeps its rows even when the lot has other ones for it, Quick picks may
    /// only grow at their end, and the lot is kept whole to land once the page is out of
    /// sight. Out of sight, the lot lands as it is.
    fn land(&mut self, feed: HomeFeed, cx: &mut Context<Self>) {
        self.error = None;
        Network::reached(cx);
        if !self.visible {
            self.pending = None;
            self.take(feed, cx);
            return;
        }

        let mut held = false;
        let mut sections = pruned(&feed.sections);
        for section in &mut sections {
            let shown = self
                .sections
                .iter()
                .find(|shown| shown.title == section.title);
            if let Some(shown) = shown.filter(|shown| shown.items != section.items) {
                *section = shown.clone();
                held = true;
            }
        }
        let grown = feed.listen_again.starts_with(&self.recent);
        let listen_again = match grown {
            true => feed.listen_again.clone(),
            false => self.recent.as_ref().clone(),
        };
        let quick_picks = feed
            .quick_picks
            .clone()
            .filter(|picks| picks.starts_with(&self.picks));
        held |= !grown || (feed.quick_picks.is_some() && quick_picks.is_none());

        self.pending = held.then_some(feed.clone());
        self.take(
            HomeFeed {
                listen_again,
                quick_picks,
                sections,
            },
            cx,
        );
    }

    /// Puts a lot on the page as it is.
    fn take(&mut self, feed: HomeFeed, cx: &mut Context<Self>) {
        self.recent = Rc::new(feed.listen_again);
        if let Some(quick_picks) = feed.quick_picks {
            self.picks = Rc::new(quick_picks);
        }
        self.merge();
        self.sections = Rc::new(pruned(&feed.sections));
        self.name_playlists(feed.sections, cx);
    }

    /// Rebuilds what the page draws as Quick picks: the recent items, then the picks that are
    /// not among them already, up to `PICKS_LIMIT`.
    fn merge(&mut self) {
        let mut quick_picks = self.recent.as_ref().clone();
        let played: HashSet<&str> = self
            .recent
            .iter()
            .filter_map(|item| match item {
                GenreItem::Track(track) => track.id.as_deref(),
                _ => None,
            })
            .collect();
        let room = PICKS_LIMIT.saturating_sub(quick_picks.len());
        let picks: Vec<GenreItem> = self
            .picks
            .iter()
            .filter(|track| track.id.as_deref().is_none_or(|id| !played.contains(id)))
            .take(room)
            .cloned()
            .map(GenreItem::Track)
            .collect();
        quick_picks.extend(picks);
        quick_picks.truncate(PICKS_LIMIT);
        self.quick_picks = Rc::new(quick_picks);
    }

    /// Tells the feed whether the home page is on screen. Leaving it lands whatever was held
    /// back while it was, so the page comes back changed rather than changing in view.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.visible = visible;
        if visible {
            return;
        }
        if let Some(pending) = self.pending.take() {
            self.take(pending, cx);
            cx.notify();
        }
    }

    /// The end of a fetch. One that brought nothing is tried again after a `RETRIES` pause,
    /// since a provider's home is the kind of call that fails for a moment and then works.
    fn fed(&mut self, cx: &mut Context<Self>) {
        self.feeding = false;
        self.mix(cx);
        if self.sections.is_empty() && self.recent.is_empty() {
            if let Some(&after) = RETRIES.get(self.failures) {
                self.failures += 1;
                self.task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(after).await;
                    this.update(cx, |this, cx| this.feed(cx)).ok();
                }));
            }
        } else {
            self.failures = 0;
        }
        cx.notify();
    }

    fn name_playlists(&mut self, sections: Vec<GenreSection>, cx: &mut Context<Self>) {
        if !sections
            .iter()
            .any(|section| section.items.iter().any(blank))
        {
            return;
        }
        let Some(client) = self.client(cx) else {
            return;
        };

        let io = self.io.clone();
        self.naming = Some(cx.spawn(async move |this, cx| {
            let named = io
                .spawn(async move { client.name_home_playlists(sections).await })
                .await;
            let Ok(named) = named else {
                return;
            };

            this.update(cx, |this, cx| {
                this.sections = Rc::new(named);
                cx.notify();
            })
            .ok();
        }));
    }

    /// What the page draws as Quick picks: what the provider listed as played lately, mixed,
    /// tracks beside albums, playlists and artists in the provider's own order, and after
    /// them the provider's picks, up to `PICKS_LIMIT` rows in all.
    pub fn quick_picks(&self) -> Rc<Vec<GenreItem>> {
        self.quick_picks.clone()
    }

    /// Whether Quick picks are still on their way: the feed is in flight and nothing of it has
    /// landed yet, or the library the picks would otherwise be mixed from is.
    pub fn is_loading(&self, cx: &App) -> bool {
        let shelf = self.shelf(cx);
        (self.feeding && self.quick_picks.is_empty())
            || self.library.read(cx).loading(shelf, LibraryPart::Tracks)
    }

    /// Mixes picks from the library, but only in place of a provider's that never came: not
    /// while the feed is still in flight, and never over picks already there. A signed-out
    /// run has no feed, so it mixes as soon as the library is ready.
    fn mix(&mut self, cx: &mut Context<Self>) {
        if self.feeding || !self.picks.is_empty() {
            return;
        }
        let shelf = self.shelf(cx);
        let ready = matches!(self.library.read(cx).state(shelf), LibraryState::Ready(_));
        if !ready {
            return;
        }
        self.picks = picks(&self.library, shelf, self.picks_seed, cx);
        self.merge();
        cx.notify();
    }
}

fn blank(item: &GenreItem) -> bool {
    match item {
        GenreItem::Playlist(playlist) => playlist.name.is_empty(),
        _ => false,
    }
}

fn pruned(sections: &[GenreSection]) -> Vec<GenreSection> {
    sections
        .iter()
        .filter_map(|section| {
            let items: Vec<GenreItem> = section
                .items
                .iter()
                .filter(|item| !blank(item))
                .cloned()
                .collect();

            (!items.is_empty()).then(|| GenreSection {
                title: section.title.clone(),
                items,
            })
        })
        .collect()
}

fn picks(library: &Entity<Library>, shelf: Shelf, seed: u64, cx: &App) -> Rc<Vec<Track>> {
    let tracks = library.read(cx).state(shelf).tracks();
    Rc::new(mixed_tracks(tracks, seed))
}

fn mixed_tracks(tracks: &[Track], seed: u64) -> Vec<Track> {
    let mut random = fastrand::Rng::with_seed(seed);
    let mut selected = tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| track.playable && track.id.is_some())
        .map(|(index, _)| index)
        .take(GROUP_SIZE)
        .collect::<Vec<_>>();
    let recent = selected.iter().copied().collect::<HashSet<_>>();
    let mut remaining = tracks
        .iter()
        .enumerate()
        .filter(|(index, track)| track.playable && track.id.is_some() && !recent.contains(index))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    random.shuffle(&mut remaining);

    let random_count = GROUP_SIZE.min(remaining.len());
    selected.extend(remaining.drain(..random_count));

    let mut artists = selected
        .iter()
        .map(|index| artist_key(&tracks[*index]))
        .collect::<HashSet<_>>();
    let mut fallback = Vec::new();
    let mut diverse_count = 0;
    for index in remaining {
        if diverse_count < GROUP_SIZE && artists.insert(artist_key(&tracks[index])) {
            selected.push(index);
            diverse_count += 1;
        } else {
            fallback.push(index);
        }
    }

    selected.extend(fallback.into_iter().take(LIMIT - selected.len()));
    let mut mixed = selected
        .into_iter()
        .map(|index| tracks[index].clone())
        .collect::<Vec<_>>();
    random.shuffle(&mut mixed);
    mixed
}

fn artist_key(track: &Track) -> &str {
    track
        .artist_refs
        .first()
        .map(|artist| artist.id.as_deref().unwrap_or(&artist.name))
        .unwrap_or(&track.artists)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use music::{ArtistRef, Track};

    use super::{GROUP_SIZE, LIMIT, mixed_tracks};

    fn track(index: usize, artist: usize, playable: bool) -> Track {
        Track {
            id: Some(format!("track-{index}")),
            name: format!("Track {index}"),
            playable,
            artists: format!("Artist {artist}"),
            artist_refs: vec![ArtistRef {
                name: format!("Artist {artist}"),
                id: Some(format!("artist-{artist}")),
            }],
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

    #[test]
    fn mixed_selection_is_stable_and_has_no_duplicates() {
        let tracks = (0..60)
            .map(|index| track(index, index, true))
            .collect::<Vec<_>>();

        let first = mixed_tracks(&tracks, 42);
        let second = mixed_tracks(&tracks, 42);
        let ids = first
            .iter()
            .filter_map(|track| track.id.as_ref())
            .collect::<HashSet<_>>();

        assert_eq!(first, second);
        assert_eq!(first.len(), LIMIT);
        assert_eq!(ids.len(), LIMIT);
        for index in 0..GROUP_SIZE {
            let expected = format!("track-{index}");
            assert!(
                first
                    .iter()
                    .any(|track| track.id.as_deref() == Some(expected.as_str()))
            );
        }
    }

    #[test]
    fn mixed_selection_excludes_unavailable_tracks() {
        let tracks = (0..50)
            .map(|index| track(index, index, index % 2 == 0))
            .collect::<Vec<_>>();

        let selected = mixed_tracks(&tracks, 7);

        assert_eq!(selected.len(), 25);
        assert!(selected.iter().all(|track| track.playable));
    }

    #[test]
    fn mixed_selection_adds_artist_variety() {
        let tracks = (0..64)
            .map(|index| track(index, index.saturating_sub(23), true))
            .collect::<Vec<_>>();

        let selected = mixed_tracks(&tracks, 99);
        let artists = selected
            .iter()
            .map(|track| &track.artists)
            .collect::<HashSet<_>>();

        assert!(artists.len() > GROUP_SIZE);
    }
}
