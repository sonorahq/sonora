//! The Discord section of Integrations: the rich presence status and what it says.

use i18n::t;
use rune::{Cx, El, row};
use state::DiscordName;
use ui::{Button, MenuItem, Picker};

use super::{Row, group, switch};
use crate::{Ui, act};

const DISCORD_NAME: &str = "discord-name";
/// The pill's padding against the text size, and its border, as a tab bar draws them.
const PAD: f32 = 0.25;
const BORDER: f32 = 1.;

/// The section. While the status is off only the master switch and the local files row stand,
/// since the rest shapes a status nobody sees.
pub(super) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> Vec<El> {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut groups = vec![group(
        theme,
        None,
        None,
        [switch(
            theme,
            "discord",
            t!("settings-discord"),
            settings.discord_presence(),
            |ui, on| ui.store.settings.set_discord_presence(on, &mut ui.store.cx),
        )],
    )];
    if settings.discord_presence() {
        // the choice only means something while Spotify is signed in and listed as a device
        let connect = settings.spotify_connect() && ui.store.devices.supported();
        let mut status = vec![
            name(ui, layer),
            switch(
                theme,
                "discord-show-paused",
                t!("settings-discord-show-paused"),
                settings.discord_show_paused(),
                |ui, on| {
                    ui.store
                        .settings
                        .set_discord_show_paused(on, &mut ui.store.cx)
                },
            ),
            switch(
                theme,
                "discord-badge",
                t!("settings-discord-badge"),
                settings.discord_badge(),
                |ui, on| ui.store.settings.set_discord_badge(on, &mut ui.store.cx),
            ),
            switch(
                theme,
                "discord-anonymous",
                t!("settings-discord-anonymous"),
                settings.discord_without_details(),
                |ui, on| {
                    ui.store
                        .settings
                        .set_discord_without_details(on, &mut ui.store.cx)
                },
            ),
        ];
        if connect {
            status.push(switch(
                theme,
                "discord-hide-on-connect",
                t!("settings-discord-hide-on-connect"),
                settings.discord_hide_on_connect(),
                |ui, on| {
                    ui.store
                        .settings
                        .set_discord_hide_on_connect(on, &mut ui.store.cx)
                },
            ));
        }
        groups.push(group(
            theme,
            Some(("settings-group-status", "icons/radio.svg")),
            None,
            status,
        ));
        groups.push(group(
            theme,
            Some(("settings-discord-buttons", "icons/link.svg")),
            None,
            [buttons(ui)],
        ));
    }
    groups.push(group(
        theme,
        Some(("settings-group-local-files", "icons/file-music.svg")),
        None,
        [switch(
            theme,
            "artwork-for-local-files",
            t!("settings-artwork-for-local-files"),
            settings.artwork_for_local_files(),
            |ui, on| {
                ui.store
                    .settings
                    .set_artwork_for_local_files(on, &mut ui.store.cx)
            },
        )],
    ));
    groups
}

fn name(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let chosen = ui.store.settings.discord_name();
    let picker = Picker::new(
        DISCORD_NAME,
        &ui.screens.settings.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        i18n::lookup(chosen.key(), None),
    )
    .name(t!("settings-discord-name"))
    .width(Picker::<Ui>::NARROW)
    .items(DiscordName::ALL.map(|name| {
        MenuItem::new(name.id(), i18n::lookup(name.key(), None))
            .selected(name == chosen)
            .on_press(act(move |ui: &mut Ui| {
                ui.store.settings.set_discord_name(name, &mut ui.store.cx)
            }))
    }))
    .build(theme, layer);
    Row::new("discord-name", t!("settings-discord-name"), picker).build(theme)
}

/// The buttons under the status, each a segment that turns on and off by itself.
fn buttons(ui: &Ui) -> El {
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let sonora = settings.discord_sonora_button();
    let provider = settings.discord_provider_button();
    let pad = theme.font_size * PAD;
    let inner = (theme.radius - pad).max(0.);
    let toggle = |key: &str, label: String, on: bool, set: fn(&mut Ui, bool)| {
        Button::new(key)
            .label(label)
            .small()
            .ghost()
            .selected(on)
            .toggled(on)
            .on_press(act(move |ui: &mut Ui| set(ui, !on)))
            .build(theme)
            .radius(inner)
    };
    // each segment turns on and off by itself, so the pill has no one lit fill to slide
    let bar = row()
        .key("discord-buttons")
        .role("group")
        .label(t!("settings-discord-buttons"))
        .center_y()
        .gap(pad)
        .padding(pad)
        .radius(theme.radius)
        .border(BORDER, theme.border.color())
        .fill(theme.secondary.color())
        .child(toggle(
            "discord-button-provider",
            t!("settings-discord-name-provider").into_owned(),
            provider,
            |ui, on| {
                ui.store
                    .settings
                    .set_discord_provider_button(on, &mut ui.store.cx)
            },
        ))
        .child(toggle(
            "discord-button-sonora",
            t!("settings-discord-name-sonora").into_owned(),
            sonora,
            |ui, on| {
                ui.store
                    .settings
                    .set_discord_sonora_button(on, &mut ui.store.cx)
            },
        ));
    Row::new("discord-buttons", t!("settings-discord-buttons"), bar).build(theme)
}
