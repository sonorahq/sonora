use gpui::prelude::*;
use gpui::{AnyElement, App, Context, Window};
use i18n::t;
use state::Sonora;
use ui::{ActiveTheme as _, Switch};

use super::rows::{chip, chips};
use super::{Group, SettingsView, Slot};

impl SettingsView {
    /// The Discord page: the status drawn as Discord shows it, changed by clicking its parts,
    /// then the choices the card cannot show as chips under it. The switch that turns the
    /// status on stands at the page's title. While the status is off the card stays, dimmed,
    /// and the chips leave, since they shape a status nobody sees.
    pub(super) fn discord_groups(&self, cx: &App) -> Vec<Group> {
        let on = self.settings.read(cx).discord_presence();
        vec![
            Group::untitled()
                .rows([Slot::Discord])
                .rows(on.then_some(Slot::DiscordOptions)),
        ]
    }

    /// The switch at the Discord page's title that puts the playing track on the profile.
    pub(super) fn discord_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).discord_presence();

        Switch::new("discord", on)
            .name(t!("settings-discord"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings
                    .update(cx, |settings, cx| settings.set_discord_presence(!on, cx));
            }))
            .into_any_element()
    }

    /// The status card, whose name, badge and buttons change where they stand.
    pub(super) fn discord_card(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        super::preview::discord::preview(&self.previews.discord, self, window, cx)
    }

    /// The choices about the status that the card cannot show, one chip each: whether it stays
    /// while paused, whether it hides the track, where its art comes from, and whether it steps
    /// aside for Spotify's own while Sonora plays as a Spotify device.
    pub(super) fn discord_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let settings = self.settings.read(cx);
        let paused = settings.discord_show_paused();
        let hidden = settings.discord_without_details();
        let upload = settings.discord_upload_covers();
        let local = settings.artwork_for_local_files();
        let quiet = settings.discord_hide_on_connect();
        // the choice only means something while Spotify is signed in and listed as a device
        let connect = settings.spotify_connect()
            && Sonora::global(cx).devices.read(cx).supported()
            && !super::connect::jellyfin(cx)
            && !super::connect::youtube(cx);

        let option =
            |id: &'static str, icon: &'static str, key: &'static str, on: bool, cx: &App| {
                chip(
                    id,
                    icon,
                    i18n::lookup(key, None),
                    i18n::lookup(key, None),
                    on,
                    cx,
                )
                .tooltip(format!("{key}-detail"))
            };
        let shown = [
            option(
                "discord-show-paused",
                "icons/pause.svg",
                "settings-discord-show-paused",
                paused,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_discord_show_paused(!paused, cx)
                });
            })),
            option(
                "discord-anonymous",
                "icons/lock.svg",
                "settings-discord-anonymous",
                hidden,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_discord_without_details(!hidden, cx)
                });
            })),
            option(
                "discord-upload-covers",
                "icons/image.svg",
                "settings-discord-upload-covers",
                upload,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_discord_upload_covers(!upload, cx)
                });
            })),
            option(
                "artwork-for-local-files",
                "icons/file-music.svg",
                "settings-artwork-for-local-files",
                local,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_artwork_for_local_files(!local, cx)
                });
            })),
        ];
        let quiet = connect.then(|| {
            option(
                "discord-hide-on-connect",
                "icons/speaker.svg",
                "settings-discord-hide-on-connect",
                quiet,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings.update(cx, |settings, cx| {
                    settings.set_discord_hide_on_connect(!quiet, cx)
                });
            }))
        });

        chips(shown.into_iter().chain(quiet))
            .flex_wrap()
            .pt(cx.theme().metrics.pad)
            .into_any_element()
    }
}
