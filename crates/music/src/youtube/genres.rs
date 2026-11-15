use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use ytmusic::nav::Nav as _;
use ytmusic::{Client, YtMusic, parse};

use crate::youtube::client::library_playlist;
use crate::youtube::wire;
use crate::{Feed, Genre, GenreDetail, GenreItem, GenreSection, HomeFeed, SavedArtist, Track};

const HOME: &str = "FEmusic_home";
const LISTEN_AGAIN: &str = "Listen again";
const QUICK_PICKS: &str = "Quick picks";
const FROM_LIBRARY: &str = "From your library";
/// The shelves that open the page after Listen again, in this order; every other shelf keeps
/// YouTube's order behind.
const LEADING: [&str; 2] = [FROM_LIBRARY, "Recaps"];
/// How many of the library's playlists stand in for `FROM_LIBRARY` until YouTube's own comes.
const STAND_IN: usize = 10;
/// A shelf of music videos: Sonora plays audio only, so it is left out.
const VIDEOS: &str = "Music videos for you";
/// How many lots of shelves the home feed is followed for; the web client stops around here.
const PAGES: usize = 8;
const CATEGORIES: &str = "FEmusic_moods_and_genres";
const CATEGORY: &str = "FEmusic_moods_and_genres_category";
const THUMB: u32 = 120;
const ARTIST_PAGE: &str = "MUSIC_PAGE_TYPE_ARTIST";
const SEPARATOR: &str = " • ";

/// The home feed. YouTube hands it out a few shelves at a time behind a continuation, and
/// the web client asks for the next lot as the page scrolls, so this follows the chain until
/// it ends or `PAGES` is reached. Listen again and Quick picks are lifted out wherever they
/// turn up, every other shelf keeps its place. A lot that fails ends the feed with what came
/// before it.
///
/// The page opens with four things: Listen again, "From your library", Recaps and Quick
/// picks. Those are sent as soon as each lot brings one, with nothing else beside them, and
/// only once all four are in does the rest come, in one message at the end, so the page
/// settles quickly and then fills below the fold rather than reshuffling lot by lot.
///
/// "From your library" is among the last shelves YouTube sends, yet it opens the page, so
/// the first lot carries a stand-in built from the library's own playlists, fetched beside
/// the first page; YouTube's shelf takes its place when it comes.
pub(crate) fn home(api: Arc<YtMusic>, account: String) -> Feed {
    let (sink, feed) = tokio::sync::mpsc::channel(PAGES);
    tokio::spawn(async move {
        let (first, playlists) = tokio::join!(
            api.execute("browse", Client::Music, json!({ "browseId": HOME })),
            api.library_playlists(),
        );
        let mut answer = match first {
            Ok(answer) => answer,
            Err(error) => {
                sink.send(Err(error)).await.ok();
                return;
            }
        };
        let mut whole = HomeFeed::default();
        match playlists {
            Ok(playlists) => whole.sections.extend(stand_in(playlists, &account)),
            Err(error) => log::warn!("youtube: cannot list the library playlists: {error:#}"),
        }
        gather(&answer, &mut whole);
        let mut opening = !opened(&whole);
        if sink.send(Ok(arranged(&whole, opening))).await.is_err() {
            return;
        }
        for page in 2..=PAGES {
            let Some(token) = continuation(&answer).map(str::to_owned) else {
                break;
            };
            answer = match api
                .execute("browse", Client::Music, json!({ "continuation": token }))
                .await
            {
                Ok(next) => next,
                Err(error) => {
                    log::warn!("youtube: cannot load home page {page}: {error:#}");
                    break;
                }
            };
            gather(&answer, &mut whole);
            if opening && sink.send(Ok(arranged(&whole, true))).await.is_err() {
                return;
            }
            opening = opening && !opened(&whole);
        }
        sink.send(Ok(arranged(&whole, false))).await.ok();
    });

    feed
}

/// Whether the four the page opens with are all in.
fn opened(feed: &HomeFeed) -> bool {
    !feed.listen_again.is_empty()
        && feed.quick_picks.is_some()
        && LEADING
            .iter()
            .all(|title| feed.sections.iter().any(|section| section.title == *title))
}

fn gather(answer: &Value, feed: &mut HomeFeed) {
    for shelf in parse::find_renderers(answer, "musicCarouselShelfRenderer") {
        match shelf_title(shelf).as_deref() {
            Some(LISTEN_AGAIN) => feed.listen_again = items(shelf),
            Some(QUICK_PICKS) => feed.quick_picks = Some(tracks(shelf)),
            Some(VIDEOS) => {}
            _ => {
                if let Some(section) = section(shelf) {
                    place(feed, section);
                }
            }
        }
    }
}

