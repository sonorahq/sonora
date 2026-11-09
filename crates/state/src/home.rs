use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use music::{GenreItem, GenreSection, HomeFeed, MusicApi, Track};

use crate::{
    Change, Cx, Event, Job, Library, LibraryPart, LibraryState, Session, SessionEvent, Shelf,
    Store, Topic,
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

/// How many of the provider's recent items lead Quick picks at most, whatever the provider
/// lists.
const RECENT_LIMIT: usize = 10;

/// The home page's feed: the provider's shelves and Quick picks, mixed from the library when the
/// provider has none. It lives as long as the app and fetches as soon as it is made.
pub struct Home {
    /// What the provider listed as played lately, as it came.
    recent: Arc<Vec<GenreItem>>,
    /// The tracks that follow the recent ones: the provider's Quick picks, or a mix from the
    /// library when the provider has none.
    picks: Arc<Vec<Track>>,
    /// `recent` and then `picks`, up to `PICKS_LIMIT`, which is what the page draws as Quick
    /// picks.
    quick_picks: Arc<Vec<GenreItem>>,
    picks_seed: u64,
    sections: Arc<Vec<GenreSection>>,
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
    task: Option<Job>,
    naming: Option<Job>,
}

impl Home {
    pub fn new(session: &Session, cx: &mut Cx) -> Self {
        let picks_seed = fastrand::u64(..);

        let mut home = Self {
            recent: Arc::new(Vec::new()),
            picks: Arc::new(Vec::new()),
            quick_picks: Arc::new(Vec::new()),
            picks_seed,
            sections: Arc::new(Vec::new()),
            feeding: false,
            error: None,
            failures: 0,
            visible: false,
            pending: None,
            task: None,
            naming: None,
        };
        home.feed(session, cx);
        home
    }

    pub fn sections(&self) -> Arc<Vec<GenreSection>> {
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
    pub fn retry(&mut self, session: &Session, cx: &mut Cx) {
        self.failures = 0;
        self.task = None;
        self.feed(session, cx);
    }

    fn client(&self, session: &Session) -> Option<Arc<dyn MusicApi>> {
        if session.guest() && session.local_client().is_some() {
            session.local_client()
        } else {
            session.client()
        }
    }

    fn shelf(&self, session: &Session) -> Shelf {
        if session.guest() {
            Shelf::Local
        } else {
            Shelf::Streaming
        }
    }

    pub fn is_local(&self, session: &Session) -> bool {
        self.shelf(session) == Shelf::Local
    }

    fn clear(&mut self) {
        self.task = None;
        self.naming = None;
        self.recent = Arc::new(Vec::new());
        self.picks = Arc::new(Vec::new());
        self.quick_picks = Arc::new(Vec::new());
        self.sections = Arc::new(Vec::new());
        self.feeding = false;
        self.error = None;
        self.failures = 0;
        self.pending = None;
    }

    /// Drops the feed and asks the provider for it again, showing the loading shelves meanwhile.
    pub fn reload(&mut self, session: &Session, cx: &mut Cx) {
        self.clear();
        self.feed(session, cx);
    }

    pub fn feed(&mut self, session: &Session, cx: &mut Cx) {
        if self.feeding || !self.sections.is_empty() {
            return;
        }
        let Some(client) = self.client(session) else {
            return;
        };

        self.feeding = true;
        self.error = None;
        self.task = Some(cx.spawn(move |link| async move {
            let opened = client.home_paged().await;
            let mut feed = match opened {
                Ok(feed) => feed,
                Err(error) => {
                    log::warn!("home: cannot load the feed: {error:#}");
                    link.post(move |store| {
                        let Store {
                            home,
                            session,
                            library,
                            cx,
                            ..
                        } = store;
                        home.error = Some(crate::blamed(&error, cx));
                        home.fed(session, library, cx);
                    });
                    return;
                }
            };

            // Every lot is the whole feed so far, so each one replaces the last on the page.
            while let Some(lot) = feed.recv().await {
                let landed = link
                    .read(move |store| {
                        let Store {
                            home, session, cx, ..
                        } = store;
                        match lot {
                            Ok(feed) => home.land(feed, session, cx),
                            Err(error) => {
                                log::warn!("home: cannot load the feed: {error:#}");
                                home.error = Some(crate::blamed(&error, cx));
                            }
                        }
                    })
                    .await;
                if landed.is_none() {
                    return;
                }
            }
            link.post(|store| {
                let Store {
                    home,
                    session,
                    library,
                    cx,
                    ..
                } = store;
                home.fed(session, library, cx);
            });
        }));
    }

    /// Puts a lot on the page. With the page in view only what is missing lands: a shelf
    /// already drawn keeps its rows even when the lot has other ones for it, Quick picks may
    /// only grow at their end, and the lot is kept whole to land once the page is out of
    /// sight. Out of sight, the lot lands as it is.
    fn land(&mut self, feed: HomeFeed, session: &Session, cx: &mut Cx) {
        self.error = None;
        cx.emit(Event::NetworkReached);
        if !self.visible {
            self.pending = None;
            self.take(feed, session, cx);
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
            session,
            cx,
        );
    }

    /// Puts a lot on the page as it is.
    fn take(&mut self, feed: HomeFeed, session: &Session, cx: &mut Cx) {
        let mut recent = feed.listen_again;
        recent.truncate(RECENT_LIMIT);
        self.recent = Arc::new(recent);
        if let Some(quick_picks) = feed.quick_picks {
            self.picks = Arc::new(quick_picks);
        }
        self.merge();
        self.sections = Arc::new(pruned(&feed.sections));
        self.name_playlists(feed.sections, session, cx);
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
        self.quick_picks = Arc::new(quick_picks);
    }

    /// Tells the feed whether the home page is on screen. Leaving it lands whatever was held
    /// back while it was, so the page comes back changed rather than changing in view.
    pub fn set_visible(&mut self, visible: bool, session: &Session, cx: &mut Cx) {
        self.visible = visible;
        if visible {
            return;
        }
        if let Some(pending) = self.pending.take() {
            self.take(pending, session, cx);
        }
    }

    /// The end of a fetch. One that brought nothing is tried again after a `RETRIES` pause,
    /// since a provider's home is the kind of call that fails for a moment and then works.
    fn fed(&mut self, session: &Session, library: &Library, cx: &mut Cx) {
        self.feeding = false;
        self.mix(session, library);
        if self.sections.is_empty() && self.recent.is_empty() {
            if let Some(&after) = RETRIES.get(self.failures) {
                self.failures += 1;
                self.task = Some(cx.after(after, |store| {
                    let Store {
                        home, session, cx, ..
                    } = store;
                    home.feed(session, cx);
                }));
            }
        } else {
            self.failures = 0;
        }
    }

    fn name_playlists(&mut self, sections: Vec<GenreSection>, session: &Session, cx: &mut Cx) {
        if !sections
            .iter()
            .any(|section| section.items.iter().any(blank))
        {
            return;
        }
        let Some(client) = self.client(session) else {
            return;
        };

        self.naming = Some(cx.spawn(move |link| async move {
            let named = client.name_home_playlists(sections).await;

            link.post(move |store| {
                store.home.sections = Arc::new(named);
            });
        }));
    }

    /// What the page draws as Quick picks: what the provider listed as played lately, mixed,
    /// tracks beside albums, playlists and artists in the provider's own order, and after
    /// them the provider's picks, up to `PICKS_LIMIT` rows in all.
    pub fn quick_picks(&self) -> Arc<Vec<GenreItem>> {
        self.quick_picks.clone()
    }

    /// Whether Quick picks are still on their way: the account is still being restored, the
    /// feed is in flight and nothing of it has landed yet, or the library the picks would
    /// otherwise be mixed from is.
    pub fn is_loading(&self, session: &Session, library: &Library) -> bool {
        let shelf = self.shelf(session);
        session.is_pending()
            || (self.feeding && self.quick_picks.is_empty())
            || library.loading(shelf, LibraryPart::Tracks)
    }

    /// Mixes picks from the library, but only in place of a provider's that never came: not
    /// while the account is still being restored or the feed is in flight, and never over picks
    /// already there. A signed-out run has no feed, so it mixes as soon as the library is ready.
    fn mix(&mut self, session: &Session, library: &Library) {
        if self.feeding || !self.picks.is_empty() || session.is_pending() {
            return;
        }
        let shelf = self.shelf(session);
        let ready = matches!(library.state(shelf), LibraryState::Ready(_));
        if !ready {
            return;
        }
        self.picks = picks(library, shelf, self.picks_seed);
        self.merge();
    }
}

impl Store {
    /// Asks for the home feed again after a failure, with the pauses between tries started over.
    pub fn retry_home(&mut self) {
        self.home.retry(&self.session, &mut self.cx);
    }

    /// Drops the home feed and asks the provider for it again, showing the loading shelves
    /// meanwhile.
    pub fn reload_home(&mut self) {
        self.home.reload(&self.session, &mut self.cx);
    }

    /// Fetches the home feed again from the provider and refreshes the library shelf it mixes
    /// Quick picks from. The page shows its loading shelves until the new feed lands.
    pub fn refresh_home(&mut self) {
        let shelf = self.home.shelf(&self.session);
        self.library.refresh(shelf, &self.session, &mut self.cx);
        self.home.reload(&self.session, &mut self.cx);
    }

    /// Tells the home feed whether its page is on screen. Leaving it lands whatever was held
    /// back while it was.
    pub fn set_home_visible(&mut self, visible: bool) {
        self.home.set_visible(visible, &self.session, &mut self.cx);
    }

    pub(crate) fn react_home(&mut self, change: &Change) {
        let Store {
            home,
            session,
            library,
            cx,
            ..
        } = self;
        match change {
            Change::Event(Event::Session(event)) => match event {
                SessionEvent::SignedIn => home.reload(session, cx),
                SessionEvent::SignedOut => home.clear(),
                SessionEvent::LocalChanged => {
                    if home.is_local(session) {
                        home.reload(session, cx);
                    }
                }
                SessionEvent::Reconnected | SessionEvent::WentOffline => {}
            },
            Change::Topic(Topic::Library) => home.mix(session, library),
            // A restore that ends offline sends no event of its own, and Quick picks wait for it.
            Change::Topic(Topic::Session) => home.mix(session, library),
            _ => {}
        }
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

fn picks(library: &Library, shelf: Shelf, seed: u64) -> Arc<Vec<Track>> {
    let tracks = library.state(shelf).tracks();
    Arc::new(mixed_tracks(tracks, seed))
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
            duration: Duration::from_secs(180),
            ..Default::default()
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
