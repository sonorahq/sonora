use gpui::prelude::*;
use gpui::{AnyElement, App, Context};
use i18n::t;
use state::{DiscordName, Sonora};
use ui::{Button, MenuItem, Picker, Switch, TabBar};

use super::{Group, SettingsView, Slot};

const DISCORD_NAME: &str = "discord-name";

impl SettingsView {
    /// The Discord page. While the status is off only the master switch and the local files
    /// row stand, since the rest shapes a status nobody sees.
    pub(super) fn discord_groups(&self, cx: &App) -> Vec<Group> {
        let settings = self.settings.read(cx);
        let mut groups = vec![Group::untitled().rows([Slot::Discord])];
        if settings.discord_presence() {
            // the choice only means something while Spotify is signed in and listed as a device
            let connect =
                settings.spotify_connect() && Sonora::global(cx).devices.read(cx).supported();
            groups.push(
                Group::new("settings-group-status", "icons/radio.svg")
                    .rows([
                        Slot::DiscordName,
                        Slot::DiscordShowPaused,
                        Slot::DiscordBadge,
                        Slot::DiscordAnonymous,
                    ])
                    .rows(connect.then_some(Slot::DiscordHideOnConnect)),
            );
            groups.push(
                Group::new("settings-discord-buttons", "icons/link.svg")
                    .rows([Slot::DiscordButtons]),
            );
        }
        groups.push(
            Group::new("settings-group-local-files", "icons/file-music.svg")
                .rows([Slot::ArtworkForLocal]),
        );
        groups
    }

    pub(super) fn discord_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).discord_presence();

        self.row(
            t!("settings-discord"),
            Switch::new("discord", on)
                .name(t!("settings-discord"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_presence(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn discord_name_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).discord_name();

        let picker = Picker::new(
            DISCORD_NAME,
            &self.popovers,
            i18n::lookup(chosen.key(), None),
        )
        .width(Picker::NARROW)
        .items(DiscordName::ALL.map(|name| {
            MenuItem::new(name.id(), i18n::lookup(name.key(), None))
                .selected(name == chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_name(name, cx));
                    cx.notify();
                }))
        }));

        self.row(
            t!("settings-discord-name"),
            picker.name(t!("settings-discord-name")).into_any_element(),
        )
    }

    pub(super) fn discord_show_paused_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).discord_show_paused();

        self.row(
            t!("settings-discord-show-paused"),
            Switch::new("discord-show-paused", on)
                .name(t!("settings-discord-show-paused"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_show_paused(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn discord_badge_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).discord_badge();

        self.row(
            t!("settings-discord-badge"),
            Switch::new("discord-badge", on)
                .name(t!("settings-discord-badge"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_discord_badge(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn artwork_for_local_files_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).artwork_for_local_files();

        self.row(
            t!("settings-artwork-for-local-files"),
            Switch::new("artwork-for-local-files", on)
                .name(t!("settings-artwork-for-local-files"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_artwork_for_local_files(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    pub(super) fn discord_anonymous_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).discord_without_details();

        self.row(
            t!("settings-discord-anonymous"),
            Switch::new("discord-anonymous", on)
                .name(t!("settings-discord-anonymous"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_discord_without_details(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    pub(super) fn discord_hide_on_connect_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).discord_hide_on_connect();

        self.row(
            t!("settings-discord-hide-on-connect"),
            Switch::new("discord-hide-on-connect", on)
                .name(t!("settings-discord-hide-on-connect"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_discord_hide_on_connect(!on, cx)
                    });
                }))
                .into_any_element(),
        )
    }

    /// The buttons under the status, each a segment that turns on and off by itself.
    pub(super) fn discord_buttons_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let sonora = settings.discord_sonora_button();
        let provider = settings.discord_provider_button();

        let buttons = TabBar::new("discord-buttons")
            .name(t!("settings-discord-buttons"))
            .flex_none()
            .items([
                Button::new("discord-button-provider")
                    .label(t!("settings-discord-name-provider"))
                    .small()
                    .ghost()
                    .selected(provider)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_discord_provider_button(!provider, cx)
                        });
                        cx.notify();
                    })),
                Button::new("discord-button-sonora")
                    .label(t!("settings-discord-name-sonora"))
                    .small()
                    .ghost()
                    .selected(sonora)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.update(cx, |settings, cx| {
                            settings.set_discord_sonora_button(!sonora, cx)
                        });
                        cx.notify();
                    })),
            ]);

        self.row(t!("settings-discord-buttons"), buttons.into_any_element())
    }
}
