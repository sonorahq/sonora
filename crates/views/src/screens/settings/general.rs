use gpui::prelude::*;
use gpui::{AnyElement, App, Context, SharedString, Window};
use i18n::{Language, t};
use router::{NavEntry, Screen};
use ui::{MenuItem, OptionCard, Pace, Picker, Rounding, Saver, Stillness, Switch};

use super::appearance::corner;
use super::rows::{Announced as _, grid, segmented};
use super::{Group, LANGUAGES, SettingsView, Slot};
use crate::shared::nav::{entry_shown, set_entry_shown};
use crate::shared::popups::matches_query;

const STARTUP: &str = "startup";
const PROVIDER: &str = "provider";
const ENTRIES: &str = "entries";

impl SettingsView {
    pub(super) fn general_groups(&self, cx: &App) -> Vec<Group> {
        vec![
            Group::new("settings-group-startup", "icons/house.svg").rows([
                Slot::Startup,
                Slot::Provider,
                Slot::Entries,
                Slot::Language,
            ]),
            Group::new("settings-group-window", "icons/panel-left-open.svg")
                .rows([Slot::Tray, Slot::TrayIcon, Slot::NowPlayingOnFileOpen])
                .rows(self.decoration_slots(cx))
                .rows(taskbar_slots()),
            Group::new("settings-group-motion", "icons/loader-circle.svg").rows([
                Slot::Motion,
                Slot::Pace,
                Slot::Saver,
            ]),
            Group::new("settings-group-context-menus", "icons/list.svg").rows([Slot::AdaptiveMenu]),
        ]
    }

    /// The rows about the window's own frame, which differ by platform.
    #[allow(
        unused_variables,
        reason = "cx is only read on Linux and FreeBSD, where rounding follows the decorations"
    )]
    fn decoration_slots(&self, cx: &App) -> Vec<Slot> {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let mut slots = vec![Slot::ServerSideDecorations, Slot::Side, Slot::TrafficLights];
        // Server-side decorations put the compositor in charge of the frame, so window
        // rounding is only ever this app's call with client-side ones.
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if !self.settings.read(cx).server_side_decorations() {
            slots.push(Slot::WindowRounding);
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "macos")))]
        let slots = vec![
            Slot::Decorations,
            Slot::Side,
            Slot::TrafficLights,
            Slot::WindowRounding,
        ];
        #[cfg(target_os = "macos")]
        let slots = vec![Slot::Decorations, Slot::WindowRounding];
        slots
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

    pub(super) fn entries_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let picker = Picker::new(ENTRIES, &self.popovers, t!("settings-entries-pick"))
            .width(Picker::REGULAR)
            .sticky()
            .items(NavEntry::ALL.map(|entry| {
                let shown = entry_shown(entry, self.settings.read(cx));

                MenuItem::new(entry.id(), i18n::lookup(entry.key(), None))
                    .selected(shown)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            set_entry_shown(entry, !shown, settings, cx)
                        });
                        cx.notify();
                    }))
            }));

        self.row(
            t!("settings-entries"),
            picker.name(t!("settings-entries")).into_any_element(),
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

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    pub(super) fn server_side_decorations_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self.settings.read(cx).server_side_decorations();

        self.row(
            t!("settings-server-side-decorations"),
            Switch::new("server-side-decorations", enabled)
                .name(t!("settings-server-side-decorations"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let decorations = this.settings.update(cx, |settings, cx| {
                        settings.set_server_side_decorations(!enabled, cx);
                        settings.window_decorations()
                    });
                    window.request_decorations(decorations);
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    pub(super) fn decorations_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).window_controls();

        self.row(
            t!("settings-window-controls"),
            Switch::new("window-controls", on)
                .name(t!("settings-window-controls"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_window_controls(!on, cx));
                }))
                .into_any_element(),
        )
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn traffic_light_controls_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).traffic_light_controls();

        self.row(
            t!("settings-traffic-light-controls"),
            Switch::new("traffic-light-controls", on)
                .name(t!("settings-traffic-light-controls"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_traffic_light_controls(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    /// The window's corners as one card per rounding, each showing a corner at that radius.
    /// AppKit has no square window, so macOS offers only the corners it can draw.
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos"
    ))]
    pub(super) fn window_rounding_row(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current = self.settings.read(cx).window_rounding();
        let offered = Rounding::ALL
            .into_iter()
            .filter(|rounding| cfg!(not(target_os = "macos")) || *rounding != Rounding::Square)
            .collect::<Vec<_>>();

        let cards = grid(offered.len() as u16, window, cx).children(
            offered
                .iter()
                .copied()
                .enumerate()
                .map(|(place, rounding)| {
                    OptionCard::new(SharedString::from(format!("window-{}", rounding.id())))
                        .choice(rounding.label(), current == rounding, place, offered.len())
                        .thumb(corner(rounding, cx))
                        .label(rounding.label())
                        .selected(current == rounding)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.settings.update(cx, |settings, cx| {
                                settings.set_window_rounding(rounding, cx)
                            });
                            cx.notify();
                        }))
                }),
        );

        self.stacked(t!("settings-window-rounding"), cards)
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn side_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let left = settings.controls_on_left();
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let shown = !settings.server_side_decorations();
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let shown = settings.window_controls();

        let sides = segmented(
            "controls-side",
            [(true, t!("common-left")), (false, t!("common-right"))],
            left,
            cx.listener(move |this, left: &bool, _, cx| {
                if !shown {
                    return;
                }
                let left = *left;
                this.settings
                    .update(cx, |settings, cx| settings.set_controls_on_left(left, cx));
            }),
        )
        .when(!shown, |bar| bar.opacity(0.4));

        self.row(
            t!("settings-controls-side"),
            sides.name(t!("settings-controls-side")).into_any_element(),
        )
    }

    pub(super) fn motion_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.read(cx).stillness();

        let choices = segmented(
            "motion",
            Stillness::ALL.map(|choice| (choice, choice.label())),
            current,
            cx.listener(|this, choice: &Stillness, _, cx| {
                let choice = *choice;
                this.settings
                    .update(cx, |settings, cx| settings.set_stillness(choice, cx));
                cx.notify();
            }),
        );

        self.row(
            t!("settings-motion"),
            choices.name(t!("settings-motion")).into_any_element(),
        )
    }

    pub(super) fn pace_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.read(cx).pace();

        let choices = segmented(
            "pace",
            Pace::ALL.map(|choice| (choice, choice.label())),
            current,
            cx.listener(|this, choice: &Pace, _, cx| {
                let choice = *choice;
                this.settings
                    .update(cx, |settings, cx| settings.set_pace(choice, cx));
                cx.notify();
            }),
        );

        self.row(
            t!("settings-pace"),
            choices.name(t!("settings-pace")).into_any_element(),
        )
    }

    pub(super) fn saver_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings.read(cx).saver();

        let choices = segmented(
            "saver",
            Saver::ALL.map(|choice| (choice, choice.label())),
            current,
            cx.listener(|this, choice: &Saver, _, cx| {
                let choice = *choice;
                this.settings
                    .update(cx, |settings, cx| settings.set_saver(choice, cx));
                cx.notify();
            }),
        );

        super::Row::new(
            t!("settings-saver"),
            choices.name(t!("settings-saver")).into_any_element(),
        )
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
