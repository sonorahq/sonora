use gpui::prelude::*;
use gpui::{App, Context, Entity, FocusHandle, Global, Render, Window, div};
use i18n::t;
use log;
use music::{Album, SavedArtist, Track};
use state::{Detail, History, Io, Sonora};
use ui::{Button, Dismiss, FORM_CONTEXT, Modal, Submit};

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    LibrarySongs(usize),
    PlaylistSongs(usize),
    History(usize),
    Albums(usize),
    Artists(usize),
    Playlists(usize),
    DeleteTrackFiles(usize),
}

impl Kind {
    fn title(&self) -> gpui::SharedString {
        match self {
            Self::PlaylistSongs(_) => t!("confirm-remove-playlist-title"),
            Self::History(_) => t!("confirm-remove-history-title"),
            Self::DeleteTrackFiles(_) => t!("confirm-delete-track-files-title"),
            _ => t!("confirm-remove-library-title"),
        }
    }

    fn detail(&self) -> gpui::SharedString {
        match *self {
            Self::LibrarySongs(count) => t!("confirm-remove-songs", count = count),
            Self::PlaylistSongs(count) => t!("confirm-remove-playlist-songs", count = count),
            Self::History(count) => t!("confirm-remove-history-songs", count = count),
            Self::Albums(count) => t!("confirm-remove-albums", count = count),
            Self::Artists(count) => t!("confirm-remove-artists", count = count),
            Self::Playlists(count) => t!("confirm-remove-playlists", count = count),
            Self::DeleteTrackFiles(count) => t!("confirm-delete-track-files", count = count),
        }
    }

    fn action(&self) -> gpui::SharedString {
        t!("common-delete")
    }
}

type Apply = Box<dyn FnOnce(&mut App)>;

struct Pending {
    kind: Kind,
    apply: Apply,
}

pub(crate) struct Confirm {
    pending: Option<Pending>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    grab: bool,
}

struct Installed(Entity<Confirm>);

impl Global for Installed {}

impl Confirm {
    pub fn entity(cx: &mut App) -> Entity<Self> {
        if cx.try_global::<Installed>().is_none() {
            let confirm = cx.new(|cx| Self {
                pending: None,
                focus: cx.focus_handle(),
                restore: None,
                grab: false,
            });
            cx.set_global(Installed(confirm));
        }
        cx.global::<Installed>().0.clone()
    }

    pub fn ask(kind: Kind, apply: impl FnOnce(&mut App) + 'static, cx: &mut App) {
        let confirm = Self::entity(cx);
        confirm.update(cx, |this, cx| {
            this.pending = Some(Pending {
                kind,
                apply: Box::new(apply),
            });
            this.grab = true;
            cx.notify();
        });
    }

    pub fn library_songs(tracks: Vec<Track>, cx: &mut App) {
        if tracks.is_empty() {
            return;
        }
        Self::ask(
            Kind::LibrarySongs(tracks.len()),
            move |cx| {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| library.save_tracks(tracks, false, cx));
            },
            cx,
        );
    }

    pub fn playlist_songs(ids: Vec<String>, detail: Entity<Detail>, count: usize, cx: &mut App) {
        if ids.is_empty() {
            return;
        }
        Self::ask(
            Kind::PlaylistSongs(count),
            move |cx| {
                detail.update(cx, |detail, cx| detail.remove_tracks_from_playlist(ids, cx));
            },
            cx,
        );
    }

    pub fn history_songs(tracks: Vec<Track>, history: Entity<History>, cx: &mut App) {
        if tracks.is_empty() {
            return;
        }
        Self::ask(
            Kind::History(tracks.len()),
            move |cx| {
                history.update(cx, |history, cx| {
                    for track in &tracks {
                        history.remove(track, cx);
                    }
                });
            },
            cx,
        );
    }

    pub fn albums(albums: Vec<Album>, cx: &mut App) {
        if albums.is_empty() {
            return;
        }
        Self::ask(
            Kind::Albums(albums.len()),
            move |cx| {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| {
                    for album in albums {
                        library.toggle_album(album, cx);
                    }
                });
            },
            cx,
        );
    }

    pub fn artists(artists: Vec<SavedArtist>, cx: &mut App) {
        if artists.is_empty() {
            return;
        }
        Self::ask(
            Kind::Artists(artists.len()),
            move |cx| {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| {
                    for artist in artists {
                        library.toggle_artist(artist, cx);
                    }
                });
            },
            cx,
        );
    }

    pub fn playlists(ids: Vec<String>, cx: &mut App) {
        if ids.is_empty() {
            return;
        }
        Self::ask(
            Kind::Playlists(ids.len()),
            move |cx| {
                let library = Sonora::global(cx).library.clone();
                library.update(cx, |library, cx| {
                    for id in ids {
                        library.remove_playlist_from_library(id, cx);
                    }
                });
            },
            cx,
        );
    }

    pub fn delete_track_files(ids: Vec<String>, cx: &mut App) {
        if ids.is_empty() {
            return;
        }
        Self::ask(
            Kind::DeleteTrackFiles(ids.len()),
            move |cx| {
                let sonora = Sonora::global(cx);
                let Some(provider) = sonora.session.read(cx).local_client() else {
                    return;
                };
                let library = sonora.library.clone();
                let io = Io::global(cx);
                cx.spawn(async move |cx| {
                    let deleted = io
                        .spawn(async move {
                            for id in ids {
                                provider
                                    .delete_track_file(&id)
                                    .await
                                    .map_err(|error| error.to_string())?;
                            }
                            Result::<(), String>::Ok(())
                        })
                        .await;

                    match deleted {
                        Ok(Ok(())) => {
                            library.update(cx, |library, cx| {
                                library.rescan_local(cx);
                            });
                        }
                        Ok(Err(error)) => {
                            log::warn!("local: cannot delete track files: {error:#}");
                        }
                        Err(error) => {
                            log::warn!("local: deletion task failed: {error}");
                        }
                    }
                })
                .detach();
            },
            cx,
        );
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = None;
        self.grab = false;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        cx.notify();
    }

    fn apply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.grab = false;
        if let Some(focus) = self.restore.take() {
            window.focus(&focus, cx);
        }
        (pending.apply)(cx);
        cx.notify();
    }
}

impl Render for Confirm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(pending) = self.pending.as_ref() else {
            return div().into_any_element();
        };
        if self.grab {
            self.restore = window.focused(cx);
            window.focus(&self.focus, cx);
            self.grab = false;
        }

        let title = pending.kind.title();
        let detail = pending.kind.detail();
        let action = pending.kind.action();

        div()
            .absolute()
            .inset_0()
            .key_context(FORM_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| {
                cx.stop_propagation();
                this.close(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Submit, window, cx| {
                cx.stop_propagation();
                this.apply(window, cx);
            }))
            .child(
                Modal::new("confirm-remove", title)
                    .detail(detail)
                    .action(
                        Button::new("cancel-confirm")
                            .ghost()
                            .label(t!("common-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    )
                    .action(
                        Button::new("apply-confirm")
                            .danger()
                            .label(action)
                            .on_click(cx.listener(|this, _, window, cx| this.apply(window, cx))),
                    )
                    .on_dismiss(cx.listener(|this, _, window, cx| this.close(window, cx))),
            )
            .into_any_element()
    }
}
