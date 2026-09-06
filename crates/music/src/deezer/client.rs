use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use reqwest::Url;
use reqwest::cookie::Jar;
use reqwest::Client;
use serde_json::{Value, json};


use crate::deezer::{stream, wire};
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, Genre, GenreDetail, HomeFeed, MediaKind, MusicApi,
    Playlist, PlaylistDetail, SavedArtist, Track, UserProfile,
};

const API: &str = "https://api.deezer.com";
const GATEWAY: &str = "https://www.deezer.com/ajax/gw-light.php";
const MEDIA: &str = "https://media.deezer.com/v1/get_url";
const PORTRAIT_LIMIT: usize = 24;
const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

#[derive(Clone)]
pub struct Session {
    http: Client,
    arl: Option<String>,
    api_token: Arc<Mutex<String>>,
    license_token: Arc<Mutex<String>>,
}

pub struct DeezerClient {
    session: Arc<Session>,
    user: UserProfile,
}

impl DeezerClient {
    pub fn new(session: Arc<Session>, user: UserProfile) -> Self {
        Self { session, user }
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let url = format!("{API}{path}");
        let response = self
            .session
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("cannot reach {url}"))?;
        let body: Value = response
            .json()
            .await
            .context("cannot read the deezer response")?;
        if let Some(error) = body.get("error")
            && !error.is_null()
        {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("deezer refused the request");
            bail!("{message}");
        }
        Ok(body)
    }

    async fn page(&self, path: &str, limit: u32) -> Result<Vec<Value>> {
        let body = self.get(&format!("{path}?limit={limit}")).await?;
        Ok(wire::list(Some(&body))
            .into_iter()
            .cloned()
            .collect())
    }
}

