use gpui::prelude::*;
use gpui::{AnyElement, ClipboardItem, Context, PathPromptOptions, SharedString, div};
use i18n::t;
use music::equalizer::MAX_FILTERS;
use music::equalizer::apo::{self, Reason, Refused, Skipped};
use state::{Outcome, Toast, Toasts, equalizer_profile_label};
use ui::{ActiveTheme as _, Button, Text, faint};

use super::EqualizerView;
use super::rows::invalid_text;

/// How many skipped lines an import toast spells out.
const REASONS: usize = 3;
/// What AutoEQ appends to a file name, which a profile named after the file leaves out.
const AUTOEQ_SUFFIX: &str = " ParametricEQ";

impl EqualizerView {
    /// Makes a new profile from an Equalizer APO text and says what came of it, with the
    /// lines it left out.
    fn import(&mut self, name: &str, text: &str, cx: &mut Context<Self>) {
        let result = self
            .playback
            .update(cx, |playback, cx| playback.import_equalizer(name, text, cx));
        // The new profile plays, under the name the settings made unique.
        let name = equalizer_profile_label(self.settings.read(cx).equalizer_profile_name());
        let toast = match result {
            Ok(skipped) if skipped.is_empty() => {
                Toast::new(Outcome::Done, "equalizer-imported").name(name)
            }
            Ok(skipped) => Toast::new(Outcome::Warning, "equalizer-imported-skipped")
                .name(name)
                .count(skipped.len())
                .detail(reasons(&skipped)),
            Err(Refused::TooMany(count)) => {
                Toast::new(Outcome::Failed, "equalizer-import-too-many")
                    .count(count)
                    .detail(t!("equalizer-filter-limit", max = MAX_FILTERS))
            }
            Err(Refused::Nothing(skipped)) if skipped.is_empty() => {
                Toast::new(Outcome::Failed, "equalizer-import-nothing")
            }
            Err(Refused::Nothing(skipped)) => {
                Toast::new(Outcome::Failed, "equalizer-import-nothing").detail(reasons(&skipped))
            }
        };
        Toasts::post(toast, cx);
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            Toasts::show(Outcome::Failed, "equalizer-paste-empty", cx);
            return;
        };
        self.import(&t!("equalizer-profile-pasted"), &text, cx);
    }

    fn import_file(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        self.file = Some(cx.spawn(async move |this, cx| {
            let path = match receiver.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(error)) => {
                    log::warn!("equalizer: cannot open the file dialog: {error:#}");
                    cx.update(|cx| Toasts::show(Outcome::Failed, "equalizer-dialog-failed", cx));
                    None
                }
                _ => None,
            };
            let Some(path) = path else {
                return;
            };
            let read = cx
                .background_spawn({
                    let path = path.clone();
                    async move { std::fs::read(path) }
                })
                .await;
            this.update(cx, |this, cx| match read {
                Ok(bytes) => {
                    let stem = path
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let name = stem.strip_suffix(AUTOEQ_SUFFIX).unwrap_or(&stem).trim();
                    let name = match name.is_empty() {
                        true => t!("equalizer-profile-pasted").to_string(),
                        false => name.to_owned(),
                    };
                    this.import(&name, &apo::decode(&bytes), cx);
                }
                Err(error) => {
                    log::warn!("equalizer: cannot read {}: {error}", path.display());
                    Toasts::post(
                        Toast::new(Outcome::Failed, "equalizer-file-unreadable")
                            .detail(error.to_string()),
                        cx,
                    );
                }
            })
            .ok();
        }));
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        let text = self.playback.read(cx).export_equalizer();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        Toasts::show(Outcome::Done, "equalizer-copied", cx);
    }

    fn export_file(&mut self, cx: &mut Context<Self>) {
        let text = self.playback.read(cx).export_equalizer();
        let name = equalizer_profile_label(self.settings.read(cx).equalizer_profile_name())
            .replace(['/', '\\', ':'], "-");
        let directory = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let receiver = cx.prompt_for_new_path(&directory, Some(&format!("{name}.txt")));
        self.file = Some(cx.spawn(async move |_, cx| {
            let path = match receiver.await {
                Ok(Ok(path)) => path,
                Ok(Err(error)) => {
                    log::warn!("equalizer: cannot open the file dialog: {error:#}");
                    cx.update(|cx| Toasts::show(Outcome::Failed, "equalizer-dialog-failed", cx));
                    None
                }
                Err(_) => None,
            };
            let Some(path) = path else {
                return;
            };
            let written = cx
                .background_spawn({
                    let path = path.clone();
                    async move { std::fs::write(path, text) }
                })
                .await;
            cx.update(|cx| match written {
                Ok(()) => Toasts::show(Outcome::Done, "equalizer-exported", cx),
                Err(error) => {
                    log::warn!("equalizer: cannot write {}: {error}", path.display());
                    Toasts::post(
                        Toast::new(Outcome::Failed, "equalizer-file-unwritable")
                            .detail(error.to_string()),
                        cx,
                    );
                }
            });
        }));
    }

    pub(super) fn transfer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = *cx.theme();
        let button =
            |id: &'static str, label: SharedString| Button::new(id).small().outline().label(label);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(faint(cx).child(t!("equalizer-transfer")))
            .child(
                div()
                    .text_size(theme.text(Text::Small))
                    .text_color(theme.muted_foreground)
                    .child(t!("equalizer-transfer-detail")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        button("equalizer-import-file", t!("equalizer-import-file"))
                            .on_click(cx.listener(|this, _, _, cx| this.import_file(cx))),
                    )
                    .child(
                        button("equalizer-paste", t!("equalizer-paste"))
                            .on_click(cx.listener(|this, _, _, cx| this.paste(cx))),
                    )
                    .child(
                        button("equalizer-copy", t!("equalizer-copy"))
                            .on_click(cx.listener(|this, _, _, cx| this.copy(cx))),
                    )
                    .child(
                        button("equalizer-export-file", t!("equalizer-export-file"))
                            .on_click(cx.listener(|this, _, _, cx| this.export_file(cx))),
                    ),
            )
            .into_any_element()
    }
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
            .to_string()
        })
        .collect();
    if skipped.len() > REASONS {
        lines.push(t!("equalizer-skipped-more", count = skipped.len() - REASONS).to_string());
    }
    lines.join("\n")
}

fn reason_text(reason: &Reason) -> SharedString {
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
