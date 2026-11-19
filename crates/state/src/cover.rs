use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Context, Entity, Task};
use music::{CoverSize, MusicApi};

use crate::{Io, Playback, Session, SessionEvent, join};

/// The artwork of one album: the large cover, and the full-resolution one when the provider
/// serves something bigger.
#[derive(Clone)]
struct Art {
    large: String,
    full: Option<String>,
}

/// The large and full-resolution artwork of the shown track, for the surfaces that draw it
/// bigger than a list thumbnail. It follows the track's own art whenever the provider can size
/// it up, so tracks of one album that carry their own art each show it.
pub struct Cover {
    session: Entity<Session>,
    playback: Entity<Playback>,
    album: Option<String>,
    thumbnail: Option<String>,
    large: Option<String>,
    full: Option<String>,
    cache: HashMap<String, Art>,
    io: Io,
    task: Option<Task<()>>,
}

impl Cover {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe(&playback, |this, _, cx| this.follow(cx))
            .detach();
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedOut => this.forget(cx),
            SessionEvent::SignedIn
            | SessionEvent::Reconnected
            | SessionEvent::WentOffline
            | SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            session,
            playback,
            album: None,
            thumbnail: None,
            large: None,
            full: None,
            cache: HashMap::new(),
            io,
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

    fn forget(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.album = None;
        self.thumbnail = None;
        self.large = None;
        self.full = None;
        self.cache.clear();
        cx.notify();
    }

    /// Follows the shown track. Both covers come straight from the track's thumbnail when the
    /// provider can size it up, and the album's art is loaded only when it cannot. A load
    /// already running for the same album is kept rather than started again.
    fn follow(&mut self, cx: &mut Context<Self>) {
        let (album, thumbnail) = match self.playback.read(cx).shown_track(cx) {
            Some(track) => (track.album_id, track.cover),
            None => (None, None),
        };
        if album == self.album && thumbnail == self.thumbnail {
            return;
        }
        let client = album.as_deref().and_then(|id| self.client(id, cx));
        let derive = |size| client.as_ref()?.cover_at(thumbnail.as_deref()?, size);
        let (large, full) = (derive(CoverSize::Large), derive(CoverSize::Full));
        self.thumbnail = thumbnail;
        if album != self.album {
            self.task = None;
            self.album = album.clone();
        }

        let cached = album.as_ref().and_then(|id| self.cache.get(id)).cloned();
        self.large = large.or_else(|| cached.as_ref().map(|art| art.large.clone()));
        self.full = full.or_else(|| cached.and_then(|art| art.full));
        cx.notify();

        let Some(id) = album else {
            return;
        };
        if self.large.is_some() || self.task.is_some() {
            return;
        }
        self.load(id, cx);
    }

    fn load(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(client) = self.client(&id, cx) else {
            return;
        };

        let io = self.io.clone();
        let wanted = id.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let found = join(io.spawn(async move {
                let large = client.album(&wanted).await?.album.cover_large;
                let full = large
                    .as_deref()
                    .and_then(|cover| client.cover_at(cover, CoverSize::Full));
                anyhow::Ok(large.map(|large| Art { large, full }))
            }))
            .await;

            this.update(cx, |this, cx| {
                this.task = None;
                match found {
                    Ok(art) => {
                        let Some(art) = art else {
                            return;
                        };
                        this.cache.insert(id.clone(), art.clone());
                        if this.album.as_deref() == Some(id.as_str()) && this.large.is_none() {
                            this.large = Some(art.large);
                            let kept = this.full.take();
                            this.full = art.full.or(kept);
                            cx.notify();
                        }
                    }
                    Err(error) => {
                        log::warn!("cover: cannot load {id}: {error:#}");
                        crate::noted(&error, cx);
                    }
                }
            })
            .ok();
        }));
    }

    /// The client that serves album `id`: the local library for a local id, the signed-in
    /// provider otherwise.
    fn client(&self, id: &str, cx: &Context<Self>) -> Option<Arc<dyn MusicApi>> {
        let session = self.session.read(cx);
        match music::is_local_id(id) {
            true => session.local_client(),
            false => session.client(),
        }
    }
}
