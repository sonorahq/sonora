//! The song page: the hero with the cover, the artists, the release date and length and Play
//! and View album, then the About card's facts beside the credits and the details, the artist's
//! About card and the album's copyright line.

use std::rc::Rc;
use ui::Smooth as _;

use i18n::t;
use music::{Credit, Track};
use router::Destination;
use rune::{Cursor, Cx, Drag, El, PointerPress, ScrollSpec, Tx, col, row, text};
use state::SongDetail;
use ui::{Avatar, Button, Fact, InfoCard, Initials, Skeleton, Text, Weight, font};

use crate::Ui;
use crate::chrome::{player_bar, sidebar_left};
use crate::shared::about::{self, AboutArtist};
use crate::shared::cells;
use crate::shared::hero::{
    Hero, Listing, MetaStrip, PlayButton, copyright_notice, release_date_label,
};
use crate::shared::menus::{self, Target};
use crate::shared::page::rems;
use crate::shared::pins::Pinned as _;
use crate::shared::trouble;

/// The narrowest a panel of facts stands before the panels wrap under each other.
const PANEL: f32 = 300.;
/// How wide the skeleton bars of the loading page reach.
const TITLE_SKELETON: f32 = 240.;
const META_SKELETON: f32 = 180.;
const ACTION_SKELETON: f32 = 96.;
const FACT_SKELETON: f32 = 120.;
/// How many lines a loading panel shows.
const SKELETON_LINES: usize = 5;
/// The space between the page's blocks and between the panels, in rems.
const BLOCK_GAP: f32 = 1.25;
/// The space between the hero's buttons, between credits, and inside a credit, in rems.
const ACTION_GAP: f32 = 0.75;
const CREDIT_GAP: f32 = 0.75;
const CREDIT_LINE_GAP: f32 = 0.125;
/// The space between the genre chips and inside one, in rems.
const CHIP_GAP: f32 = 0.5;
const CHIP_X: f32 = 0.75;
const CHIP_Y: f32 = 0.25;
/// The hairline round a chip and a loading panel, and a chip's corners: fully round.
const BORDER: f32 = 1.;
const PILL: f32 = 999.;
/// The space between the lines of the details card, in rems.
const DETAIL_GAP: f32 = 0.25;

/// The song page's own state. Everything it shows lives in the store's song page, and its menu
/// and About dialog are the shared ones.
#[derive(Default)]
pub struct State {}

