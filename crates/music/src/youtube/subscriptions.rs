use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use ytmusic::nav::Nav as _;
use ytmusic::{Client, YtMusic};

use crate::SavedArtist;

const LIBRARY_SUBSCRIPTIONS: &str = "FEmusic_library_corpus_artists";
/// The most library pages read before the list is taken as whole.
const MAX_PAGES: usize = 50;

/// Every artist the account subscribes to. The library sends a couple dozen a page, so later
/// pages are followed until the list runs out, and a page that fails fails the whole list.
pub async fn saved(api: &YtMusic) -> Result<Vec<SavedArtist>> {
    let mut response = api
        .execute(
            "browse",
            Client::Music,
            json!({ "browseId": LIBRARY_SUBSCRIPTIONS }),
        )
        .await
        .context("cannot load the library artists")?;
    let mut artists = Vec::new();
    for page in 1.. {
        artists.extend(
            ["musicTwoRowItemRenderer", "musicResponsiveListItemRenderer"]
                .into_iter()
                .flat_map(|kind| ytmusic::parse::find_renderers(&response, kind))
                .filter_map(saved_artist),
        );
        let Some(token) = next_page(&response).filter(|_| page < MAX_PAGES) else {
            break;
        };
        response = api
            .execute("browse", Client::Music, json!({ "continuation": token }))
            .await
            .with_context(|| format!("cannot load page {} of the library artists", page + 1))?;
    }
    Ok(artists)
}

/// The artists YouTube Music finds for `query`, from its search filtered to artists.
pub async fn search(api: &YtMusic, query: &str) -> Result<Vec<SavedArtist>> {
    let response = api
        .execute(
            "search",
            Client::Music,
            json!({ "query": query, "params": ytmusic::search::ARTISTS }),
        )
        .await
        .context("cannot search youtube music for artists")?;
    Ok(
        ytmusic::parse::find_renderers(&response, "musicShelfRenderer")
            .into_iter()
            .flat_map(|shelf| shelf.items(&["contents"]))
            .filter_map(|item| item.at(&["musicResponsiveListItemRenderer"]))
            .filter_map(saved_artist)
            .collect(),
    )
}

/// An artist out of a library or search row. Only a channel id (`UC…`) counts, which leaves
/// out the profiles and podcasts a row can also point at.
fn saved_artist(renderer: &Value) -> Option<SavedArtist> {
    let id = renderer
        .str_at(&["navigationEndpoint", "browseEndpoint", "browseId"])
        .or_else(|| {
            renderer.str_at(&[
                "flexColumns",
                "0",
                "musicResponsiveListItemFlexColumnRenderer",
                "text",
                "runs",
                "0",
                "navigationEndpoint",
                "browseEndpoint",
                "browseId",
            ])
        })?
        .to_string();
    id.starts_with("UC").then_some(SavedArtist {
        id,
        name: renderer.run_text(&["title"]).or_else(|| {
            renderer.run_text(&[
                "flexColumns",
                "0",
                "musicResponsiveListItemFlexColumnRenderer",
                "text",
            ])
        })?,
        cover: ytmusic::parse::thumbnails(renderer)
            .last()
            .map(|thumbnail| thumbnail.url.clone()),
        added_at: None,
    })
}

pub async fn set_saved(api: &YtMusic, artist_id: &str, saved: bool) -> Result<()> {
    let artist = api
        .execute("browse", Client::Music, json!({ "browseId": artist_id }))
        .await
        .with_context(|| format!("cannot load artist {artist_id} before changing subscription"))?;
    let channel_id = ytmusic::parse::find_renderer(&artist, "subscribeButtonRenderer")
        .and_then(|button| button.get("channelId"))
        .and_then(|channel_id| channel_id.as_str())
        .context("artist response has no subscription channel")?;
    let endpoint = match saved {
        true => "subscription/subscribe",
        false => "subscription/unsubscribe",
    };
    api.execute(
        endpoint,
        Client::Music,
        json!({ "channelIds": [channel_id] }),
    )
    .await
    .with_context(|| format!("cannot change subscription for artist {artist_id}"))?;
    Ok(())
}

/// The token for the library list's next page. Only the shelf or grid's own continuation
/// counts, since the sort chips carry reload tokens that would start the list over.
fn next_page(response: &Value) -> Option<String> {
    [
        "musicShelfRenderer",
        "musicShelfContinuation",
        "gridRenderer",
        "gridContinuation",
    ]
    .into_iter()
    .flat_map(|kind| ytmusic::parse::find_renderers(response, kind))
    .find_map(ytmusic::parse::shelf_continuation)
    .or_else(|| {
        ytmusic::parse::find_renderers(response, "continuationItemRenderer")
            .into_iter()
            .find_map(|item| item.str_at(&["continuationEndpoint", "continuationCommand", "token"]))
            .map(str::to_owned)
    })
}
