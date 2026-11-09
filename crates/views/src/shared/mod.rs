//! Pieces several screens share.
pub(crate) mod theme_miniature;
pub(crate) mod device_code;
pub(crate) mod popups;
pub(crate) mod steps;
pub(crate) mod about;
pub(crate) mod adaptive;
pub(crate) mod confirm;
pub(crate) mod device_menu;
pub(crate) mod dialogs;
pub(crate) mod equalizer;
pub(crate) mod menus;
pub(crate) mod pins;
pub(crate) mod playlist_editor;
pub(crate) mod tag_editor;
pub(crate) mod trouble;
pub(crate) mod widevine;
// The building blocks below wait for the wave-2 screens; drop each allow once a screen uses it.
#[allow(dead_code, unused_imports)]
pub(crate) mod album_grid;
#[allow(dead_code, unused_imports)]
pub(crate) mod cards;
#[allow(dead_code, unused_imports)]
pub(crate) mod cells;
#[allow(dead_code, unused_imports)]
pub(crate) mod hero;
#[allow(dead_code, unused_imports)]
pub(crate) mod local;
#[allow(dead_code, unused_imports)]
pub(crate) mod nav;
#[allow(dead_code, unused_imports)]
pub(crate) mod page;
#[allow(dead_code, unused_imports)]
pub(crate) mod picks;
#[allow(dead_code, unused_imports)]
pub(crate) mod shelves;
#[allow(dead_code, unused_imports)]
pub(crate) mod text;
#[allow(dead_code, unused_imports)]
pub(crate) mod track_card;
#[allow(dead_code, unused_imports)]
pub(crate) mod tracks;
pub use dialogs::Dialogs;
