use gpui::prelude::*;
use gpui::{AnyElement, App, Context};
use i18n::t;
use state::{ConnectName, Sonora};
use ui::{Field, Form, MenuItem, Picker, Switch};

use super::{Group, SettingsView, Slot};

const CONNECT_NAME: &str = "spotify-connect-name";

impl SettingsView {
    /// The provider's device controls, with name settings for the built-in Spotify player.
    pub(super) fn connect_groups(&self, cx: &App) -> Vec<Group> {
        if !Sonora::global(cx).devices.read(cx).supported() {
            return Vec::new();
        }
        if jellyfin(cx) {
            return vec![
                Group::new("settings-group-jellyfin-remote", "icons/speaker.svg")
                    .rows([Slot::SpotifyConnect]),
            ];
        }

        let settings = self.settings.read(cx);
        let mut slots = vec![Slot::SpotifyConnect];
        if settings.spotify_connect() && !self.soloist_plays(cx) {
            slots.push(Slot::SpotifyConnectName);
            if settings.spotify_connect_name() == ConnectName::Custom {
                slots.push(Slot::SpotifyConnectCustom);
            }
        }
        vec![Group::new("settings-group-spotify-connect", "icons/speaker.svg").rows(slots)]
    }

    /// Enables account discovery and remote playback controls.
    pub(super) fn spotify_connect_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let on = self.settings.read(cx).spotify_connect();
        let label = match jellyfin(cx) {
            true => t!("settings-jellyfin-remote"),
            false => t!("settings-spotify-connect"),
        };
        self.row(
            label.clone(),
            Switch::new("spotify-connect", on)
                .name(label)
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

    pub(super) fn spotify_connect_custom_row(&self, cx: &App) -> AnyElement {
        self.row(
            t!("settings-spotify-connect-custom"),
            Form::new("settings-spotify-connect-form")
                .w(Picker::WIDE)
                .field(Field::new(&self.connect_name, cx))
                .into_any_element(),
        )
    }
}

/// Whether the signed-in provider is Jellyfin, whose device list is its remote control and
/// takes its device name from the sign-in.
pub(super) fn jellyfin(cx: &App) -> bool {
    Sonora::global(cx).session.read(cx).provider_slug() == Some("jellyfin")
}
