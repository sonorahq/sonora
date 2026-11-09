//! Local files: choosing the folders the local library reads, and importing and exporting
//! playlists as M3U8, through the system's file dialogs.

use i18n::t;
use music::Playlist;
use rune::{El, FileDialogOptions, FileDialogResult, Tx};
use state::Outcome;
use ui::{Button, Theme, Vacancy};

use crate::Ui;

/// The extension an exported playlist is saved with, read as UTF-8 by every player.
const PLAYLIST_EXTENSION: &str = "m3u8";

/// The button that asks for a folder of music to add to the local library.
pub(crate) fn choose_button(key: impl Into<String>) -> Button {
    Button::new(key)
        .label(t!("settings-choose-folder"))
        .on_press_tx(|_: &mut Ui, tx: &mut Tx| choose_folder(tx))
}

/// What the local library shows before it has a folder: a line saying so and the button that
/// asks for one.
pub(crate) fn unconfigured(key: &str, theme: &Theme) -> El {
    Vacancy::new(t!("library-local-unconfigured"))
        .spoken(format!("{key}-caption"))
        .icon("icons/folder-plus.svg")
        .action(choose_button(key.to_owned()).outline().build(theme))
        .build(theme)
}

/// Asks for a folder and adds it to the local library.
pub(crate) fn choose_folder(tx: &mut Tx) {
    let options = FileDialogOptions {
        title: t!("settings-choose-folder").into_owned(),
        ..Default::default()
    };
    tx.open_folder(options, |ui: &mut Ui, result: FileDialogResult, _: &mut Tx| {
        match result {
            Ok(Some(path)) => {
                ui.store.add_local_folders(vec![path]);
                ui.settle();
            }
            Ok(None) => {}
            Err(error) => failed(ui, &error.0),
        }
    });
}

/// Asks for an M3U or M3U8 file and reads it into a new local playlist.
pub(crate) fn import_playlist(tx: &mut Tx) {
    let options = FileDialogOptions::default().filter("M3U", &["m3u", PLAYLIST_EXTENSION]);
    tx.open_file(options, |ui: &mut Ui, result: FileDialogResult, _: &mut Tx| {
        match result {
            Ok(Some(path)) => {
                let store = &mut ui.store;
                store.library.import_playlist(path, &store.session, &mut store.cx);
                ui.settle();
            }
            Ok(None) => {}
            Err(error) => failed(ui, &error.0),
        }
    });
}

/// Asks where to save a local playlist and writes it there as M3U8.
pub(crate) fn export_playlist(playlist: &Playlist, tx: &mut Tx) {
    let name = playlist
        .name
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-");
    let directory = dirs::audio_dir()
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir);
    let options = FileDialogOptions {
        directory,
        file_name: Some(format!("{name}.{PLAYLIST_EXTENSION}")),
        ..Default::default()
    };
    let id = playlist.id.clone();
    tx.save_file(options, move |ui: &mut Ui, result: FileDialogResult, _: &mut Tx| {
        match result {
            Ok(Some(path)) => {
                let store = &mut ui.store;
                store.library.export_playlist(id, path, &store.session, &mut store.cx);
                ui.settle();
            }
            Ok(None) => {}
            Err(error) => failed(ui, &error.0),
        }
    });
}

/// Says a file dialog could not open.
fn failed(ui: &mut Ui, error: &str) {
    log::warn!("library: cannot open the file dialog: {error}");
    state::Toasts::show(Outcome::Failed, "toast-file-dialog-failed", &mut ui.store.cx);
    ui.settle();
}