pub(crate) fn view(ui: &Ui, cx: &mut Cx<Ui>, _layer: &mut Vec<El>) -> El {
    let theme = &ui.theme;
    let song = ui.store.pages.song.as_ref();
    if let Some(failure) = song.and_then(|song| failure(ui, song)) {
        return col()
            .key("song-failure")
            .role("alert")
            .w_full()
            .h_full()
            .center()
            .child(failure);
    }
    let inset = theme.metrics.inset;
    let mut page = col()
        .key("song-page")
        .w_full()
        .h_full()
        .min_h(0.)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .smooth::<Ui>("song-page")
        .px(inset)
        .pt(inset)
        .pb(inset + player_bar::bottom(ui, cx));
    let Some(song) = song else {
        return page;
    };
    let Some(track) = song.track() else {
        if song.is_loading() {
            page = page.child(loading(ui));
        }
        return page;
    };
    let gap = rems(theme, BLOCK_GAP);
    let panels = row()
        .key("song-panels")
        .w_full()
        .flex_wrap()
        .items_stretch()
        .gap(gap)
        .child(
            col()
                .key("song-overview")
                .min_w(PANEL)
                .grow(1.)
                .child(overview(ui, song, track)),
        )
        .child(
            col()
                .key("song-side")
                .min_w(PANEL)
                .grow(1.)
                .gap(gap)
                .child(credits(ui, song, track))
                .child(discovery(ui, track)),
        );
    let profile = song.artist().map(|artist| {
        let name = artist.name.clone();
        let biography = artist.biography.clone();
        let artist_id = track
            .artist_refs
            .first()
            .and_then(|artist| artist.id.clone());
        row().key("song-profile").w_full().pt(gap).child(
            AboutArtist::new("song-artist-profile", artist.name.clone())
                .cover(artist.cover_large.clone())
                .biography(artist.biography.clone())
                .on_open(move |ui: &mut Ui| {
                    about::open(ui, name.clone(), biography.clone(), artist_id.clone())
                })
                .build(ui),
        )
    });
    let copyright = song
        .album()
        .and_then(|album| album.album.copyrights.first())
        .map(|copyright| {
            ui::spoken("song-copyright", copyright_notice(copyright), theme)
                .font(font(theme, Text::Tiny, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .wrap()
                .pt(gap)
                .min_w(0.)
        });
    page.child(hero(ui, song, track))
        .child(panels)
        .children(profile)
        .children(copyright)
}

/// The hero: the cover, the title with its explicit badge, the artists, the release date and the
/// length, and Play beside View album. A right-click opens the song's menu, and a drag carries
/// its pin to the sidebar.
fn hero(ui: &Ui, song: &SongDetail, track: &Track) -> El {
    let theme = &ui.theme;
    let album = song.album().map(|detail| &detail.album);
    let cover = album
        .and_then(|album| album.cover_large.clone())
        .or_else(|| track.cover.clone());
    let release = album
        .map(|album| release_date_label(&album.release_date))
        .filter(|release| !release.is_empty());

    let mut meta = MetaStrip::new().item(
        cells::artist_links(
            "song-artists",
            &track.artist_refs,
            track.artists.clone(),
            theme.muted_foreground,
        )
        .text_size(theme.text(Text::Small))
        .truncate()
        .build(theme),
    );
    if let Some(release) = release {
        meta = meta.text(release);
    }
    meta = meta.text(ui::clock(track.duration).into_owned());

    let view_album = track.album_id.clone().map(|album_id| {
        Button::new("open-album")
            .label(t!("song-view-album"))
            .outline()
            .on_press(move |ui: &mut Ui| ui.go(Destination::Album(album_id.as_str().into())))
            .build(theme)
    });
    let actions = row()
        .key("song-actions")
        .center_y()
        .gap(rems(theme, ACTION_GAP))
        .child(PlayButton::new("play-song", Listing::Owned(Rc::new(vec![track.clone()]))).build(ui))
        .children(view_album);

    let menu = Target::track(track.clone());
    let hero = Hero::new("song-hero", track.name.clone())
        .cover(cover.clone())
        .eyebrow(t!("song-eyebrow").into_owned())
        .meta(meta.build(theme))
        .actions(actions)
        .explicit(track.explicit)
        .on_context(move |ui: &mut Ui, press: PointerPress, _: &mut Tx| {
            menus::open(ui, menu.clone(), press)
        })
        .build(ui);
    match track.pin() {
        Some(pin) => {
            let pin = pin.cover(cover);
            hero.on_drag(move |ui: &mut Ui, drag: &mut Drag, tx: &mut Tx| {
                sidebar_left::carry(ui, &pin, drag, tx)
            })
        }
        None => hero,
    }
}

/// The About card: the album, the release date, the streams, the place on the album, the label
/// and the popularity, striped every other row.
fn overview(ui: &Ui, song: &SongDetail, track: &Track) -> El {
    let theme = &ui.theme;
    let album = song.album().map(|detail| &detail.album);
    let album_name = album
        .map(|album| album.name.clone())
        .unwrap_or_else(|| track.album.clone());
    let release = album
        .map(|album| album.release_date.clone())
        .unwrap_or_default();
    let release = match release.is_empty() {
        true => t!("common-unknown").into_owned(),
        false => release_date_label(&release),
    };
    let label = album
        .map(|album| album.label.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| t!("common-not-provided").into_owned());
    let number = match (track.disc_number, track.track_number) {
        (disc, number) if disc > 1 => t!("song-disc-track", disc = disc, track = number),
        (_, number) if number > 0 => t!("song-track", track = number),
        _ => t!("common-not-provided"),
    }
    .into_owned();
    let streams = song
        .playcount()
        .map(cells::count)
        .unwrap_or_else(|| t!("common-not-available").into_owned());
    let facts = [
        ("song-fact-album", t!("song-album").into_owned(), album_name),
        (
            "song-fact-released",
            t!("song-released").into_owned(),
            release,
        ),
        (
            "song-fact-streams",
            t!("song-streams").into_owned(),
            streams,
        ),
        (
            "song-fact-position",
            t!("song-position").into_owned(),
            number,
        ),
        ("song-fact-label", t!("song-label").into_owned(), label),
        (
            "song-fact-popularity",
            t!("song-popularity").into_owned(),
            t!("song-popularity-value", value = track.popularity).into_owned(),
        ),
    ];
    InfoCard::new(t!("song-about"))
        .spoken("song-about-card")
        .stretch()
        .child(
            col().children(
                facts
                    .into_iter()
                    .enumerate()
                    .map(|(index, (key, name, value))| {
                        Fact::new(name, value)
                            .spoken(key)
                            .striped(index % 2 == 1)
                            .build(theme)
                    }),
            ),
        )
        .build(theme)
}

/// The credits card: each contributor with their portrait or initials and their role, a link to
/// their page when the provider gives one. A song without credits lists its artists as
/// performers.
fn credits(ui: &Ui, song: &SongDetail, track: &Track) -> El {
    let theme = &ui.theme;
    let rows: Vec<Credit> = match track.credits.is_empty() {
        true => track
            .artist_refs
            .iter()
            .map(|artist| Credit {
                name: artist.name.clone(),
                role: t!("song-performed-by").into_owned(),
                id: artist.id.clone(),
            })
            .collect(),
        false => track.credits.clone(),
    };
    let portraits = song.portraits();
    let total = rows.len();
    let lines = rows.into_iter().enumerate().map(|(index, credit)| {
        let portrait = credit
            .id
            .as_ref()
            .and_then(|id| portraits.get(id))
            .map(String::as_str);
        let avatar = match portrait {
            Some(portrait) => Avatar::new(ui.artwork(), Some(portrait))
                .size(theme.metrics.thumb)
                .build(theme)
                .a11y_hidden(),
            None => Initials::new(credit.name.clone(), theme.metrics.thumb).build(theme),
        };
        // A linked row speaks as one link, so its lines stay anonymous, while an unlinked row has
        // no node of its own and exposes each line instead.
        let linked = credit.id.is_some();
        let line = |key: String, value: String, el: El| match linked {
            true => el.a11y_hidden(),
            false => el.key(key).role("text").label(value),
        };
        let name = line(
            format!("song-credit-name-{index}"),
            credit.name.clone(),
            text(credit.name.clone())
                .font(font(theme, Text::Body, Weight::MEDIUM))
                .color(theme.foreground.color())
                .truncate()
                .min_w(0.),
        );
        let role = line(
            format!("song-credit-role-{index}"),
            credit.role.clone(),
            text(credit.role.clone())
                .font(font(theme, Text::Small, Weight::NORMAL))
                .color(theme.muted_foreground.color())
                .min_w(0.),
        );
        let entry = row()
            .key(format!("song-credit-{index}"))
            .center_y()
            .min_w(0.)
            .gap(rems(theme, CREDIT_GAP))
            .px(theme.metrics.pad)
            .py(theme.metrics.pad / 2.)
            .radius(theme.radius)
            .child(avatar)
            .child(
                col()
                    .grow(1.)
                    .shrink()
                    .min_w(0.)
                    .gap(rems(theme, CREDIT_LINE_GAP))
                    .child(name)
                    .child(role),
            );
        match credit.id {
            Some(id) => entry
                .role("link")
                .label(credit.name)
                .description(credit.role)
                .position_in_set(index + 1, total)
                .focusable(true)
                .cursor(Cursor::Pointer)
                .hover(theme.secondary_hover.color())
                .press_scale(ui::PRESS_SCALE)
                .on_press(move |ui: &mut Ui| ui.go(Destination::Artist(id.as_str().into()))),
            None => entry,
        }
    });
    InfoCard::new(t!("song-credits"))
        .spoken("song-credits-card")
        .child(col().gap(rems(theme, CREDIT_GAP)).children(lines))
        .build(theme)
}

/// The details card: the genres as chips, the languages and whether the song is explicit.
fn discovery(ui: &Ui, track: &Track) -> El {
    let theme = &ui.theme;
    let languages = match track.languages.is_empty() {
        true => t!("common-not-provided").into_owned(),
        false => track
            .languages
            .iter()
            .map(|code| language_label(code))
            .collect::<Vec<_>>()
            .join(", "),
    };
    let genres = match track.tags.is_empty() {
        true => Fact::new(t!("song-genres"), t!("common-not-available"))
            .spoken("song-fact-genres")
            .build(theme),
        false => {
            let chips = track.tags.iter().enumerate().map(|(index, tag)| {
                ui::spoken(format!("song-genre-{index}"), tag.clone(), theme)
                    .font(font(theme, Text::Small, Weight::NORMAL))
                    .truncate()
                    .min_w(0.)
                    .px(rems(theme, CHIP_X))
                    .py(rems(theme, CHIP_Y))
                    .radius(PILL)
                    .fill(theme.secondary.color())
                    .border(BORDER, theme.border.color())
            });
            row()
                .key("song-genres-row")
                .items_start()
                .justify_between()
                .gap(rems(theme, 1.))
                .min_w(0.)
                .px(theme.metrics.pad)
                .py(theme.metrics.pad / 2.)
                .child(
                    ui::spoken("song-genres", t!("song-genres"), theme)
                        .color(theme.muted_foreground.color())
                        .grow(1.)
                        .shrink()
                        .min_w(0.),
                )
                .child(
                    row()
                        .grow(1.)
                        .shrink()
                        .min_w(0.)
                        .flex_wrap()
                        .justify_end()
                        .gap(rems(theme, CHIP_GAP))
                        .children(chips),
                )
        }
    };
    let content = match track.explicit {
        true => t!("song-explicit"),
        false => t!("song-clean"),
    };
    InfoCard::new(t!("song-details"))
        .spoken("song-details-card")
        .child(
            col()
                .gap(rems(theme, DETAIL_GAP))
                .child(genres)
                .child(
                    Fact::new(t!("song-language"), languages)
                        .spoken("song-fact-language")
                        .striped(true)
                        .build(theme),
                )
                .child(
                    Fact::new(t!("song-content"), content)
                        .spoken("song-fact-content")
                        .build(theme),
                ),
        )
        .build(theme)
}

/// The page while the song is on its way: a skeleton hero over two skeleton panels.
fn loading(ui: &Ui) -> El {
    let theme = &ui.theme;
    let pad = theme.metrics.pad;
    let bar = |width: f32, height: f32| Skeleton::new().build(theme).size(width, height);
    let panel = |key: &str| {
        col()
            .key(key.to_owned())
            .min_w(PANEL)
            .grow(1.)
            .gap(rems(theme, 0.75))
            .padding(pad)
            .radius(theme.radius)
            .border(BORDER, theme.border.color())
            .child(bar(FACT_SKELETON, pad))
            .children((0..SKELETON_LINES).map(|_| Skeleton::new().build(theme).w_full().h(pad)))
    };
    let gap = rems(theme, BLOCK_GAP);
    col()
        .key("song-loading")
        .w_full()
        .gap(gap)
        .child(
            row()
                .center_y()
                .gap(gap)
                .child(bar(theme.metrics.cover, theme.metrics.cover))
                .child(
                    col()
                        .grow(1.)
                        .min_w(0.)
                        .gap(rems(theme, 0.75))
                        .child(bar(FACT_SKELETON, pad))
                        .child(bar(TITLE_SKELETON, theme.metrics.control))
                        .child(bar(META_SKELETON, pad))
                        .child(bar(ACTION_SKELETON, theme.metrics.control)),
                ),
        )
        .child(
            row()
                .w_full()
                .flex_wrap()
                .items_stretch()
                .gap(gap)
                .child(panel("song-loading-overview"))
                .child(panel("song-loading-side")),
        )
        .a11y_hidden()
}

/// The page a song shows instead of its hero when it cannot be read: the No connection state as
/// soon as the network is gone, and the failure of its own load otherwise. Opening the same song
/// again is the retry.
fn failure(ui: &Ui, song: &SongDetail) -> Option<El> {
    let id = song.id()?.to_owned();
    let reason = match trouble::unreachable(ui, &id) {
        true => None,
        false => Some(song.error()?.to_owned()),
    };
    Some(trouble::lost(
        ui,
        "song-lost",
        t!("trouble-not-loaded"),
        reason.as_deref(),
        move |ui: &mut Ui| ui.store.open_song(&id),
    ))
}

/// A language's name from its ISO code, or the code itself for one without a name of its own.
fn language_label(code: &str) -> String {
    match code {
        "ar" => t!("language-ar"),
        "de" => t!("language-de"),
        "en" => t!("language-en"),
        "es" => t!("language-es"),
        "fr" => t!("language-fr"),
        "hi" => t!("language-hi"),
        "it" => t!("language-it"),
        "ja" => t!("language-ja"),
        "ko" => t!("language-ko"),
        "pt" => t!("language-pt"),
        "ru" => t!("language-ru"),
        "tr" => t!("language-tr"),
        "uk" => t!("language-uk"),
        "zh" => t!("language-zh"),
        "zxx" => t!("language-zxx"),
        _ => return code.to_owned(),
    }
    .into_owned()
}
