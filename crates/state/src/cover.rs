use std::collections::HashMap;
use std::sync::Arc;

use music::{CoverSize, MusicApi, Track};

use crate::{Change, Cx, Event, Job, Session, SessionEvent, Store, Topic};

/// The artwork of one album: the large cover, and the full-resolution one when the provider
/// serves something bigger.
#[derive(Clone)]
struct Art {
    large: String,
    full: Option<String>,
}

/// The artwork of the album the shown track is from, which the player, the fullscreen view and
/// the presence integrations draw. A change raises [`Topic::Cover`].
pub struct Cover {
    album: Option<String>,
    large: Option<String>,
    full: Option<String>,
    cache: HashMap<String, Art>,
    task: Option<Job>,
}

impl Cover {
    pub fn new() -> Self {
        Self {
            album: None,
            large: None,
            full: None,
            cache: HashMap::new(),
            task: None,
        }
    }

    pub fn large(&self) -> Option<&str> {
        self.large.as_deref()
    }

    /// Returns the large artwork only when it belongs to `album`.
    pub fn large_for(&self, album: &str) -> Option<&str> {
        self.large
            .as_deref()
            .filter(|_| self.album.as_deref() == Some(album))
    }

    /// The full-resolution artwork for a cover that fills the screen, only when it belongs to
    /// `album` and the provider serves something bigger than the large one.
    pub fn full_for(&self, album: &str) -> Option<&str> {
        self.full
            .as_deref()
            .filter(|_| self.album.as_deref() == Some(album))
    }

    fn forget(&mut self, cx: &mut Cx) {
        self.task = None;
        self.album = None;
        self.large = None;
        self.full = None;
        self.cache.clear();
        cx.changed(Topic::Cover);
    }

    /// Follows the shown track's album. Both covers come straight from the track's thumbnail
    /// when the provider can tell, and the album is loaded only when it cannot.
    fn follow(&mut self, shown: Option<Track>, session: &Session, cx: &mut Cx) {
        let (album, thumbnail) = match shown {
            Some(track) => (track.album_id, track.cover),
            None => (None, None),
        };
        if album == self.album {
            return;
        }
        self.task = None;
        self.album = album.clone();
        let cached = album.as_ref().and_then(|id| self.cache.get(id)).cloned();
        let client = album.as_deref().and_then(|id| client(session, id));
        let derive = |size| client.as_ref()?.cover_at(thumbnail.as_deref()?, size);
        self.large = cached
            .as_ref()
            .map(|art| art.large.clone())
            .or_else(|| derive(CoverSize::Large));
        self.full = cached
            .and_then(|art| art.full)
            .or_else(|| derive(CoverSize::Full));
        cx.changed(Topic::Cover);

        let Some(id) = album else {
            return;
        };
        if self.large.is_some() {
            return;
        }
        self.load(id, session, cx);
    }

    fn load(&mut self, id: String, session: &Session, cx: &mut Cx) {
        let Some(client) = client(session, &id) else {
            return;
        };

        let wanted = id.clone();
        self.task = Some(cx.spawn(|link| async move {
            let found = async {
                let large = client.album(&wanted).await?.album.cover_large;
                let full = large
                    .as_deref()
                    .and_then(|cover| client.cover_at(cover, CoverSize::Full));
                anyhow::Ok(large.map(|large| Art { large, full }))
            }
            .await;

            link.post(move |store| {
                let Store {
                    cover: this, cx, ..
                } = store;
                this.task = None;
                match found {
                    Ok(art) => {
                        let Some(art) = art else {
                            return;
                        };
                        this.cache.insert(id.clone(), art.clone());
                        if this.album.as_deref() == Some(id.as_str()) {
                            this.large = Some(art.large);
                            this.full = art.full.or(this.full.take());
                            cx.changed(Topic::Cover);
                        }
                    }
                    Err(error) => {
                        log::warn!("cover: cannot load {id}: {error:#}");
                        crate::noted(&error, cx);
                    }
                }
            });
        }));
    }
}

impl Default for Cover {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// Follows the shown track's album as playback changes, and lets every cover go when the
    /// account signs out.
    pub(crate) fn react_cover(&mut self, change: &Change) {
        match change {
            Change::Topic(Topic::Playback) => {
                let shown = self.playback.shown_track(&self.devices);
                self.cover.follow(shown, &self.session, &mut self.cx);
            }
            Change::Event(Event::Session(SessionEvent::SignedOut)) => {
                self.cover.forget(&mut self.cx)
            }
            _ => {}
        }
    }
}

/// The client that serves album `id`: the local library for a local id, the signed-in provider
/// otherwise.
fn client(session: &Session, id: &str) -> Option<Arc<dyn MusicApi>> {
    match music::is_local_id(id) {
        true => session.local_client(),
        false => session.client(),
    }
}
