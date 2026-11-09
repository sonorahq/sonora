//! The Spotify Connect group of the Playback section: whether Sonora shows up in Spotify's
//! device list, and under which name.

use i18n::t;
use rune::{Cx, El, TextEdit, Tx, row};
use state::ConnectName;
use ui::{Input, MenuItem, Picker};

use super::{Row, group, switch};
use crate::{Ui, act};

const CONNECT_NAME: &str = "spotify-connect-name";

/// The field for the custom name Sonora goes by in Spotify's device list.
#[derive(Default)]
pub struct State {
    pub edit: TextEdit,
    /// Whether the field has been filled from the saved name yet.
    seeded: bool,
}

/// The group, or `None` while the signed-in provider has no device list. The name rows only
/// stand while the device is on. The Playback page lays it out among its own groups.
pub(crate) fn view(ui: &Ui, _cx: &mut Cx<Ui>, layer: &mut Vec<El>) -> Option<El> {
    if !ui.store.devices.supported() {
        return None;
    }
    let theme = &ui.theme;
    let settings = &ui.store.settings;
    let mut rows = vec![switch(
        theme,
        "spotify-connect",
        t!("settings-spotify-connect"),
        settings.spotify_connect(),
        |ui, on| ui.store.settings.set_spotify_connect(on, &mut ui.store.cx),
    )];
    if settings.spotify_connect() {
        rows.push(name(ui, layer));
        if settings.spotify_connect_name() == ConnectName::Custom {
            rows.push(custom(ui));
        }
    }
    Some(group(
        theme,
        Some(("settings-group-spotify-connect", "icons/speaker.svg")),
        None,
        rows,
    ))
}

fn name(ui: &Ui, layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let chosen = ui.store.settings.spotify_connect_name();
    let picker = Picker::new(
        CONNECT_NAME,
        &ui.screens.settings.popovers,
        |ui: &mut Ui| &mut ui.screens.settings.popovers,
        i18n::lookup(chosen.key(), None),
    )
    .name(t!("settings-spotify-connect-name"))
    .width(Picker::<Ui>::NARROW)
    .items(ConnectName::ALL.map(|name| {
        MenuItem::new(name.id(), i18n::lookup(name.key(), None))
            .selected(name == chosen)
            .on_press(act(move |ui: &mut Ui| {
                ui.store
                    .settings
                    .set_spotify_connect_name(name, &mut ui.store.cx)
            }))
    }))
    .build(theme, layer);
    Row::new(
        "spotify-connect-name",
        t!("settings-spotify-connect-name"),
        picker,
    )
    .build(theme)
}

/// The custom name, saved as it is typed.
fn custom(ui: &Ui) -> El {
    let theme = &ui.theme;
    let field = Input::new(
        "settings-spotify-connect-custom-field",
        &ui.screens.settings.connect_name.edit,
        t!("settings-spotify-connect-custom-hint"),
        |ui: &mut Ui| &mut ui.screens.settings.connect_name.edit,
    )
    .name(t!("settings-spotify-connect-custom"))
    .compact()
    .on_change(|ui: &mut Ui, _: &mut Tx| {
        let typed = ui.screens.settings.connect_name.edit.text.clone();
        ui.store
            .settings
            .set_spotify_connect_custom_name(typed, &mut ui.store.cx);
        ui.settle();
    })
    .build(theme);
    Row::new(
        "spotify-connect-custom",
        t!("settings-spotify-connect-custom"),
        row()
            .key("spotify-connect-custom")
            .w(Picker::<Ui>::WIDE)
            .child(field),
    )
    .build(theme)
    .on_layout(|ui: &mut Ui, _: [f32; 4]| {
        let state = &mut ui.screens.settings.connect_name;
        if !state.seeded {
            state.edit = TextEdit::new(ui.store.settings.spotify_connect_custom_name());
            state.seeded = true;
        }
    })
}
