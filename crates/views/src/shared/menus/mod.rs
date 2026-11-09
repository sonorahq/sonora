//! Context menus: the menu of every song, album, artist, playlist and pin, drawn once over the
//! workspace, and the handlers a card or a row takes to open it.

mod card;
mod context;

pub(crate) use card::opener;
pub(crate) use context::*;
