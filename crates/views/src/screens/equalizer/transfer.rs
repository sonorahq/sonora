//! Equalizer APO text in and out: importing a profile from a file or the clipboard, and writing
//! the playing one out to either.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};

use i18n::t;
use music::equalizer::MAX_FILTERS;
use music::equalizer::apo::{self, Reason, Refused, Skipped};
use rune::{ClipboardContent, El, FileDialogOptions, FileDialogResult, Tx, col, row};
use state::{Outcome, Store, Toast, Toasts, equalizer_profile_label};
use ui::Button;

use super::heading;
use super::rows::invalid_text;
use crate::Ui;
use crate::screens::settings::Row;
use crate::shared::page::rems;

/// How many skipped lines an import toast spells out.
const REASONS: usize = 3;
/// What AutoEQ appends to a file name, which a profile named after the file leaves out.
const AUTOEQ_SUFFIX: &str = " ParametricEQ";
/// The icon before the group's heading.
const ICON: &str = "icons/arrow-up-down.svg";

/// The import and export group: a row that reads Equalizer APO text in from a file or the
/// clipboard, and a row that writes the playing profile out to either.
pub(super) fn view(ui: &Ui) -> El {
    let theme = &ui.theme;
    let button = |key: &str, label: String| Button::new(key).small().outline().label(label);
    let pair = |key: &str, first: El, second: El| {
        row()
            .key(key.to_owned())
            .center_y()
            .gap(rems(theme, 0.5))
            .child(first)
            .child(second)
    };
    let import = pair(
        "equalizer-import-buttons",
        button("equalizer-paste", t!("equalizer-paste").into_owned())
            .on_press_tx(|_: &mut Ui, tx: &mut Tx| paste(tx))
            .build(theme),
        button(
            "equalizer-import-file",
            t!("equalizer-import-file").into_owned(),
        )
        .on_press_tx(|_: &mut Ui, tx: &mut Tx| import_file(tx))
        .build(theme),
    );
    let export = pair(
        "equalizer-export-buttons",
        button("equalizer-copy", t!("equalizer-copy").into_owned())
            .on_press_tx(copy)
            .build(theme),
        button(
            "equalizer-export-file",
            t!("equalizer-export-file").into_owned(),
        )
        .on_press_tx(export_file)
        .build(theme),
    );

    col()
        .key("equalizer-transfer-group")
        .w_full()
        .child(heading(theme, "equalizer-transfer", ICON, None))
        .child(Row::new("equalizer-import", t!("equalizer-import"), import).build(theme))
        .child(Row::new("equalizer-export", t!("equalizer-export"), export).build(theme))
}

/// Makes a new profile from an Equalizer APO text and says what came of it, with the lines it
/// left out.
fn import(store: &mut Store, name: &str, text: &str) {
    let Store {
        equalizer,
        settings,
        cx,
        ..
    } = store;
    let result = equalizer.import(name, text, settings, cx);
    // The new profile plays, under the name the settings made unique.
    let name = equalizer_profile_label(settings.equalizer_profile_name());
    let toast = match result {
        Ok(skipped) if skipped.is_empty() => {
            Toast::new(Outcome::Done, "equalizer-imported").name(name)
        }
        Ok(skipped) => Toast::new(Outcome::Warning, "equalizer-imported-skipped")
            .name(name)
            .count(skipped.len())
            .detail(reasons(&skipped)),
        Err(Refused::TooMany(count)) => Toast::new(Outcome::Failed, "equalizer-import-too-many")
            .count(count)
            .detail(t!("equalizer-filter-limit", max = MAX_FILTERS)),
        Err(Refused::TooLarge) => Toast::new(Outcome::Failed, "equalizer-import-too-large"),
        Err(Refused::Nothing(skipped)) if skipped.is_empty() => {
            Toast::new(Outcome::Failed, "equalizer-import-nothing")
        }
        Err(Refused::Nothing(skipped)) => {
            Toast::new(Outcome::Failed, "equalizer-import-nothing").detail(reasons(&skipped))
        }
    };
    Toasts::post(toast, cx);
}

/// Reads the clipboard and imports what it holds.
fn paste(tx: &mut Tx) {
    tx.read_clipboard(|ui: &mut Ui, content: &ClipboardContent, _: &mut Tx| {
        let text = content.text();
        match text.trim().is_empty() {
            true => Toasts::show(Outcome::Failed, "equalizer-paste-empty", &mut ui.store.cx),
            false => import(&mut ui.store, &t!("equalizer-profile-pasted"), text),
        }
        ui.settle();
    });
}

/// Asks for a file and imports it, read off the UI thread.
fn import_file(tx: &mut Tx) {
    tx.open_file(
        FileDialogOptions::default(),
        |ui: &mut Ui, result: FileDialogResult, _: &mut Tx| {
            match result {
                Ok(Some(path)) => read(ui, path),
                Ok(None) => {}
                Err(error) => dialog_failed(ui, &error.0),
            }
            ui.settle();
        },
    );
}

