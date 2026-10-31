use gpui::{App, ElementId, PathPromptOptions};
use i18n::t;
use music::Playlist;
use state::{Outcome, Sonora, Toasts};
use ui::{Button, Vacancy};

/// The extension an exported playlist is saved with, read as UTF-8 by every player.
const PLAYLIST_EXTENSION: &str = "m3u8";

pub(crate) fn choose_button(id: impl Into<ElementId>) -> Button {
    Button::new(id)
        .label(t!("settings-choose-folder"))
        .on_click(|_, _, cx| choose_folder(cx))
}

pub(crate) fn unconfigured(id: impl Into<ElementId>) -> Vacancy {
    let id: ElementId = id.into();
    Vacancy::new(t!("library-local-unconfigured"))
        .spoken((id.clone(), "caption"))
        .icon("icons/folder-plus.svg")
        .action(choose_button(id).outline())
}

pub(crate) fn choose_folder(cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: None,
    });
    let library = Sonora::global(cx).library.clone();
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = receiver.await else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        library.update(cx, |library, cx| library.add_local_folders(paths, cx));
    })
    .detach();
}

/// Asks for an M3U or M3U8 file and reads it into a new local playlist.
pub(crate) fn import_playlist(cx: &mut App) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: None,
    });
    let library = Sonora::global(cx).library.clone();
    cx.spawn(async move |cx| {
        let path = match receiver.await {
            Ok(Ok(Some(paths))) => paths.into_iter().next(),
            Ok(Err(error)) => {
                log::warn!("library: cannot open the file dialog: {error:#}");
                cx.update(|cx| Toasts::show(Outcome::Failed, "toast-file-dialog-failed", cx));
                None
            }
            _ => None,
        };
        let Some(path) = path else {
            return;
        };
        library.update(cx, |library, cx| library.import_playlist(path, cx));
    })
    .detach();
}

/// Asks where to save a local playlist and writes it there as M3U8.
pub(crate) fn export_playlist(playlist: Playlist, cx: &mut App) {
    let name = playlist
        .name
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "-");
    let directory = dirs::audio_dir()
        .or_else(dirs::document_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_default();
    let receiver =
        cx.prompt_for_new_path(&directory, Some(&format!("{name}.{PLAYLIST_EXTENSION}")));
    let library = Sonora::global(cx).library.clone();
    cx.spawn(async move |cx| {
        let path = match receiver.await {
            Ok(Ok(path)) => path,
            Ok(Err(error)) => {
                log::warn!("library: cannot open the file dialog: {error:#}");
                cx.update(|cx| Toasts::show(Outcome::Failed, "toast-file-dialog-failed", cx));
                None
            }
            Err(_) => None,
        };
        let Some(path) = path else {
            return;
        };
        library.update(cx, |library, cx| {
            library.export_playlist(playlist.id, path, cx)
        });
    })
    .detach();
}