/// Adds a shelf to the feed, in place of one already there under the same title, so a
/// stand-in gives way to the shelf it stood in for.
fn place(feed: &mut HomeFeed, section: GenreSection) {
    match feed
        .sections
        .iter()
        .position(|there| there.title == section.title)
    {
        Some(at) => feed.sections[at] = section,
        None => feed.sections.push(section),
    }
}

/// The library's first playlists as the `FROM_LIBRARY` shelf, the way `playlists` lists them:
/// YouTube's own shelf is those and the saved albums, so this is close until it arrives.
fn stand_in(playlists: Vec<ytmusic::Playlist>, account: &str) -> Option<GenreSection> {
    let items: Vec<GenreItem> = playlists
        .into_iter()
        .filter_map(|playlist| library_playlist(playlist, account))
        .take(STAND_IN)
        .map(GenreItem::Playlist)
        .collect();

    (!items.is_empty()).then(|| GenreSection {
        title: FROM_LIBRARY.to_owned(),
        items,
    })
}

/// The feed with the `LEADING` shelves first, in their order; while `opening`, those are the
/// only shelves in it.
fn arranged(feed: &HomeFeed, opening: bool) -> HomeFeed {
    let mut sections = feed.sections.clone();
    let mut arranged = Vec::with_capacity(sections.len());
    for title in LEADING {
        if let Some(at) = sections.iter().position(|section| section.title == title) {
            arranged.push(sections.remove(at));
        }
    }
    if !opening {
        arranged.append(&mut sections);
    }

    HomeFeed {
        listen_again: feed.listen_again.clone(),
        quick_picks: feed.quick_picks.clone(),
        sections: arranged,
    }
}

pub(crate) async fn genres(api: &YtMusic) -> Result<Vec<Genre>> {
    let answer = api
        .execute("browse", Client::Music, json!({ "browseId": CATEGORIES }))
        .await?;

    Ok(parse::find_renderers(&answer, "gridRenderer")
        .into_iter()
        .flat_map(|grid| grid.items(&["items"]))
        .filter_map(card)
        .collect())
}

pub(crate) async fn genre(api: &YtMusic, params: &str) -> Result<GenreDetail> {
    let answer = api
        .execute(
            "browse",
            Client::Music,
            json!({ "browseId": CATEGORY, "params": params }),
        )
        .await?;

    Ok(GenreDetail {
        name: answer
            .run_text(&["header", "musicHeaderRenderer", "title"])
            .unwrap_or_default(),
        sections: parse::find_renderers(&answer, "musicCarouselShelfRenderer")
            .into_iter()
            .filter_map(section)
            .collect(),
    })
}

fn section(shelf: &Value) -> Option<GenreSection> {
    let title = shelf_title(shelf).unwrap_or_default();
    let items = items(shelf);

    (!items.is_empty()).then_some(GenreSection { title, items })
}

/// The songs and videos of a shelf, for a row that plays rather than browses.
fn tracks(shelf: &Value) -> Vec<Track> {
    items(shelf)
        .into_iter()
        .filter_map(|item| match item {
            GenreItem::Track(track) => Some(track),
            _ => None,
        })
        .collect()
}

/// Every card of a shelf in the order YouTube lists them, whatever each one is.
fn items(shelf: &Value) -> Vec<GenreItem> {
    shelf
        .items(&["contents"])
        .iter()
        .enumerate()
        .filter_map(|(index, node)| item(node, index))
        .collect()
}

fn shelf_title(shelf: &Value) -> Option<String> {
    shelf.run_text(&["header", "musicCarouselShelfBasicHeaderRenderer", "title"])
}

/// What kind of page a browse card leads to.
fn page_type(renderer: &Value) -> Option<&str> {
    renderer.str_at(&[
        "navigationEndpoint",
        "browseEndpoint",
        "browseEndpointContextSupportedConfigs",
        "browseEndpointContextMusicConfig",
        "pageType",
    ])
}

fn track(item: &Value) -> Option<ytmusic::Track> {
    let renderer = item.at(&["musicTwoRowItemRenderer"])?;
    if matches!(
        page_type(renderer),
        Some("MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_PLAYLIST")
    ) {
        return None;
    }

    let endpoint = renderer.at(&[
        "thumbnailOverlay",
        "musicItemThumbnailOverlayRenderer",
        "content",
        "musicPlayButtonRenderer",
        "playNavigationEndpoint",
        "watchEndpoint",
    ])?;
    let video_id = endpoint.str_at(&["videoId"])?.to_owned();
    let artists = card_artists(renderer.runs(&["subtitle"]));
    let kind = match endpoint.str_at(&[
        "watchEndpointMusicSupportedConfigs",
        "watchEndpointMusicConfig",
        "musicVideoType",
    ]) {
        Some("MUSIC_VIDEO_TYPE_ATV") => ytmusic::TrackKind::Song,
        _ => ytmusic::TrackKind::Video,
    };

    Some(ytmusic::Track {
        video_id: Some(video_id),
        title: renderer.run_text(&["title"])?,
        artists,
        album: None,
        duration: None,
        thumbnails: parse::thumbnails(renderer),
        explicit: parse::explicit(renderer),
        available: true,
        kind,
        set_video_id: None,
        liked: None,
        views: None,
    })
}