/// Reads the file at `path` on tokio and imports it, named after the file.
fn read(ui: &mut Ui, path: PathBuf) {
    let io = ui.store.cx.io.clone();
    let job = ui.store.cx.spawn(move |link| async move {
        let read = io
            .spawn_blocking({
                let path = path.clone();
                move || read_capped(&path)
            })
            .await
            .unwrap_or_else(|error| Err(io::Error::other(error)));
        link.post(move |store| match read {
            Ok(bytes) => {
                let stem = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let name = stem.strip_suffix(AUTOEQ_SUFFIX).unwrap_or(&stem).trim();
                let name = match name.is_empty() {
                    true => t!("equalizer-profile-pasted").into_owned(),
                    false => name.to_owned(),
                };
                import(store, &name, &apo::decode(&bytes));
            }
            Err(error) => {
                log::warn!("equalizer: cannot read {}: {error}", path.display());
                Toasts::post(
                    Toast::new(Outcome::Failed, "equalizer-file-unreadable")
                        .detail(error.to_string()),
                    &mut store.cx,
                );
            }
        });
    });
    ui.screens.equalizer.file = Some(job);
}

/// Copies the playing profile as Equalizer APO text.
fn copy(ui: &mut Ui, tx: &mut Tx) {
    tx.copy(ui.store.equalizer.export());
    Toasts::show(Outcome::Done, "equalizer-copied", &mut ui.store.cx);
    ui.settle();
}

/// Asks where to save the playing profile and writes it there as Equalizer APO text, on tokio.
fn export_file(ui: &mut Ui, tx: &mut Tx) {
    let text = ui.store.equalizer.export();
    let name = equalizer_profile_label(ui.store.settings.equalizer_profile_name())
        .replace(['/', '\\', ':'], "-");
    let options = FileDialogOptions {
        directory: dirs::document_dir().or_else(dirs::home_dir),
        file_name: Some(format!("{name}.txt")),
        ..Default::default()
    };
    tx.save_file(
        options,
        move |ui: &mut Ui, result: FileDialogResult, _: &mut Tx| {
            match result {
                Ok(Some(path)) => write(ui, path, text.clone()),
                Ok(None) => {}
                Err(error) => dialog_failed(ui, &error.0),
            }
            ui.settle();
        },
    );
}

/// Writes `text` to `path` on tokio and says how it went.
fn write(ui: &mut Ui, path: PathBuf, text: String) {
    let io = ui.store.cx.io.clone();
    let job = ui.store.cx.spawn(move |link| async move {
        let written = io
            .spawn_blocking({
                let path = path.clone();
                move || std::fs::write(path, text)
            })
            .await
            .unwrap_or_else(|error| Err(io::Error::other(error)));
        link.post(move |store| match written {
            Ok(()) => Toasts::show(Outcome::Done, "equalizer-exported", &mut store.cx),
            Err(error) => {
                log::warn!("equalizer: cannot write {}: {error}", path.display());
                Toasts::post(
                    Toast::new(Outcome::Failed, "equalizer-file-unwritable")
                        .detail(error.to_string()),
                    &mut store.cx,
                );
            }
        });
    });
    ui.screens.equalizer.file = Some(job);
}

/// Says a file dialog could not open.
fn dialog_failed(ui: &mut Ui, error: &str) {
    log::warn!("equalizer: cannot open the file dialog: {error}");
    Toasts::show(Outcome::Failed, "equalizer-dialog-failed", &mut ui.store.cx);
}

/// The first skipped lines of an import, one per line of text, and how many more there are.
fn reasons(skipped: &[Skipped]) -> String {
    let mut lines: Vec<String> = skipped
        .iter()
        .take(REASONS)
        .map(|skipped| {
            t!(
                "equalizer-skipped-line",
                line = skipped.line,
                reason = reason_text(&skipped.reason).to_string()
            )
            .into_owned()
        })
        .collect();
    if skipped.len() > REASONS {
        lines.push(t!("equalizer-skipped-more", count = skipped.len() - REASONS).into_owned());
    }
    lines.join("\n")
}

/// The file at `path` read up to one byte past `apo::MAX_TEXT`, enough for the import to refuse
/// an oversized file without holding all of it.
fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(apo::MAX_TEXT as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn reason_text(reason: &Reason) -> i18n::SharedString {
    match reason {
        Reason::UnknownType(name) => t!("equalizer-skip-unknown-type", name = name),
        Reason::Unreadable => t!("equalizer-skip-unreadable"),
        Reason::NoFrequency => t!("equalizer-skip-no-frequency"),
        Reason::NoGain => t!("equalizer-skip-no-gain"),
        Reason::NoWidth => t!("equalizer-skip-no-width"),
        Reason::Expression => t!("equalizer-skip-expression"),
        Reason::NoPreamp => t!("equalizer-skip-no-preamp"),
        Reason::Invalid(invalid) => invalid_text(*invalid),
        Reason::Unsupported(command) => t!("equalizer-skip-unsupported", name = command),
    }
}
