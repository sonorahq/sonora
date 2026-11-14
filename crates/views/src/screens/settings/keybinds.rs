use std::collections::BTreeMap;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Keystroke, SharedString, Window, div};
use i18n::t;
use input::{Section, Shortcut};
use state::{Outcome, Toasts};
use ui::{ActiveTheme as _, Button, Kbd};

use super::{Group, SettingsView, Slot};

impl SettingsView {
    /// One group per section of shortcuts, then the row that resets them all under no heading.
    pub(super) fn keybinds_groups(&self) -> Vec<Group> {
        let shortcuts: Vec<&Shortcut> = input::shortcuts().collect();
        Section::ALL
            .into_iter()
            .filter_map(|section| {
                let slots: Vec<Slot> = shortcuts
                    .iter()
                    .enumerate()
                    .filter(|(_, shortcut)| shortcut.section == section)
                    .map(|(index, _)| Slot::Keybind(index))
                    .collect();
                (!slots.is_empty()).then(|| Group::new(section.key(), section.icon()).rows(slots))
            })
            .chain(std::iter::once(
                Group::untitled().rows([Slot::KeybindsReset]),
            ))
            .collect()
    }

    /// The row of the shortcut at `index`: what it does, a button to set it back to its
    /// defaults once changed, and its key, which a click starts recording anew.
    pub(super) fn keybind_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(shortcut) = input::shortcuts().nth(index) else {
            return div().into_any_element();
        };
        let id = shortcut.id;
        let title = i18n::lookup(shortcut.key, None);
        let settings = self.settings.read(cx);
        let changed = shortcut.changed(settings.keybinds());
        let shown = input::shown(id, settings.keybinds());
        let recording = self.recording == Some(id);

        let muted = cx.theme().muted_foreground;
        let key = Button::new(("keybind", index))
            .small()
            .ghost()
            .selected(recording)
            .name(t!("keybinds-change", action = title.as_ref()))
            .map(|button| match (recording, shown) {
                (true, _) => button.label(t!("keybinds-recording")),
                (false, Some(keystroke)) => {
                    button.leading(Kbd::new(keystroke).large().spoken(("keybind-keys", index)))
                }
                (false, None) => button.label(t!("keybinds-none")).tint(muted),
            })
            .on_click(cx.listener(
                move |this, _, window, cx| match this.recording == Some(id) {
                    true => this.stop_recording(cx),
                    false => this.record(id, window, cx),
                },
            ));
        let reset = changed.then(|| {
            Button::new(("keybind-reset", index))
                .small()
                .ghost()
                .icon("icons/undo-2.svg")
                .tooltip("keybinds-reset")
                .on_click(cx.listener(move |this, _, _, cx| this.reset_keybind(id, cx)))
        });

        self.row(
            title,
            div()
                .flex()
                .items_center()
                .gap_1()
                .children(reset)
                .child(key)
                .into_any_element(),
        )
    }

    /// The row that sets every shortcut back to its defaults.
    pub(super) fn keybinds_reset_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let changed = !self.settings.read(cx).keybinds().is_empty();
        self.row(
            t!("keybinds-reset-all"),
            Button::new("keybinds-reset-all")
                .small()
                .secondary()
                .label(t!("keybinds-reset-all-button"))
                .disabled(!changed)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.stop_recording(cx);
                    this.settings.update(cx, |settings, cx| {
                        settings.set_keybinds(Default::default(), cx)
                    });
                }))
                .into_any_element(),
        )
    }

    /// Starts listening for the next key pressed anywhere in the app, which becomes the key of
    /// the shortcut `id`. The press goes nowhere else, so it never fires what it was bound to.
    fn record(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        // no field may keep the focus, or the press would type into it as well
        window.focus(&self.focus, cx);
        self.recording = Some(id);
        let view = cx.entity().downgrade();
        self.recorder = Some(cx.intercept_keystrokes(move |event, _, cx| {
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            view.update(cx, |this, cx| this.recorded(&keystroke, cx))
                .ok();
        }));
        cx.notify();
    }

    /// Stops listening for a new key, leaving the shortcut as it was.
    pub(super) fn stop_recording(&mut self, cx: &mut Context<Self>) {
        if self.recording.take().is_some() {
            self.recorder = None;
            cx.notify();
        }
    }

    /// Takes a key pressed while recording. Escape stops without a change, Backspace leaves the
    /// shortcut without a key, and any other press becomes its key. A modifier pressed alone
    /// keeps the recording going, since it starts a combination.
    fn recorded(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        let Some(id) = self.recording else {
            return;
        };
        let bare = !keystroke.modifiers.modified();
        if bare && keystroke.key == "escape" {
            self.stop_recording(cx);
            return;
        }
        if bare && keystroke.key == "backspace" {
            self.stop_recording(cx);
            self.change_keybinds(cx, |keybinds| {
                input::clear(keybinds, id);
                Vec::new()
            });
            return;
        }
        let Some(spelled) = input::keystroke_of(keystroke) else {
            return;
        };
        self.stop_recording(cx);
        self.change_keybinds(cx, |keybinds| input::assign(keybinds, id, &spelled));
    }

    /// Sets the shortcut `id` back to the keys it has out of the box.
    fn reset_keybind(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.stop_recording(cx);
        self.change_keybinds(cx, |keybinds| input::reset(keybinds, id));
    }

    /// Saves the shortcuts as `change` leaves them, and says which shortcuts lost a key to the
    /// change, since they stop answering to it.
    fn change_keybinds(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut BTreeMap<String, Vec<String>>) -> Vec<&'static Shortcut>,
    ) {
        let mut keybinds = self.settings.read(cx).keybinds().clone();
        let taken = change(&mut keybinds);
        self.settings
            .update(cx, |settings, cx| settings.set_keybinds(keybinds, cx));
        for shortcut in taken {
            Toasts::about(
                Outcome::Info,
                "toast-keybind-taken",
                i18n::lookup(shortcut.key, None),
                cx,
            );
        }
    }

    /// The name a search finds the shortcut at `index` by, with its current key as the detail.
    pub(super) fn keybind_meta(
        &self,
        index: usize,
        cx: &App,
    ) -> Option<(SharedString, SharedString)> {
        let shortcut = input::shortcuts().nth(index)?;
        let shown =
            input::shown(shortcut.id, self.settings.read(cx).keybinds()).unwrap_or_default();
        Some((i18n::lookup(shortcut.key, None), SharedString::from(shown)))
    }
}
