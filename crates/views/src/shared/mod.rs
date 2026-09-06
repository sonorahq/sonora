pub(crate) mod about;
pub(crate) mod adaptive;
pub(crate) mod album_grid;
pub(crate) mod cards;
pub(crate) mod cells;
pub(crate) mod confirm;
pub(crate) mod hero;
pub(crate) mod local;
pub(crate) mod menus;
pub(crate) mod page;
pub(crate) mod picks;
pub(crate) mod pins;
pub(crate) mod playlist_editor;
pub(crate) mod popups;
pub(crate) mod shelves;
pub(crate) mod steps;
pub(crate) mod tag_editor;
pub(crate) mod text;
pub(crate) mod track_card;
pub(crate) mod tracks;
pub(crate) mod transport;
pub(crate) mod trouble;
pub(crate) mod visualizer;

pub(crate) fn effects() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("SONORA_BLUR").as_deref() != Ok("0"))
}

pub(crate) fn provider_logo(slug: &str) -> &'static str {
    match slug {
        "spotify" => "icons/spotify.svg",
        "youtube" => "icons/youtubemusic.svg",
        "deezer" => "icons/deezer.svg",
        _ => "icons/music.svg",
    }
}
