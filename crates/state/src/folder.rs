use std::sync::Arc;

use gpui::{Context, Entity, Task};
use music::{MusicApi, Track};
use tokio::task::JoinSet;

use crate::library::Shelf;
use crate::session::{Session, SessionEvent};
use crate::{Io, Playback, join};

/// The shuffle of a folder page. A folder holds playlists rather than tracks, so playing one means
/// reading every playlist inside it first; that read is what this owns. The folder itself needs
/// nothing loaded — it is already in the library's outline.
pub struct FolderShuffle {
    session: Entity<Session>,
    playback: Entity<Playback>,
    io: Io,
    gathering: Option<Task<()>>,
}

impl FolderShuffle {
    pub fn new(
        session: Entity<Session>,
        playback: Entity<Playback>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| {
            if matches!(event, SessionEvent::SignedOut) {
                this.gathering = None;
                cx.notify();
            }
        })
        .detach();

        Self {
            session,
            playback,
            io,
            gathering: None,
        }
    }

    /// Whether a folder is still being read. The page's button waits on this.
    pub fn gathering(&self) -> bool {
        self.gathering.is_some()
    }

    /// Reads every playlist in `playlists` and starts them shuffled. Leaving the page drops the
    /// task, which cancels the reads. A playlist that fails is left out rather than failing the
    /// lot, so one unreachable playlist cannot silence a whole folder.
    pub fn play(&mut self, shelf: Shelf, playlists: Vec<String>, cx: &mut Context<Self>) {
        let Some(client) = self.session.read(cx).client_of(shelf) else {
            return;
        };
        if playlists.is_empty() || self.gathering.is_some() {
            return;
        }

        let io = self.io.clone();
        let playback = self.playback.clone();
        self.gathering = Some(cx.spawn(async move |this, cx| {
            let gathered = join(io.spawn(gather(client, playlists))).await;

            this.update(cx, |this, cx| {
                this.gathering = None;
                match gathered {
                    Ok(tracks) if tracks.is_empty() => {}
                    Ok(tracks) => playback.update(cx, |playback, cx| {
                        playback.shuffle_any(tracks, None, cx);
                    }),
                    Err(error) => log::warn!("library: cannot read a folder: {error:#}"),
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }
}

/// Every track of every playlist named, fetched at once. Order does not matter: what comes back
/// is shuffled.
async fn gather(client: Arc<dyn MusicApi>, playlists: Vec<String>) -> anyhow::Result<Vec<Track>> {
    let mut reads = JoinSet::new();
    for id in playlists {
        let client = client.clone();
        reads.spawn(async move { (id.clone(), client.playlist_tracks(&id).await) });
    }

    let mut tracks = Vec::new();
    while let Some(read) = reads.join_next().await {
        match read {
            Ok((_, Ok(listed))) => tracks.extend(listed),
            Ok((id, Err(error))) => {
                log::warn!("library: cannot read {id} in a folder: {error:#}")
            }
            Err(error) => log::warn!("library: a folder read did not finish: {error:#}"),
        }
    }

    Ok(tracks)
}
