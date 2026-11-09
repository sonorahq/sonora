//! The playlist dialogs: naming a new playlist, renaming an owned one and changing its cover,
//! confirming a delete, and asking before a track goes into a playlist that holds it already.

use i18n::t;
use music::Playlist;
use rune::{El, FileDialogOptions, TextEdit, Tx, col, row};
use state::{Addition, CoverEdit, Leaving, Outcome, Shelf, Toasts, cover_problem};
use ui::{Button, Cover, Input, Modal};

use crate::Ui;
use crate::shared::dialogs::rems;

/// How wide the dialog is, in covers.
const WIDTH: f32 = 2.8;

/// What the dialog is for.
#[derive(Clone)]
pub(crate) enum Edit {
    Create {
        addition: Addition,
        shelf: Shelf,
    },
    /// The name and the cover of an owned playlist.
    Details(Playlist),
    Delete(Playlist),
    /// A dropped track the playlist already holds, and where it was dragged from when it moves.
    Again {
        playlist: Playlist,
        track: String,
        leaving: Option<Leaving>,
    },
}

/// The dialog's state while it is up.
pub(crate) struct Editor {
    edit: Option<Edit>,
    name: TextEdit,
    /// What saving does to the cover, which the preview already shows.
    cover: CoverEdit,
    /// Whether a file dialog for the cover is up.
    picking: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            edit: None,
            name: TextEdit::default(),
            cover: CoverEdit::Keep,
            picking: false,
        }
    }
}

impl Editor {
    pub(crate) fn shows(&self) -> bool {
        self.edit.is_some()
    }

    pub(crate) fn close(&mut self) {
        self.edit = None;
        self.picking = false;
    }
}

/// Opens the dialog for `edit`, with the name field holding the playlist's name when renaming.
pub(crate) fn open(ui: &mut Ui, edit: Edit) {
    let editor = &mut ui.dialogs.playlist;
    let name = match &edit {
        Edit::Details(playlist) => playlist.name.clone(),
        _ => String::new(),
    };
    editor.name = TextEdit::new(name);
    editor.cover = CoverEdit::Keep;
    editor.picking = false;
    editor.edit = Some(edit);
}

fn close(ui: &mut Ui) {
    ui.dialogs.playlist.close();
}

/// Whether the dialog only asks, with no field to fill.
fn plain(edit: &Edit) -> bool {
    matches!(edit, Edit::Delete(_) | Edit::Again { .. })
}

/// Asks for a picture and shows it in the preview once it passes a quick check. A file that
/// fails says why in a toast and leaves the cover as it was.
fn choose_cover(ui: &mut Ui, tx: &mut Tx) {
    ui.dialogs.playlist.picking = true;
    let options = FileDialogOptions {
        title: t!("playlist-cover-choose").into_owned(),
        ..Default::default()
    }
    .filter("Image", &["jpg", "jpeg", "png", "webp"]);
    tx.open_file(options, |ui: &mut Ui, picked, _: &mut Tx| {
        let editor = &mut ui.dialogs.playlist;
        editor.picking = false;
        match picked {
            Ok(Some(path)) => match music::cover::check_file(&path) {
                Ok(()) => editor.cover = CoverEdit::Replace(path),
                Err(unusable) => {
                    Toasts::show(Outcome::Failed, cover_problem(unusable), &mut ui.store.cx)
                }
            },
            Ok(None) => {}
            Err(error) => {
                log::warn!("library: cannot open the file dialog: {error}");
                Toasts::show(
                    Outcome::Failed,
                    "toast-file-dialog-failed",
                    &mut ui.store.cx,
                );
            }
        }
        ui.settle();
    });
}

/// Saves what the dialog holds and puts it away.
fn apply(ui: &mut Ui) {
    let editor = &mut ui.dialogs.playlist;
    let Some(edit) = editor.edit.take() else {
        return;
    };
    let name = editor.name.text.trim().to_owned();
    let cover = std::mem::replace(&mut editor.cover, CoverEdit::Keep);
    let store = &mut ui.store;
    let (library, session, cx) = (&mut store.library, &store.session, &mut store.cx);
    match edit {
        Edit::Create { addition, shelf } if !name.is_empty() => {
            library.create_playlist(name, addition, shelf, session, cx)
        }
        Edit::Create { .. } => {}
        Edit::Details(playlist) => {
            let renamed = (!name.is_empty() && name != playlist.name).then_some(name);
            library.edit_playlist(playlist.id, renamed, cover, session, cx);
        }
        Edit::Delete(playlist) => library.delete_playlist(playlist.id, session, cx),
        Edit::Again {
            playlist,
            track,
            leaving,
        } => match leaving {
            Some(leaving) => library.move_to_playlist(playlist.id, track, leaving, session, cx),
            None => library.add_to_playlist(playlist.id, track, session, cx),
        },
    }
    ui.settle();
}

