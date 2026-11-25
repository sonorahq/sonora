use gpui::prelude::*;
use gpui::{AnyElement, Context, SharedString, div};
use i18n::{Language, t};
use router::Screen;
use ui::{Button, MenuItem, Picker, Saver, Switch};

use super::{Group, LANGUAGES, SettingsView, Slot};
use crate::shared::popups::matches_query;

const STARTUP: &str = "startup";
const PROVIDER: &str = "provider";

impl SettingsView {
    /// The General page: how Sonora starts, how its window lives on the desktop, and how its
    /// menus behave. How things look is on the Appearance tab.
    pub(super) fn general_groups(&self) -> Vec<Group> {
        vec![
            Group::new("settings-group-startup").rows([
                Slot::Startup,
                Slot::Provider,
                Slot::Language,
            ]),
            Group::new("settings-group-window")
                .rows([Slot::Tray, Slot::TrayIcon, Slot::NowPlayingOnFileOpen])
                .rows(taskbar_slots())
                .rows([Slot::Saver]),
            Group::new("settings-group-context-menus").rows([Slot::AdaptiveMenu]),
        ]
    }

    pub(super) fn startup_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = Screen::from_id(self.settings.read(cx).startup()).unwrap_or(Screen::Home);
        let current = i18n::lookup(chosen.key(), None);

        let picker = Picker::new(STARTUP, &self.popovers, current)
            .width(Picker::NARROW)
            .items(Screen::ALL.map(|screen| {
                MenuItem::new(screen.id(), i18n::lookup(screen.key(), None))
                    .selected(screen == chosen)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_startup(screen.id(), cx));
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-startup"),
            picker.name(t!("settings-startup")).into_any_element(),
        )
    }

    pub(super) fn provider_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let configured = self.settings.read(cx).startup_provider().map(str::to_owned);
        let providers: Vec<_> = self
            .session
            .read(cx)
            .providers()
            .map(|provider| (provider.slug, provider.name))
            .collect();
        let current = match configured.as_deref() {
            None => t!("settings-provider-last-used"),
            Some(slug) => providers
                .iter()
                .find(|(provider, _)| *provider == slug)
                .map(|(_, name)| SharedString::from(*name))
                .unwrap_or_else(|| t!("common-unavailable", name = slug)),
        };

