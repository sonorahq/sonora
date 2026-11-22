use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Context, Entity, Task};
use music::{Playlist, UserDetail};
use tokio::task::AbortHandle;

use crate::{Io, Session, SessionEvent, join, mosaic};

pub struct Profile {
    id: Option<String>,
    user: Option<Arc<UserDetail>>,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
    request: Option<AbortHandle>,
    mosaics: mosaic::Tasks,
}

impl Profile {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone() {
                    this.clear();
                    this.open(&id, cx);
                }
            }
            SessionEvent::SignedOut => {
                this.clear();
                cx.notify();
            }
            SessionEvent::Reconnected | SessionEvent::WentOffline | SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            id: None,
            user: None,
            loading: false,
            error: None,
            session,
            io,
            task: None,
            request: None,
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

    pub fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.id.as_deref() == Some(id) && (self.loading || self.user.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());

        let Some(client) = self.session.read(cx).client() else {
            cx.notify();
            return;
        };

        self.loading = true;
        cx.notify();

        let id = id.to_owned();
        let request = self.io.spawn({
            let id = id.clone();
            async move { client.user(&id).await }
        });
        self.request = Some(request.abort_handle());
        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;

            this.update(cx, |this, cx| {
                if this.id.as_deref() != Some(id.as_str()) {
                    return;
                }

                this.loading = false;
                this.request = None;
                match crate::settled(loaded, cx) {
                    Ok(user) => {
                        this.user = Some(Arc::new(user));
                        this.build_mosaics(cx);
                    }
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Drops the profile and fetches it again, so the page shows its loading state until the
    /// new one lands.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id.clone() else {
            return;
        };
        self.clear();
        self.open(&id, cx);
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

    fn build_mosaics(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<_> = self
            .adopt_mosaics()
            .into_iter()
            .filter(|(id, _)| !self.mosaics.contains_key(id))
            .collect();
        let Some(client) = self.session.read(cx).client() else {
            return;
        };
        self.mosaics.extend(mosaic::fetch_covers(
            &self.io,
            client,
            wanted,
            None,
            |this| &mut this.mosaics,
            Self::paint_mosaic,
            cx,
        ));
    }

    fn paint_mosaic(
        &mut self,
        id: String,
        stamp: u32,
        covers: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if covers.len() < mosaic::TILES {
            self.mosaics.remove(&id);
            return;
        }

        let task = mosaic::paint(
            &self.io,
            id.clone(),
            stamp,
            covers,
            |this| &mut this.mosaics,
            Self::set_playlist_cover,
            cx,
        );
        self.mosaics.insert(id, task);
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
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.id = None;
        self.user = None;
        self.loading = false;
        self.error = None;
    }
}
