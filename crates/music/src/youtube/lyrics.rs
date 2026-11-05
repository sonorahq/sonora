use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use ytmusic::{Client, YtMusic, nav::Nav as _, parse::find_renderers};

use crate::lyrics::catalog;
use crate::{Lyrics, LyricsHit, LyricsProvider, LyricsQuery};

const SOURCE: &str = "YouTube Music";

/// Public lyrics lookup has its own guest client and does not need a playback account.
pub struct YouTubeLyrics {
    api: YtMusic,
}

impl YouTubeLyrics {
    pub fn new() -> Self {
        Self {
            api: YtMusic::anonymous(),
        }
    }

    async fn sheet(&self, id: &str) -> Result<Option<Lyrics>> {
        let next = self
            .api
            .execute(
                "next",
                Client::Music,
                json!({
                    "videoId": id,
                    "playlistId": format!("RDAMVM{id}"),
                    "enablePersistentPlaylistPanel": true,
                }),
            )
            .await?;
        let Some(browse) = browse_id(&next) else {
            return Ok(None);
        };
        let response = self
            .api
            .execute("browse", Client::Music, json!({"browseId": browse}))
            .await?;
        Ok(find_renderers(&response, "musicDescriptionShelfRenderer")
            .into_iter()
            .filter_map(|shelf| shelf.run_text(&["description"]))
            .find(|text| !text.trim().is_empty())
            .map(Lyrics::plain))
    }
}

impl Default for YouTubeLyrics {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LyricsProvider for YouTubeLyrics {
    fn name(&self) -> &'static str {
        SOURCE
    }

    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>> {
        if let Some(id) = query.id_for("youtube")
            && let Some(sheet) = self.sheet(id).await?
        {
            return Ok(vec![catalog::hit(SOURCE, query, sheet)]);
        }
        // Videos can lack lyrics even when the corresponding album recording has them. The search
        // goes out signed out so it stays out of the account's search history.
        let tracks = self
            .api
            .search_songs_with(&format!("{} {}", query.title, query.artist), false)
            .await?
            .into_iter()
            .enumerate()
            .map(|(index, track)| super::wire::track(track, index as u32))
            .collect();
        let mut hits = Vec::new();
        for (id, mut hit) in catalog::candidates(SOURCE, query, tracks) {
            if query.id_for("youtube") == Some(id.as_str()) {
                continue;
            }
            if let Some(sheet) = self.sheet(&id).await? {
                hit.lyrics = sheet;
                hits.push(hit);
            }
        }
        Ok(hits)
    }
}

fn browse_id(next: &Value) -> Option<&str> {
    find_renderers(next, "browseEndpoint")
        .into_iter()
        .find_map(|endpoint| {
            let kind = endpoint.str_at(&[
                "browseEndpointContextSupportedConfigs",
                "browseEndpointContextMusicConfig",
                "pageType",
            ]);
            let id = endpoint.str_at(&["browseId"])?;
            (kind == Some("MUSIC_PAGE_TYPE_TRACK_LYRICS") || id.starts_with("MPLYt")).then_some(id)
        })
}
