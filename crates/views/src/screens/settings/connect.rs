use gpui::prelude::*;
use gpui::{AnyElement, App, Context};
use i18n::t;
use state::{ConnectName, Sonora};
use ui::{Field, Form, MenuItem, Picker, Switch};

use super::{Group, SettingsView, Slot};

const CONNECT_NAME: &str = "spotify-connect-name";

impl SettingsView {
    /// The device list group, left out while the signed-in provider has no device list: Spotify
    /// Connect, whose name rows only stand while the device is on, which it never is while
    /// Soloist plays, or Jellyfin's remote control, named by the sign-in.
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

    /// The switch that lists this app on the account's devices. Soloist is the account's
    /// device while it plays, so the switch is out of use then and shows off.
    pub(super) fn spotify_connect_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let unavailable = self.soloist_plays(cx);
        let on = self.settings.read(cx).spotify_connect();

        let label = match jellyfin(cx) {
            true => t!("settings-jellyfin-remote"),
            false => t!("settings-spotify-connect"),
        };
        self.soloist_row_for(
            label.clone(),
            Switch::new("spotify-connect", on && !unavailable)
                .name(label)
                .disabled(unavailable)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings
                        .update(cx, |settings, cx| settings.set_spotify_connect(!on, cx));
                }))
                .into_any_element(),
            unavailable,
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
