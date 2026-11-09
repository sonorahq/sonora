//! Who is in the Sonora Session or Spotify Jam the listener is in, for the Sessions menu's
//! Members row. The panel that lists them lives in `chrome::aside::members`.

use music::connect::JamMember;

use crate::Ui;
use crate::chrome::session;

/// The people in the Sonora Session or Jam the listener is in, the host first, or none.
pub(crate) fn gathered(ui: &Ui) -> Vec<JamMember> {
    if let Some(room) = ui.store.listening.room() {
        return session::people(room);
    }
    let devices = &ui.store.devices;
    devices
        .jam()
        .filter(|_| devices.available())
        .map(|jam| jam.members.clone())
        .unwrap_or_default()
}
