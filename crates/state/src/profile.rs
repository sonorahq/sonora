use std::collections::HashMap;
use std::sync::Arc;

use music::{Playlist, UserDetail};

use crate::{Change, Cx, Event, Job, Session, SessionEvent, Store, mosaic};

/// A user's profile page: what [`Store::pages`](crate::Pages) shows as `user`.
pub struct Profile {
    id: Option<String>,
    user: Option<Arc<UserDetail>>,
    loading: bool,
    error: Option<String>,
    /// Fetches the mosaics' covers.
    http: reqwest::Client,
    task: Option<Job>,
    mosaics: HashMap<String, Job>,
}

impl Profile {
    pub(crate) fn new() -> Self {
        Self {
            id: None,
            user: None,
            loading: false,
            error: None,
            http: reqwest::Client::new(),
            task: None,
            mosaics: HashMap::new(),
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn user(&self) -> Option<&UserDetail> {
        self.user.as_deref()
    }

    pub fn playlists(&self) -> &[Playlist] {
        self.user
            .as_ref()
            .map(|user| user.playlists.as_slice())
            .unwrap_or_default()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn open(&mut self, id: &str, session: &Session, cx: &mut Cx) {
        if self.id.as_deref() == Some(id) && (self.loading || self.user.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());

        let Some(client) = session.client() else {
            return;
        };

        self.loading = true;

        let id = id.to_owned();
        self.task = Some(cx.spawn(move |link| async move {
            let loaded = client.user(&id).await;

            link.post(move |store| {
                let Store {
                    pages, session, cx, ..
                } = store;
                let Some(this) = showing(&mut pages.user, &id) else {
                    return;
                };

                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(user) => {
                        this.user = Some(Arc::new(user));
                        this.build_mosaics(session, cx);
                    }
                    Err(reason) => this.error = Some(reason),
                }
            });
        }));
    }

    /// Drops the profile and fetches it again, so the page shows its loading state until the
    /// new one lands.
    fn refresh(&mut self, session: &Session, cx: &mut Cx) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.open(&id, session, cx);
    }

    fn adopt_mosaics(&mut self) -> Vec<(String, u32)> {
        let Some(user) = self.user.as_mut() else {
            return Vec::new();
        };

        let mut wanted = Vec::new();
        for playlist in Arc::make_mut(user).playlists.iter_mut() {
            if playlist.cover.is_some() {
                continue;
            }
            match mosaic::cached(&playlist.id, playlist.track_count) {
                Some(cover) => playlist.cover = Some(cover),
                None => wanted.push((playlist.id.clone(), playlist.track_count)),
            }
        }

        wanted
    }

    fn build_mosaics(&mut self, session: &Session, cx: &mut Cx) {
        let wanted = self.adopt_mosaics();
        let Some(client) = session.client() else {
            return;
        };
        let Some(user) = self.id.clone() else {
            return;
        };

        for (id, stamp) in wanted {
            if self.mosaics.contains_key(&id) {
                continue;
            }

            let client = client.clone();
            let key = id.clone();
            let user = user.clone();
            let task = cx.spawn(move |link| async move {
                let covers = client.playlist_covers(&id, mosaic::TILES).await;
                match covers {
                    Ok(covers) => link.post(move |store| {
                        let Store { pages, cx, .. } = store;
                        if let Some(this) = showing(&mut pages.user, &user) {
                            this.paint_mosaic(id, stamp, covers, cx);
                        }
                    }),
                    Err(error) => log::warn!("profile: cannot read playlist covers: {error:#}"),
                }
            });
            self.mosaics.insert(key, task);
        }
    }

    fn paint_mosaic(&mut self, id: String, stamp: u32, covers: Vec<String>, cx: &mut Cx) {
        if covers.len() < mosaic::TILES {
            self.mosaics.remove(&id);
            return;
        }
        let Some(user) = self.id.clone() else {
            return;
        };

        let http = self.http.clone();
        let key = id.clone();
        let task = cx.spawn(move |link| async move {
            let built = mosaic::build(http, &id, stamp, covers).await;

            link.post(move |store| {
                let Some(this) = showing(&mut store.pages.user, &user) else {
                    return;
                };
                this.mosaics.remove(&id);
                match built {
                    Ok(cover) => this.set_playlist_cover(&id, cover),
                    Err(error) => log::warn!("profile: cannot build a mosaic: {error:#}"),
                }
            });
        });
        self.mosaics.insert(key, task);
    }

    fn set_playlist_cover(&mut self, id: &str, cover: String) {
        let Some(user) = self.user.as_mut() else {
            return;
        };
        if let Some(playlist) = Arc::make_mut(user)
            .playlists
            .iter_mut()
            .find(|playlist| playlist.id == id)
        {
            playlist.cover = Some(cover);
        }
    }

    fn clear(&mut self) {
        self.task = None;
        self.mosaics.clear();
        self.id = None;
        self.user = None;
        self.loading = false;
        self.error = None;
    }
}

impl Store {
    /// Opens the profile page of the user `id`.
    pub fn open_user(&mut self, id: &str) {
        let Store {
            pages, session, cx, ..
        } = self;
        pages
            .user
            .get_or_insert_with(Profile::new)
            .open(id, session, cx);
    }

    /// Drops the profile page, and the loads it has under way.
    pub fn close_user(&mut self) {
        self.pages.user = None;
    }

    /// Drops the profile and fetches it again, so the page shows its loading state until the
    /// new one lands.
    pub fn refresh_user(&mut self) {
        let Store {
            pages, session, cx, ..
        } = self;
        if let Some(user) = pages.user.as_mut() {
            user.refresh(session, cx);
        }
    }

    pub(crate) fn react_profile(&mut self, change: &Change) {
        let Change::Event(Event::Session(event)) = change else {
            return;
        };
        let Store {
            pages, session, cx, ..
        } = self;
        let Some(this) = pages.user.as_mut() else {
            return;
        };
        match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone() {
                    this.clear();
                    this.open(&id, session, cx);
                }
            }
            SessionEvent::SignedOut => this.clear(),
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        }
    }
}

/// The page in `slot` when it still shows the user `id`, which a load that lands checks first,
/// since another profile may have opened in its place.
fn showing<'a>(slot: &'a mut Option<Profile>, id: &str) -> Option<&'a mut Profile> {
    slot.as_mut()
        .filter(|profile| profile.id.as_deref() == Some(id))
}