/// The cover the preview shows: the picked file, nothing once removed, and otherwise the
/// playlist's current cover as the library knows it.
fn preview(ui: &Ui, playlist: &Playlist) -> Option<String> {
    match &ui.dialogs.playlist.cover {
        CoverEdit::Replace(path) => Some(format!("file://{}", path.display())),
        CoverEdit::Remove => None,
        CoverEdit::Keep => ui
            .store
            .library
            .playlist(&playlist.id)
            .unwrap_or(playlist)
            .cover
            .clone(),
    }
}

/// The name field, which Return saves from.
fn name_field(ui: &Ui) -> El {
    Input::new(
        "playlist-name",
        &ui.dialogs.playlist.name,
        t!("playlist-name-placeholder"),
        |ui: &mut Ui| &mut ui.dialogs.playlist.name,
    )
    .autofocus()
    .on_submit(|ui: &mut Ui, _: &mut Tx| apply(ui))
    .build(&ui.theme)
    .grow(1.)
}

/// The cover preview with its buttons beside the name field. The cover buttons show only where
/// the provider takes a cover.
fn details(ui: &Ui, playlist: &Playlist) -> El {
    let theme = &ui.theme;
    let editor = &ui.dialogs.playlist;
    let covers = ui
        .store
        .session
        .capabilities_of(Shelf::of(&playlist.id))
        .playlist_covers;
    let removable = editor.cover != CoverEdit::Remove;
    let shown = preview(ui, playlist);
    let art = Cover::new(ui.artwork(), shown.as_deref())
        .size(theme.metrics.cover)
        .radius(theme.radius)
        .fallback("icons/list-music.svg")
        .build(theme);
    let mut column = col()
        .key("playlist-details")
        .grow(1.)
        .min_w(0.)
        .gap(rems(theme, 0.5))
        // The field grows along its parent's main axis, so a row keeps it from filling the
        // column down to the cover's height.
        .child(row().w_full().child(name_field(ui)));
    if covers {
        let mut buttons = row()
            .key("playlist-cover-buttons")
            .flex_wrap()
            .gap(rems(theme, 0.5))
            .child(
                Button::new("choose-playlist-cover")
                    .outline()
                    .small()
                    .icon("icons/image.svg")
                    .label(t!("playlist-cover-choose"))
                    .disabled(editor.picking)
                    .on_press_tx(choose_cover)
                    .build(theme),
            );
        if removable {
            buttons = buttons.child(
                Button::new("remove-playlist-cover")
                    .ghost()
                    .small()
                    .icon("icons/image-off.svg")
                    .label(t!("playlist-cover-remove"))
                    .on_press(|ui: &mut Ui| ui.dialogs.playlist.cover = CoverEdit::Remove)
                    .build(theme),
            );
        }
        column = column.child(buttons);
    }
    row()
        .key("playlist-editor-body")
        .w_full()
        .gap(theme.metrics.pad)
        .child(art)
        .child(column)
}

/// Mounts the dialog, if it is up, into `layer`.
pub(crate) fn view(ui: &Ui, layer: &mut Vec<El>) {
    let Some(edit) = &ui.dialogs.playlist.edit else {
        return;
    };
    let theme = &ui.theme;
    let deleting = matches!(edit, Edit::Delete(_));
    let title = match edit {
        Edit::Create { .. } => t!("playlist-create-title"),
        Edit::Details(_) => t!("playlist-edit-title"),
        Edit::Delete(_) => t!("playlist-delete-title"),
        Edit::Again { .. } => t!("playlist-again-title"),
    };
    let detail = match edit {
        Edit::Delete(playlist) => {
            Some(t!("playlist-delete-confirm", name = playlist.name.as_str()))
        }
        Edit::Again { playlist, .. } => {
            Some(t!("playlist-again-confirm", name = playlist.name.as_str()))
        }
        _ => None,
    };
    let mut modal = Modal::new("playlist-editor", title).width(theme.metrics.cover * WIDTH);
    if let Some(detail) = detail {
        modal = modal.detail(detail);
    }
    modal = match edit {
        Edit::Details(playlist) => modal.child(details(ui, playlist)),
        _ if plain(edit) => modal,
        _ => modal.child(row().w_full().child(name_field(ui))),
    };
    let mut save = Button::new("apply-playlist-edit").label(match edit {
        Edit::Delete(_) => t!("common-delete"),
        Edit::Again { .. } => t!("playlist-again-add"),
        _ => t!("common-save"),
    });
    save = match deleting {
        true => save.destructive(),
        false => save.primary(),
    };
    modal
        .action(
            Button::new("cancel-playlist-edit")
                .ghost()
                .label(t!("common-cancel"))
                .on_press(close)
                .build(theme),
        )
        .action(save.on_press(apply).build(theme).default_button())
        .on_dismiss(close)
        .build(theme, layer);
}
