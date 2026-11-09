//! The one question asked before something is taken away for good: songs, albums, artists or
//! playlists out of the library, songs out of a playlist or the history, track files off the
//! disk, the Widevine module, every download, or an equalizer profile.

use i18n::t;
use music::{Album, SavedArtist, Track};
use state::{Library, Outcome, Shelf, Toasts, equalizer_profile_label};
use ui::{Button, Modal};

use crate::Ui;

/// What is being asked about, and how many of it.
#[derive(Clone)]
pub(crate) enum Kind {
    LibrarySongs(usize),
    PlaylistSongs(usize),
    History(usize),
    Albums(usize),
    Artists(usize),
    Playlists(usize),
    DeleteTrackFiles(usize),
    Widevine,
    /// Every download, of every provider.
    Downloads,
    /// An equalizer profile, by its stored name.
    EqualizerProfile(String),
}

impl Kind {
    fn title(&self) -> String {
        match self {
            Self::PlaylistSongs(_) => t!("confirm-remove-playlist-title"),
            Self::History(_) => t!("confirm-remove-history-title"),
            Self::DeleteTrackFiles(_) => t!("confirm-delete-track-files-title"),
            Self::Widevine => t!("confirm-uninstall-widevine-title"),
            Self::Downloads => t!("confirm-remove-downloads-title"),
            Self::EqualizerProfile(_) => t!("equalizer-delete"),
            _ => t!("confirm-remove-library-title"),
        }
        .into_owned()
    }

    fn detail(&self) -> String {
        match *self {
            Self::LibrarySongs(count) => t!("confirm-remove-songs", count = count),
            Self::PlaylistSongs(count) => t!("confirm-remove-playlist-songs", count = count),
            Self::History(count) => t!("confirm-remove-history-songs", count = count),
            Self::Albums(count) => t!("confirm-remove-albums", count = count),
            Self::Artists(count) => t!("confirm-remove-artists", count = count),
            Self::Playlists(count) => t!("confirm-remove-playlists", count = count),
            Self::DeleteTrackFiles(count) => t!("confirm-delete-track-files", count = count),
            Self::Widevine => t!("confirm-uninstall-widevine"),
            Self::Downloads => t!("confirm-remove-downloads"),
            Self::EqualizerProfile(ref name) => {
                let name = equalizer_profile_label(name);
                t!("equalizer-delete-confirm", name = name.as_ref())
            }
        }
        .into_owned()
    }

    fn action(&self) -> String {
        match self {
            Self::Widevine => t!("settings-widevine-uninstall"),
            Self::Downloads => t!("settings-downloads-remove"),
            _ => t!("common-delete"),
        }
        .into_owned()
    }
}

/// What confirming does.
type Apply = Box<dyn FnOnce(&mut Ui)>;

/// The question on screen and what answering yes does.
pub(crate) struct Pending {
    kind: Kind,
    apply: Apply,
}

/// Whether taking the heart off `id` only unstars it, so no question is worth asking. On a
/// `Shape::Catalog` shelf the favorites sit over a library that stays put; on a `Shape::Saved`
/// one the heart is the library itself, and the question stands.
pub(crate) fn unstarring(library: &Library, id: &str) -> bool {
    library.shape(Shelf::of(id)).is_catalog()
}

/// Puts the question `kind` up, running `apply` once it is confirmed.
pub(crate) fn ask(ui: &mut Ui, kind: Kind, apply: impl FnOnce(&mut Ui) + 'static) {
    ui.dialogs.confirm = Some(Pending {
        kind,
        apply: Box::new(apply),
    });
}

/// Takes songs off the favorites, asking first where the heart is the library itself.
pub(crate) fn library_songs(ui: &mut Ui, tracks: Vec<Track>) {
    if tracks.is_empty() {
        return;
    }
    let count = tracks.len();
    let starred = tracks
        .first()
        .and_then(|track| track.id.as_deref())
        .is_some_and(|id| unstarring(&ui.store.library, id));
    let apply = move |ui: &mut Ui| {
        let store = &mut ui.store;
        store
            .library
            .save_tracks(tracks, false, &store.session, &mut store.cx);
    };
    match starred {
        true => apply(ui),
        false => ask(ui, Kind::LibrarySongs(count), apply),
    }
}

/// Takes songs out of the playlist on the playlist page, by track id. `count` is how many rows
/// were picked, which is what the question counts.
pub(crate) fn playlist_songs(ui: &mut Ui, ids: Vec<String>, count: usize) {
    if ids.is_empty() {
        return;
    }
    ask(ui, Kind::PlaylistSongs(count), move |ui: &mut Ui| {
        ui.store.remove_tracks_from_playlist_page(ids)
    });
}

