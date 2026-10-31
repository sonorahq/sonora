pub(crate) mod about;
pub(crate) mod adaptive;
pub(crate) mod album_grid;
pub(crate) mod ambient;
pub(crate) mod cards;
pub(crate) mod cells;
pub(crate) mod confirm;
pub(crate) mod device_code;
pub(crate) mod equalizer;
pub(crate) mod hero;
pub(crate) mod local;
pub(crate) mod menus;
pub(crate) mod nav;
pub(crate) mod page;
pub(crate) mod picks;
pub(crate) mod pins;
pub(crate) mod playlist_editor;
pub(crate) mod popups;
pub(crate) mod shelves;
pub(crate) mod steps;
pub(crate) mod tag_editor;
pub(crate) mod text;
pub(crate) mod theme_miniature;
pub(crate) mod track_card;
pub(crate) mod tracks;
pub(crate) mod transport;
pub(crate) mod trouble;
pub(crate) mod visualizer;
pub(crate) mod widevine;

pub(crate) use ui::effects;

/// The brand colours of the providers' marks.
const SPOTIFY: u32 = 0x1ed760;
const YOUTUBE: u32 = 0xff0033;
const DEEZER: u32 = 0xa238ff;
const APPLE: u32 = 0xfa2d48;
const SUBSONIC: u32 = 0xffa31a;

pub(crate) fn provider_logo(slug: &str) -> &'static str {
    match slug {
        "spotify" => "icons/spotify.svg",
        "youtube" => "icons/youtubemusic.svg",
        "subsonic" => "icons/subsonic.svg",
        "deezer" => "icons/deezer.svg",
        "apple" => "icons/applemusic.svg",
        "tidal" => "icons/tidal.svg",
        _ => "icons/music.svg",
    }
}

/// The brand colour of the provider `slug`'s mark. Tidal's mark is plain black or white, so it
/// takes the foreground, as does any provider without a colour of its own.
pub(crate) fn provider_brand(slug: &str, cx: &gpui::App) -> gpui::Hsla {
    match slug {
        "spotify" => gpui::rgb(SPOTIFY).into(),
        "youtube" => gpui::rgb(YOUTUBE).into(),
        "deezer" => gpui::rgb(DEEZER).into(),
        "apple" => gpui::rgb(APPLE).into(),
        "subsonic" => gpui::rgb(SUBSONIC).into(),
        _ => ui::ActiveTheme::theme(cx).foreground,
    }
}