        let mut picker = Picker::new(PROVIDER, &self.popovers, current)
            .width(Picker::REGULAR)
            .items([
                MenuItem::new("provider-last-used", t!("settings-provider-last-used"))
                    .selected(configured.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_startup_provider(None, cx);
                        });
                        this.popovers.close();
                        cx.notify();
                    })),
            ]);

        for (slug, name) in providers {
            let selected = configured.as_deref() == Some(slug);
            picker = picker.item(
                MenuItem::new(slug, SharedString::from(name))
                    .selected(selected)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_startup_provider(Some(slug), cx);
                        });
                        this.popovers.close();
                        cx.notify();
                    })),
            );
        }

        self.row(
            t!("settings-provider"),
            picker.name(t!("settings-provider")).into_any_element(),
        )
    }

    pub(super) fn language_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).language().to_owned();
        let current = match Language::from_id(&chosen) {
            Some(language) => SharedString::from(language.label()),
            None => t!("settings-language-system"),
        };

        let asked = self.languages.query();
        let entries = std::iter::once((i18n::AUTO, t!("settings-language-system")))
            .chain(
                Language::ALL
                    .into_iter()
                    .map(|language| (language.id(), SharedString::from(language.label()))),
            )
            .filter(|(id, label)| matches_query(id, label, &asked))
            .collect::<Vec<_>>();
        let barren = entries.is_empty();
        let count = entries.len();
        let cursor = self.languages.cursor(count);
        let submitted = entries.clone();

        let picker = Picker::new(LANGUAGES, &self.popovers, current)
            .name(t!("settings-language"))
            .width(Picker::WIDE)
            .menu(self.languages.menu("languages-menu", Picker::WIDE))
            .items(entries.into_iter().enumerate().map(|(place, (id, label))| {
                MenuItem::new(id, label)
                    .highlighted(place == cursor)
                    .checked(chosen == id)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings
                            .update(cx, |settings, cx| settings.set_language(id, cx));
                        this.popovers.close();
                        cx.notify();
                    }))
            }))
            .when(barren, |picker| {
                picker
                    .item(MenuItem::new("language-empty", t!("settings-language-none")).disabled())
            });
        let picker = self.languages.controls(
            picker,
            count,
            move |this, place, _, cx| {
                let Some((id, _)) = submitted.get(place) else {
                    return;
                };
                this.settings
                    .update(cx, |settings, cx| settings.set_language(*id, cx));
                this.popovers.close();
                cx.notify();
            },
            cx,
        );

        self.row(t!("settings-language"), picker.into_any_element())
    }

    /// The frame cap of a window in the background as a stepper: minus caps it lower, plus
    /// lifts it towards no cap at all.
    pub(super) fn saver_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.read(cx).saver();
        let place = Saver::ALL
            .iter()
            .position(|saver| *saver == current)
            .unwrap_or_default();
        // the savers run from no cap to the lowest, so a lower cap is the next one along
        let lower = Saver::ALL.get(place + 1).copied();
        let higher = place
            .checked_sub(1)
            .and_then(|place| Saver::ALL.get(place))
            .copied();
        let step =
            |id: &'static str, label: &'static str, name: &'static str, next: Option<Saver>| {
                Button::new(id)
                    .label(label)
                    .name(i18n::lookup(name, None))
                    .small()
                    .outline()
                    .disabled(next.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(next) = next else { return };
                        this.settings
                            .update(cx, |settings, cx| settings.set_saver(next, cx));
                        cx.notify();
                    }))
            };

        let stepper = div()
            .id("settings-saver-stepper")
            .role(gpui::accesskit::Role::Group)
            .aria_label(t!("settings-saver"))
            .flex()
            .items_center()
            .gap_2()
            .child(step(
                "saver-lower",
                "\u{2212}",
                "settings-saver-lower",
                lower,
            ))
            .child(ui::spoken_steady(
                "settings-saver-value",
                current.label(),
                Saver::ALL.map(Saver::label),
            ))
            .child(step("saver-higher", "+", "settings-saver-higher", higher));

        super::Row::new(t!("settings-saver"), stepper.into_any_element())
            .detail(t!("settings-saver-detail"))
            .into_any_element()
    }

    pub(super) fn adaptive_menu_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).adaptive_menu();

        self.row(
            t!("settings-adaptive-menu"),
            Switch::new("adaptive-menu", on)
                .name(t!("settings-adaptive-menu"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_adaptive_menu(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn tray_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).close_to_tray();

        self.row(
            t!("settings-close-to-tray"),
            Switch::new("close-to-tray", on)
                .name(t!("settings-close-to-tray"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_close_to_tray(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn tray_icon_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).tray_icon();

        self.row(
            t!("settings-tray-icon"),
            Switch::new("tray-icon", on)
                .name(t!("settings-tray-icon"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_tray_icon(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn now_playing_on_file_open_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).now_playing_on_file_open();

        self.row(
            t!("settings-now-playing-on-file-open"),
            Switch::new("now-playing-on-file-open", on)
                .name(t!("settings-now-playing-on-file-open"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_now_playing_on_file_open(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    pub(super) fn taskbar_progress_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).taskbar_progress();

        self.row(
            t!("settings-taskbar-progress"),
            Switch::new("taskbar-progress", on)
                .name(t!("settings-taskbar-progress"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_taskbar_progress(!on, cx));
                }))
                .into_any_element(),
        )
    }
}

/// The rows about the Windows taskbar button. Only Windows has that button, so the list is
/// empty everywhere else and the row is left off the page.
fn taskbar_slots() -> Vec<Slot> {
    match cfg!(target_os = "windows") {
        true => vec![Slot::TaskbarProgress],
        false => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taskbar_progress_row_is_listed_only_on_windows() {
        let slots = taskbar_slots();
        assert_eq!(
            slots.contains(&Slot::TaskbarProgress),
            cfg!(target_os = "windows")
        );
        assert_eq!(slots.is_empty(), !cfg!(target_os = "windows"));
    }
}