/// Forgets plays of these songs from the history.
pub(crate) fn history_songs(ui: &mut Ui, tracks: Vec<Track>) {
    if tracks.is_empty() {
        return;
    }
    ask(ui, Kind::History(tracks.len()), move |ui: &mut Ui| {
        let store = &mut ui.store;
        for track in &tracks {
            store.history.remove(track, &store.session, &mut store.cx);
        }
    });
}

/// Takes albums off the favorites, asking first where the heart is the library itself.
pub(crate) fn albums(ui: &mut Ui, albums: Vec<Album>) {
    if albums.is_empty() {
        return;
    }
    let count = albums.len();
    let starred = albums
        .first()
        .is_some_and(|album| unstarring(&ui.store.library, &album.id));
    let apply = move |ui: &mut Ui| {
        let store = &mut ui.store;
        for album in albums {
            store
                .library
                .toggle_album(album, &store.session, &mut store.cx);
        }
    };
    match starred {
        true => apply(ui),
        false => ask(ui, Kind::Albums(count), apply),
    }
}

/// Unfollows artists, asking first where the heart is the library itself.
pub(crate) fn artists(ui: &mut Ui, artists: Vec<SavedArtist>) {
    if artists.is_empty() {
        return;
    }
    let count = artists.len();
    let starred = artists
        .first()
        .is_some_and(|artist| unstarring(&ui.store.library, &artist.id));
    let apply = move |ui: &mut Ui| {
        let store = &mut ui.store;
        for artist in artists {
            store
                .library
                .toggle_artist(artist, &store.session, &mut store.cx);
        }
    };
    match starred {
        true => apply(ui),
        false => ask(ui, Kind::Artists(count), apply),
    }
}

/// Takes playlists out of the library.
pub(crate) fn playlists(ui: &mut Ui, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    ask(ui, Kind::Playlists(ids.len()), move |ui: &mut Ui| {
        let store = &mut ui.store;
        store
            .library
            .remove_playlists_from_library(ids, &store.session, &mut store.cx);
    });
}

/// Deletes local track files from the disk, then hides the tracks and takes them out of the
/// queue. A file that cannot be deleted says so in a toast.
pub(crate) fn delete_track_files(ui: &mut Ui, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    ask(ui, Kind::DeleteTrackFiles(ids.len()), move |ui: &mut Ui| {
        let store = &mut ui.store;
        let Some(provider) = store.session.local_client() else {
            return;
        };
        let io = store.cx.io.clone();
        store
            .cx
            .spawn(move |link| async move {
                let result = io
                    .spawn(async move {
                        let mut failed = 0;
                        let mut deleted = Vec::new();
                        for id in ids {
                            match provider.delete_track_file(&id).await {
                                Ok(()) => deleted.push(id),
                                Err(error) => {
                                    failed += 1;
                                    log::warn!("local: cannot delete track file {id}: {error:#}");
                                }
                            }
                        }
                        (failed, deleted)
                    })
                    .await;
                link.post(move |store| {
                    if let Ok((_, deleted)) = &result {
                        store.library.hide_local_tracks(deleted, &mut store.cx);
                        store.remove_from_queue(deleted);
                    }
                    match result {
                        Ok((failed, _)) if failed > 0 => Toasts::show(
                            Outcome::Failed,
                            "toast-local-delete-failed",
                            &mut store.cx,
                        ),
                        Err(error) => {
                            log::warn!("local: deletion task failed: {error}");
                            Toasts::show(
                                Outcome::Failed,
                                "toast-local-delete-failed",
                                &mut store.cx,
                            );
                        }
                        Ok(_) => {}
                    }
                });
            })
            .detach();
    });
}

/// Puts the question away unanswered.
pub(crate) fn close(ui: &mut Ui) {
    ui.dialogs.confirm = None;
}

/// Answers yes: puts the question away, then does what it asked about.
fn apply(ui: &mut Ui) {
    let Some(pending) = ui.dialogs.confirm.take() else {
        return;
    };
    (pending.apply)(ui);
    ui.settle();
}

/// Mounts the question, if one is up, into `layer`.
pub(crate) fn view(ui: &Ui, layer: &mut Vec<rune::El>) {
    let Some(pending) = &ui.dialogs.confirm else {
        return;
    };
    let theme = &ui.theme;
    let kind = &pending.kind;
    Modal::new("confirm-remove", kind.title())
        .detail(kind.detail())
        .action(
            Button::new("cancel-confirm")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(close)
                .build(theme),
        )
        .action(
            Button::new("apply-confirm")
                .destructive()
                .label(kind.action())
                .on_press(apply)
                .build(theme)
                .default_button(),
        )
        .on_dismiss(close)
        .build(theme, layer);
}
