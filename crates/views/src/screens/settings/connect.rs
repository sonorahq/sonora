use gpui::prelude::*;
use gpui::{AnyElement, App, Context, div};
use i18n::t;
use state::{ConnectName, Sonora};
use ui::{MenuItem, Picker, Switch};

use super::{Group, SettingsView, Slot};

const CONNECT_NAME: &str = "spotify-connect-name";

impl SettingsView {
    /// The Spotify Connect group, left out while the signed-in provider has no device list.
    /// The name rows only stand while the device is on.
    pub(super) fn connect_groups(&self, cx: &App) -> Vec<Group> {
        if !Sonora::global(cx).devices.read(cx).supported() {
            return Vec::new();
        }

        let settings = self.settings.read(cx);
        let mut slots = vec![Slot::SpotifyConnect];
        if settings.spotify_connect() {
            slots.push(Slot::SpotifyConnectName);
            if settings.spotify_connect_name() == ConnectName::Custom {
                slots.push(Slot::SpotifyConnectCustom);
            }
        }
        vec![Group::new("settings-group-spotify-connect", "icons/speaker.svg").rows(slots)]
    }

    pub(super) fn spotify_connect_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).spotify_connect();

        self.row(
            t!("settings-spotify-connect"),
            Switch::new("spotify-connect", on)
                .name(t!("settings-spotify-connect"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_spotify_connect(!on, cx));
                }))
                .into_any_element(),
        )
    }

    pub(super) fn spotify_connect_name_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let chosen = self.settings.read(cx).spotify_connect_name();

        let picker = Picker::new(
            CONNECT_NAME,
            &self.popovers,
            i18n::lookup(chosen.key(), None),
        )
        .width(Picker::NARROW)
        .items(ConnectName::ALL.map(|name| {
            MenuItem::new(name.id(), i18n::lookup(name.key(), None))
                .selected(name == chosen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings.update(cx, |settings, cx| {
                        settings.set_spotify_connect_name(name, cx)
                    });
                    cx.notify();
                }))
        }));

        self.row(
            t!("settings-spotify-connect-name"),
            picker
                .name(t!("settings-spotify-connect-name"))
                .into_any_element(),
        )
    }

    pub(super) fn spotify_connect_custom_row(&self) -> AnyElement {
        self.row(
            t!("settings-spotify-connect-custom"),
            div()
                .w(Picker::WIDE)
                .child(self.connect_name.clone())
                .into_any_element(),
        )
    }
}