#[async_trait]
impl MusicApi for DeezerClient {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let path = match kind {
            MediaKind::Track => "track",
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
        };
        Some(format!("https://www.deezer.com/{path}/{id}"))
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(self.user.clone())
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let profile = self.get(&format!("/artist/{artist_id}")).await?;
        let top = self
            .page(&format!("/artist/{artist_id}/top"), 10)
            .await?
            .iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect();
        let albums = self
            .page(&format!("/artist/{artist_id}/albums"), 50)
            .await?
            .iter()
            .map(wire::album)
            .collect();
        Ok(wire::artist(&profile, top, albums))
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        Ok(wire::artist_profile(
            &self.get(&format!("/artist/{artist_id}")).await?,
        ))
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let mut images = HashMap::new();
        for id in ids.into_iter().take(PORTRAIT_LIMIT) {
            let Ok(artist) = self.get(&format!("/artist/{id}")).await else {
                continue;
            };
            if let Some(cover) = wire::picture_large(&artist) {
                images.insert(id, cover);
            }
        }
        Ok(images)
    }

    async fn saved_tracks(&self, limit: u32) -> Result<Vec<Track>> {
        let Some(user) = (!self.user.id.is_empty()).then_some(self.user.id.as_str()) else {
            return Ok(Vec::new());
        };
        Ok(self
            .page(&format!("/user/{user}/tracks"), limit)
            .await?
            .iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect())
    }

    async fn set_track_saved(&self, _track_id: &str, _saved: bool) -> Result<()> {
        bail!("saving tracks is not supported yet")
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        Ok(wire::track(&self.get(&format!("/track/{track_id}")).await?, 0))
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn playlists(&self, limit: u32) -> Result<Vec<Playlist>> {
        let Some(user) = (!self.user.id.is_empty()).then_some(self.user.id.as_str()) else {
            return Ok(Vec::new());
        };
        Ok(self
            .page(&format!("/user/{user}/playlists"), limit)
            .await?
            .iter()
            .map(|playlist| wire::playlist(playlist, user))
            .collect())
    }

    async fn create_playlist(&self, _name: &str) -> Result<String> {
        bail!("creating playlists is not supported yet")
    }

    async fn rename_playlist(&self, _playlist_id: &str, _name: &str) -> Result<()> {
        bail!("renaming playlists is not supported yet")
    }

    async fn delete_playlist(&self, _playlist_id: &str) -> Result<()> {
        bail!("deleting playlists is not supported yet")
    }

    async fn remove_playlist_from_library(&self, _playlist_id: &str) -> Result<()> {
        bail!("removing playlists is not supported yet")
    }

    async fn add_playlist_to_library(&self, _playlist_id: &str) -> Result<()> {
        bail!("saving playlists is not supported yet")
    }

    async fn set_playlist_public(&self, _playlist_id: &str, _public: bool) -> Result<()> {
        bail!("playlist visibility is not supported yet")
    }

    async fn add_track_to_playlist(&self, _playlist_id: &str, _track_id: &str) -> Result<()> {
        bail!("adding tracks to playlists is not supported yet")
    }

    async fn remove_track_from_playlist(&self, _playlist_id: &str, _track_id: &str) -> Result<()> {
        bail!("removing tracks from playlists is not supported yet")
    }

    async fn saved_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let Some(user) = (!self.user.id.is_empty()).then_some(self.user.id.as_str()) else {
            return Ok(Vec::new());
        };
        Ok(self
            .page(&format!("/user/{user}/albums"), limit)
            .await?
            .iter()
            .map(wire::album)
            .collect())
    }

    async fn set_album_saved(&self, _album_id: &str, _saved: bool) -> Result<()> {
        bail!("saving albums is not supported yet")
    }

    async fn saved_artists(&self, limit: u32) -> Result<Vec<SavedArtist>> {
        let Some(user) = (!self.user.id.is_empty()).then_some(self.user.id.as_str()) else {
            return Ok(Vec::new());
        };
        Ok(self
            .page(&format!("/user/{user}/artists"), limit)
            .await?
            .iter()
            .map(wire::saved_artist)
            .collect())
    }

    async fn set_artist_saved(&self, _artist_id: &str, _saved: bool) -> Result<()> {
        bail!("following artists is not supported yet")
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        Ok(wire::album_detail(
            &self.get(&format!("/album/{album_id}")).await?,
        ))
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        Ok(wire::playlist_detail(
            &self.get(&format!("/playlist/{playlist_id}")).await?,
            &self.user.id,
        ))
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self.playlist(playlist_id).await?.tracks)
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        Ok(crate::distinct_covers(
            &self.playlist_tracks(playlist_id).await?,
            wanted,
        ))
    }

    async fn track_radio(&self, track_id: &str) -> Result<Vec<Track>> {
        let track = self.track(track_id).await?;
        let Some(artist_id) = track
            .artist_refs
            .first()
            .and_then(|artist| artist.id.clone())
        else {
            return Ok(Vec::new());
        };
        Ok(self
            .page(&format!("/artist/{artist_id}/radio"), 25)
            .await
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let body = self
            .get(&format!(
                "/search/track?q={}&limit=25",
                urlencoding(query)
            ))
            .await?;
        Ok(wire::list(Some(&body))
            .into_iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let body = self
            .get(&format!(
                "/search/album?q={}&limit=20",
                urlencoding(query)
            ))
            .await?;
        Ok(wire::list(Some(&body)).into_iter().map(wire::album).collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let body = self
            .get(&format!(
                "/search/playlist?q={}&limit=20",
                urlencoding(query)
            ))
            .await?;
        Ok(wire::list(Some(&body))
            .into_iter()
            .map(|playlist| wire::playlist(playlist, &self.user.id))
            .collect())
    }

    async fn home(&self) -> Result<HomeFeed> {
        let chart = self.get("/chart").await?;
        let tracks = wire::list(chart.get("tracks"))
            .into_iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect();
        let albums = wire::list(chart.get("albums"))
            .into_iter()
            .map(wire::album)
            .map(crate::GenreItem::Album)
            .collect();
        let playlists = wire::list(chart.get("playlists"))
            .into_iter()
            .map(|playlist| wire::playlist(playlist, &self.user.id))
            .map(crate::GenreItem::Playlist)
            .collect();
        Ok(HomeFeed {
            listen_again: Vec::new(),
            quick_picks: Some(tracks),
            sections: vec![
                crate::GenreSection {
                    title: "Albums".into(),
                    items: albums,
                },
                crate::GenreSection {
                    title: "Playlists".into(),
                    items: playlists,
                },
            ],
        })
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        let body = self.get("/genre").await?;
        Ok(wire::list(Some(&body))
            .into_iter()
            .filter_map(|genre| {
                Some(Genre {
                    id: wire::id(genre)?,
                    name: genre.get("name")?.as_str()?.to_string(),
                    cover: wire::picture(genre),
                })
            })
            .collect())
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        let profile = self.get(&format!("/genre/{genre_id}")).await?;
        let artists = self
            .page(&format!("/genre/{genre_id}/artists"), 20)
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|artist| {
                Some(crate::GenreItem::Genre(Genre {
                    id: wire::id(artist)?,
                    name: artist.get("name")?.as_str()?.to_string(),
                    cover: wire::picture(artist),
                }))
            })
            .collect();
        Ok(GenreDetail {
            name: profile
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            sections: vec![crate::GenreSection {
                title: "Artists".into(),
                items: artists,
            }],
        })
    }
}

impl Session {
    pub fn new(arl: Option<String>) -> Self {
        let jar = Jar::default();
        if let Some(arl) = &arl
            && let Ok(url) = Url::parse("https://www.deezer.com")
        {
            jar.add_cookie_str(
                &format!("arl={arl}; Domain=.deezer.com; Path=/"),
                &url,
            );
        }
        Self {
            http: Client::builder()
                .user_agent(AGENT)
                .cookie_provider(Arc::new(jar))
                .build()
                .unwrap_or_else(|_| Client::new()),
            arl,
            api_token: Arc::new(Mutex::new(String::new())),
            license_token: Arc::new(Mutex::new(String::new())),
        }
    }

    pub async fn identify(&self) -> Result<UserProfile> {
        let Some(_arl) = &self.arl else {
            return Ok(UserProfile {
                id: String::new(),
                display_name: "Deezer".to_string(),
            });
        };
        let url = format!(
            "{GATEWAY}?method=deezer.getUserData&input=3&api_version=1.0&api_token=null"
        );
        let body: Value = self
            .http
            .post(url)
            .json(&json!({}))
            .send()
            .await
            .context("cannot reach deezer")?
            .json()
            .await
            .context("cannot read deezer user data")?;
        let results = body.get("results").context("deezer sent no user data")?;
        let user = results.get("USER").context("deezer sent no user")?;
        let id = user
            .get("USER_ID")
            .and_then(|id| {
                id.as_u64()
                    .map(|id| id.to_string())
                    .or_else(|| id.as_str().map(str::to_string))
            })
            .context("deezer sent no user id")?;
        if id == "0" {
            bail!("the ARL was not accepted");
        }
        if let Some(token) = results
            .get("checkForm")
            .or_else(|| results.get("checkFormLogin"))
            .and_then(Value::as_str)
        {
            *self.api_token.lock().expect("deezer api token") = token.to_string();
        }
        if let Some(token) = user
            .get("OPTIONS")
            .and_then(|options| options.get("license_token"))
            .or_else(|| results.get("LICENSE_TOKEN"))
            .and_then(Value::as_str)
        {
            *self.license_token.lock().expect("deezer license") = token.to_string();
        }
        let display_name = user
            .get("BLOG_NAME")
            .or_else(|| user.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .unwrap_or("Deezer")
            .to_string();
        Ok(UserProfile { id, display_name })
    }

    pub async fn audio(&self, track_id: &str) -> Result<Vec<u8>> {
        if self.arl.is_some() {
            match self.stream(track_id).await {
                Ok(data) if !data.is_empty() => return Ok(data),
                Ok(_) => log::warn!("deezer: stream for {track_id} was empty"),
                Err(error) => log::warn!("deezer: cannot stream {track_id}: {error:#}"),
            }
        }
        self.preview(track_id).await
    }

    async fn stream(&self, track_id: &str) -> Result<Vec<u8>> {
        let token = self
            .api_token
            .lock()
            .expect("deezer api token")
            .clone();
        let license = self
            .license_token
            .lock()
            .expect("deezer license")
            .clone();
        if token.is_empty() || license.is_empty() {
            let _ = self.identify().await?;
        }
        let token = self
            .api_token
            .lock()
            .expect("deezer api token")
            .clone();
        let license = self
            .license_token
            .lock()
            .expect("deezer license")
            .clone();
        if token.is_empty() || license.is_empty() {
            bail!("deezer sent no stream tokens");
        }
        let id: u64 = track_id.parse().context("track id is not a number")?;
        let listed = self
            .gateway("song.getListData", json!({ "sng_ids": [id] }), &token)
            .await?;
        let song = listed
            .get("data")
            .and_then(Value::as_array)
            .or_else(|| listed.as_array())
            .and_then(|songs| songs.first())
            .or(Some(&listed))
            .context("deezer sent no track token")?;
        let track_token = song
            .get("TRACK_TOKEN")
            .and_then(Value::as_str)
            .context("deezer sent no track token")?;
        let body = json!({
            "license_token": license,
            "media": [{
                "type": "FULL",
                "formats": [
                    {"cipher": "BF_CBC_STRIPE", "format": "MP3_320"},
                    {"cipher": "BF_CBC_STRIPE", "format": "MP3_128"}
                ]
            }],
            "track_tokens": [track_token]
        });
        let media: Value = self
            .http
            .post(MEDIA)
            .json(&body)
            .send()
            .await
            .context("cannot reach deezer media")?
            .json()
            .await
            .context("cannot read deezer media")?;
        let url = media
            .pointer("/data/0/media/0/sources/0/url")
            .and_then(Value::as_str)
            .context("deezer sent no stream url")?;
        let bytes = self
            .http
            .get(url)
            .send()
            .await
            .context("cannot fetch the stream")?
            .bytes()
            .await
            .context("cannot read the stream")?;
        stream::unlock(track_id, &bytes)
    }

    async fn gateway(&self, method: &str, payload: Value, token: &str) -> Result<Value> {
        let url = format!(
            "{GATEWAY}?method={method}&input=3&api_version=1.0&api_token={token}"
        );
        let body: Value = self
            .http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("cannot reach deezer")?
            .json()
            .await
            .context("cannot read the deezer gateway")?;
        if let Some(error) = body.get("error")
            && !error.is_null()
            && error.as_array().is_none_or(|errors| !errors.is_empty())
            && !error.as_object().is_some_and(|map| map.is_empty())
        {
            bail!("deezer gateway refused {method}: {error}");
        }
        body.get("results")
            .cloned()
            .context("deezer gateway sent no results")
    }

    async fn preview(&self, track_id: &str) -> Result<Vec<u8>> {
        let url = format!("{API}/track/{track_id}");
        let body: Value = self
            .http
            .get(&url)
            .send()
            .await
            .context("cannot reach deezer")?
            .json()
            .await
            .context("cannot read the track")?;
        let preview = wire::preview(&body).context("this track has no preview")?;
        let response = self
            .http
            .get(&preview)
            .send()
            .await
            .context("cannot fetch the preview")?;
        let bytes = response
            .bytes()
            .await
            .context("cannot read the preview")?;
        if bytes.len() < 128 || bytes.starts_with(b"<") || bytes.starts_with(b"{") {
            bail!("deezer preview was not audio");
        }
        Ok(bytes.to_vec())
    }
}

fn urlencoding(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}
