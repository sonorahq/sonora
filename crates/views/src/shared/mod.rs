pub(crate) mod about;
pub(crate) mod adaptive;
pub(crate) mod album_grid;
pub(crate) mod ambient;
pub(crate) mod animation;
pub(crate) mod blend;
pub(crate) mod cards;
pub(crate) mod cells;
pub(crate) mod confirm;
pub(crate) mod cover_viewer;
pub(crate) mod device_code;
pub(crate) mod device_menu;
pub(crate) mod equalizer;
pub(crate) mod hero;
pub(crate) mod local;
pub(crate) mod menus;
pub(crate) mod nav;
pub(crate) mod now_playing;
pub(crate) mod page;
pub(crate) mod part;
pub(crate) mod picks;
pub(crate) mod pins;
pub(crate) mod playlist_editor;
pub(crate) mod popups;
pub(crate) mod powerbar;
pub(crate) mod rooted;
pub(crate) mod shelves;
pub(crate) mod smart_playlist;
pub(crate) mod soloist;
pub(crate) mod steps;
pub(crate) mod tag_editor;
pub(crate) mod text;
pub(crate) mod theme_miniature;
pub(crate) mod track_card;
pub(crate) mod tracks;
pub(crate) mod transport;
pub(crate) mod trouble;
pub(crate) mod visualizer;
pub(crate) mod volume;
pub(crate) mod widevine;

pub(crate) use ui::effects;

/// The brand colours of the providers' marks.
const SPOTIFY: u32 = 0x1ed760;
const YOUTUBE: u32 = 0xff0033;
const DEEZER: u32 = 0xa238ff;
const APPLE: u32 = 0xfa2d48;
const SUBSONIC: u32 = 0xffa31a;
const JELLYFIN: u32 = 0x00a4dc;
const PLEX: u32 = 0xe5a00d;
/// The brand colours of the scrobbling services' marks. ListenBrainz's mark is navy and orange
/// and Libre.fm's orange with a red dot, so each takes the colour that holds on a dark page.
const LASTFM: u32 = 0xd51007;
const LIBREFM: u32 = 0xf57900;
const LISTENBRAINZ: u32 = 0xeb743b;
const MALOJA: u32 = 0xdec329;
const TEALFM: u32 = 0x14b8a6;
/// How much of the foreground text keeps where the theme would grey it over the ambient field.
/// The theme's grey is about that share of white over the page colour.
const QUIET: f32 = 0.56;

pub(crate) fn provider_logo(slug: &str) -> &'static str {
    match slug {
        "spotify" => "icons/spotify.svg",
        "youtube" => "icons/youtubemusic.svg",
        "subsonic" => "icons/subsonic.svg",
        "plex" => "icons/plex.svg",
        "deezer" => "icons/deezer.svg",
        "apple" => "icons/applemusic.svg",
        "tidal" => "icons/tidal.svg",
        "jellyfin" => "icons/jellyfin.svg",
        _ => "icons/music.svg",
    }
}

/// The mark of the scrobbling service `id`, as `assets/icons/common/LICENSE` credits them.
pub(crate) fn scrobbler_logo(id: &str) -> &'static str {
    match id {
        "lastfm" => "icons/lastfm.svg",
        "librefm" => "icons/librefm.svg",
        "listenbrainz" => "icons/listenbrainz.svg",
        "maloja" => "icons/maloja.svg",
        "tealfm" => "icons/tealfm.svg",
        _ => "icons/music.svg",
    }
}

/// The brand colour of the scrobbling service `id`'s mark, or the accent for a service
/// without one.
pub(crate) fn scrobbler_brand(id: &str, cx: &gpui::App) -> gpui::Hsla {
    match id {
        "lastfm" => gpui::rgb(LASTFM).into(),
        "librefm" => gpui::rgb(LIBREFM).into(),
        "listenbrainz" => gpui::rgb(LISTENBRAINZ).into(),
        "maloja" => gpui::rgb(MALOJA).into(),
        "tealfm" => gpui::rgb(TEALFM).into(),
        _ => ui::ActiveTheme::theme(cx).primary,
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
        "jellyfin" => gpui::rgb(JELLYFIN).into(),
        "plex" => gpui::rgb(PLEX).into(),
        _ => ui::ActiveTheme::theme(cx).foreground,
    }
}

/// The foreground at `QUIET`, in place of the theme's grey for text over the ambient field:
/// unsung lyrics, quieter lanes and the artist line. A flat grey stands apart from the colours
/// behind it, while translucent light takes them on.
pub(crate) fn quiet(theme: &ui::Theme) -> gpui::Hsla {
    theme.foreground.opacity(QUIET)
}