/// The token for the next lot of shelves, wherever this page keeps it: the first page under
/// its section list, a continuation under `continuationContents`.
fn continuation(answer: &Value) -> Option<&str> {
    parse::find_renderers(answer, "nextContinuationData")
        .into_iter()
        .find_map(|next| next.str_at(&["continuation"]))
}

/// A shelf card as the item it stands for. A browse card is told apart by where it leads, so a
/// playlist and an album are tried first; a song or a video is what is left with a play button.
fn item(node: &Value, index: usize) -> Option<GenreItem> {
    if let Some(source) = parse::two_row_playlist(node) {
        let thumb = thumb(&source.thumbnails);
        let mut playlist = wire::playlist(source, false, true);
        playlist.cover = thumb;
        if let Some(owner) = playlist_owner(node) {
            playlist.owner = owner;
        }
        return Some(GenreItem::Playlist(playlist));
    }
    if let Some(source) = parse::two_row_album(node) {
        let thumb = thumb(&source.thumbnails);
        let mut album = wire::album(source);
        album.cover = thumb;
        return Some(GenreItem::Album(album));
    }
    if let Some(artist) = artist(node) {
        return Some(GenreItem::Artist(artist));
    }

    let source = parse::list_item_track(node).or_else(|| track(node))?;
    Some(GenreItem::Track(wire::track(source, index as u32)))
}

/// The artists under a song or video card. A channel with a page comes as a link; one without,
/// which many videos have, is plain text, so the words before the first separator are taken
/// as the name when no link is there.
fn card_artists(runs: &[Value]) -> Vec<ytmusic::ArtistRef> {
    let linked = parse::artist_runs(runs);
    if !linked.is_empty() {
        return linked;
    }

    runs.iter()
        .filter_map(|run| run.str_at(&["text"]))
        .take_while(|text| *text != SEPARATOR)
        .filter(|text| !matches!(*text, ", " | " & ") && !text.trim().is_empty())
        .filter(|text| *text != "Song" && *text != "Video")
        .map(|text| ytmusic::ArtistRef {
            name: text.to_owned(),
            id: None,
            channel: false,
        })
        .collect()
}

/// Who a shelf playlist is by, as YouTube words it under the card: "Made for ekipazh fx" on a
/// recap, the artists of a mix, "Auto playlist" on the liked songs. The kind and the song count
/// are dropped from the subtitle, since the card says those on its own.
fn playlist_owner(node: &Value) -> Option<String> {
    let renderer = node.at(&["musicTwoRowItemRenderer"])?;
    let subtitle: String = renderer
        .runs(&["subtitle"])
        .iter()
        .filter_map(|run| run.str_at(&["text"]))
        .collect();
    let owner: Vec<&str> = subtitle
        .split(SEPARATOR)
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "Playlist" && !counted(part))
        .collect();

    (!owner.is_empty()).then(|| owner.join(SEPARATOR))
}

fn counted(part: &str) -> bool {
    ["song", "songs", "track", "tracks", "view", "views"]
        .iter()
        .any(|unit| part.ends_with(unit))
        && part.starts_with(|c: char| c.is_ascii_digit())
}

fn artist(node: &Value) -> Option<SavedArtist> {
    let renderer = node.at(&["musicTwoRowItemRenderer"])?;
    if page_type(renderer) != Some(ARTIST_PAGE) {
        return None;
    }
    let id = renderer.str_at(&["navigationEndpoint", "browseEndpoint", "browseId"])?;
    if !id.starts_with("UC") {
        return None;
    }

    Some(SavedArtist {
        id: id.to_owned(),
        name: renderer.run_text(&["title"])?,
        cover: wire::cover(&parse::thumbnails(renderer)),
        added_at: None,
    })
}

fn thumb(thumbnails: &[ytmusic::Thumbnail]) -> Option<String> {
    thumbnails
        .iter()
        .find(|thumb| thumb.width >= THUMB)
        .or_else(|| thumbnails.last())
        .map(|thumb| thumb.url.clone())
}

fn card(item: &Value) -> Option<Genre> {
    let button = item.get("musicNavigationButtonRenderer")?;

    Some(Genre {
        id: button
            .str_at(&["clickCommand", "browseEndpoint", "params"])?
            .to_owned(),
        name: button.run_text(&["buttonText"])?,
        cover: None,
    })
}
