//! SoundCloud data access. Reads go over the internal `api-v2.soundcloud.com`: the listener's
//! token rides in an `Authorization: OAuth …` header, and every call carries the web app's
//! `client_id`, extracted from SoundCloud's own bundle the way Deezer's stream secret is, with
//! the last known value as a fallback. Writes go over the public `api.soundcloud.com`, and only
//! for a sign-in through a registered app, since the cookie's token reads alone.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::soundcloud::oauth::{self, App, Grant};
use crate::soundcloud::{auth, wire};
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, ArtistRef, Encoding, GenreItem, GenreSection,
    HomeFeed, MediaKind, MusicApi, Offer, Playlist, PlaylistDetail, Quality, SUGGESTIONS,
    SavedArtist, Track, UserDetail, UserProfile, distinct_covers, escape,
};

const API: &str = "https://api-v2.soundcloud.com";
/// The public api, which takes the writes a registered app's grant may make.
const PUBLIC: &str = "https://api.soundcloud.com";
/// The page whose script bundle carries the web app's `client_id`.
const WEB: &str = "https://soundcloud.com";

/// The levels SoundCloud serves, best first. Every track streams as AAC over HLS, at 160 kbps
/// for the standard stream and 96 kbps for the lower one.
pub(crate) const QUALITIES: &[Offer] = &[
    Offer {
        quality: Quality::Normal,
        encoding: Encoding::Aac { kbps: 160 },
        up_to: false,
    },
    Offer {
        quality: Quality::Low,
        encoding: Encoding::Aac { kbps: 96 },
        up_to: false,
    },
];

/// The AAC HLS transcoding presets, best first.
const FORMATS: [&str; 2] = ["aac_160k", "aac_96k"];

/// How many items a listing asks for per page. The api caps a page around here and hands out a
/// `next_href` for the rest.
const PAGE: u32 = 100;

/// The last known `client_id` SoundCloud's web app used, kept so a bundle redesign degrades to
/// a warning instead of killing every request. Refreshed when the extraction no longer matches.
const FALLBACK_CLIENT_ID: &str = "2t9loNQH90kzJcsFCODdigxfp325aq4z";

/// A track resolved down to where its audio lives: the media playlist it streams from.
pub struct Opened {
    /// The media playlist (`.m3u8`) url, already signed and ready to fetch.
    pub m3u8: String,
    pub duration: Option<Duration>,
    /// The preset served, which is also the vault variant the bytes are kept under.
    pub served: &'static str,
}

#[derive(Clone)]
pub struct SoundCloudClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    /// The token every request carries. A registered app's grant is renewed in place before it
    /// lapses; a cookie's never lapses here and is simply refused once SoundCloud drops it.
    grant: tokio::sync::Mutex<Grant>,
    /// The registered app the grant came from, which can renew it and write. None for a
    /// cookie sign-in.
    app: Option<App>,
    client_id: String,
    /// Where reads, writes and token renewals go: the real hosts, or a test's loopback server.
    api: String,
    public: String,
    token_url: String,
    profile: RwLock<UserProfile>,
    /// Artist avatars already looked up, so a search that ranks the same users on every
    /// keystroke does not fetch them again.
    portraits: Mutex<HashMap<String, String>>,
}

impl SoundCloudClient {
    /// Builds an http client for the SoundCloud api.
    pub(crate) fn http() -> Result<reqwest::Client> {
        reqwest::Client::builder()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64; rv:142.0) Gecko/20100101 Firefox/142.0")
            .build()
            .context("cannot build the soundcloud http client")
    }

    /// Signs in with an `oauth_token` cookie: extracts the web app's `client_id` and checks the
    /// token against `/me`. A refused token answers an empty id. The client reads only.
    pub async fn connect(token: &str) -> Result<Self> {
        Self::sign_in(Grant::lasting(token), None).await
    }

    /// Signs in with a registered app's grant, which also writes. A grant about to lapse is
    /// renewed on the first request.
    pub async fn connect_app(grant: Grant, app: App) -> Result<Self> {
        Self::sign_in(grant, Some(app)).await
    }

    async fn sign_in(grant: Grant, app: Option<App>) -> Result<Self> {
        let http = Self::http()?;
        let client_id = client_id(&http).await;
        let client = Self::build(http, grant, app, client_id, API, PUBLIC, oauth::TOKEN);
        client.refresh().await?;
        if client.inner.profile.read().await.id.is_empty() {
            bail!("the oauth token was refused; sign in to soundcloud.com again");
        }
        Ok(client)
    }

    /// A client for `token` that has not reached SoundCloud. It can recall a downloaded track
    /// and do nothing else until it connects.
    pub fn offline(token: &str) -> Result<Self> {
        Ok(Self::build(
            Self::http()?,
            Grant::lasting(token),
            None,
            FALLBACK_CLIENT_ID.to_owned(),
            API,
            PUBLIC,
            oauth::TOKEN,
        ))
    }

    fn build(
        http: reqwest::Client,
        grant: Grant,
        app: Option<App>,
        client_id: String,
        api: &str,
        public: &str,
        token_url: &str,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                grant: tokio::sync::Mutex::new(grant),
                app,
                client_id,
                api: api.to_owned(),
                public: public.to_owned(),
                token_url: token_url.to_owned(),
                profile: RwLock::new(UserProfile {
                    id: String::new(),
                    display_name: String::new(),
                    avatar: None,
                }),
                portraits: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Whether the sign-in can like, follow and edit playlists: only a registered app's can.
    pub fn writable(&self) -> bool {
        self.inner.app.is_some()
    }

    /// The access token to send now. One about to lapse is renewed first when the app can
    /// renew it, and the renewed grant is kept, so the next start does not sign in again. A
    /// unit test's client keeps nothing, so a test never touches the real accounts folder.
    async fn bearer(&self) -> Result<String> {
        let mut grant = self.inner.grant.lock().await;
        if let (true, Some(app), Some(refresh)) = (
            grant.stale(oauth::now()),
            &self.inner.app,
            grant.refresh.clone(),
        ) {
            let fresh =
                oauth::refresh(&self.inner.http, &self.inner.token_url, app, &refresh).await?;
            if !cfg!(test)
                && let Err(error) = auth::store(&auth::Credentials::from_grant(&fresh))
            {
                log::warn!("soundcloud: cannot keep the renewed sign-in: {error:#}");
            }
            *grant = fresh;
        }
        Ok(grant.access.clone())
    }

    /// The http client the api and media requests share, for a media request that wants the
    /// same timeouts as the api's.
    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.http
    }

    /// Pulls the account from `/me`, which also proves the token is still good.
    async fn refresh(&self) -> Result<()> {
        let answer = self
            .get("/me")
            .await
            .context("cannot load the soundcloud account")?;
        *self.inner.profile.write().await = wire::profile(&answer);
        Ok(())
    }

    async fn user_id(&self) -> String {
        self.inner.profile.read().await.id.clone()
    }

    /// A `GET` to `target`, a path on the api or a full url a `next_href` handed out. The
    /// `client_id` and the `OAuth` header go on every request.
    async fn get(&self, target: &str) -> Result<Value> {
        self.send(reqwest::Method::GET, target, None).await
    }

    async fn send(
        &self,
        method: reqwest::Method,
        target: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let url = match target.starts_with("http") {
            true => target.to_owned(),
            false => {
                let separator = if target.contains('?') { '&' } else { '?' };
                format!(
                    "{}{target}{separator}client_id={}",
                    self.inner.api,
                    escape::component(&self.inner.client_id)
                )
            }
        };
        let token = self.bearer().await?;
        let mut request = self
            .inner
            .http
            .request(method.clone(), &url)
            .header("Authorization", format!("OAuth {token}"));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("cannot reach {url}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .context("cannot read the soundcloud answer")?;
        if !status.is_success() {
            let trouble = json_text(&text).unwrap_or_else(|| status.to_string());
            bail!("soundcloud refused {method} {target}: {trouble}");
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text)
            .with_context(|| format!("soundcloud answered {target} with no json"))
    }

    /// Walks a listing that answers `collection` + `next_href`, one page at a time.
    async fn list(&self, path: &str) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        let mut target = path.to_owned();
        loop {
            let answer = self.get(&target).await?;
            let items = wire::collection(&answer);
            let next = answer
                .get("next_href")
                .and_then(Value::as_str)
                .filter(|next| !next.is_empty() && *next != target.as_str())
                .map(str::to_owned);
            all.extend(items);
            match next {
                Some(next) => target = next,
                None => return Ok(all),
            }
        }
    }

    /// Turns an id or a permalink into a numeric id, resolving the permalink when the id is not
    /// one. SoundCloud's share urls carry permalinks, while the api wants numeric ids.
    async fn numeric(&self, id: &str) -> Result<String> {
        if !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()) {
            return Ok(id.to_owned());
        }
        let url = match id.starts_with("http") {
            true => id.to_owned(),
            false => format!("https://soundcloud.com/{id}"),
        };
        let answer = self
            .get(&format!("/resolve?url={}", escape::component(&url)))
            .await
            .with_context(|| format!("cannot resolve the soundcloud link {id}"))?;
        wire::id(&answer).context("the soundcloud link resolves to no id")
    }

    /// The full record of one track, including the transcodings and `track_authorization`
    /// playback needs.
    async fn track_data(&self, track_id: &str) -> Result<Value> {
        let id = self.numeric(track_id).await?;
        let answer = self
            .get(&format!("/tracks?ids={}", escape::component(&id)))
            .await
            .with_context(|| format!("cannot load the soundcloud track {track_id}"))?;
        match answer.as_array().and_then(|list| list.first()).cloned() {
            Some(track) => Ok(track),
            None => Ok(answer),
        }
    }

    /// The full record of a playlist, whether it is a regular playlist or one of SoundCloud's
    /// own `soundcloud:system-playlists:…` mixes, whose id is a urn rather than a number.
    async fn playlist_data(&self, id: &str) -> Result<Value> {
        let path = if id.starts_with("soundcloud:system-playlists:") {
            format!("/system-playlists/{id}?representation=full")
        } else {
            let id = self.numeric(id).await?;
            format!("/playlists/{}?representation=full", escape::component(&id))
        };
        self.get(&path).await
    }

    /// The tracks of a playlist, in order. Some playlists list stubs (`{id, kind}`) instead of
    /// full tracks, so those are fetched in batches and put back in place.
    async fn tracks_from(&self, answer: &Value) -> Result<Vec<Track>> {
        let entries = answer
            .get("tracks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut tracks: Vec<Option<Track>> = Vec::with_capacity(entries.len());
        let mut stubs: Vec<(usize, String)> = Vec::new();
        for entry in &entries {
            match wire::track(entry) {
                Some(track) => tracks.push(Some(track)),
                None => match entry.get("id").and_then(wire::id) {
                    Some(id) => {
                        stubs.push((tracks.len(), id));
                        tracks.push(None);
                    }
                    None => tracks.push(None),
                },
            }
        }
        if !stubs.is_empty() {
            let mut by_id: HashMap<String, Track> = HashMap::new();
            for chunk in stubs.chunks(50) {
                let joined = chunk
                    .iter()
                    .map(|(_, id)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(",");
                let Ok(page) = self
                    .get(&format!("/tracks?ids={}", escape::component(&joined)))
                    .await
                else {
                    continue;
                };
                for track in wire::track_list(&page) {
                    if let Some(id) = track.id.clone() {
                        by_id.insert(id, track);
                    }
                }
            }
            for (at, id) in stubs {
                tracks[at] = by_id.get(&id).cloned();
            }
        }
        Ok(tracks.into_iter().flatten().collect())
    }

    /// Resolves a track down to its media playlist: a signed `.m3u8` url for the best AAC HLS
    /// transcoding the track offers at `quality`.
    pub async fn open_stream(&self, track_id: &str, quality: Quality) -> Result<Opened> {
        let data = self.track_data(track_id).await?;
        let duration =
            wire::number(&data, &["duration", "full_duration"]).map(Duration::from_millis);
        let authorization = wire::text(&data, &["track_authorization"])
            .filter(|token| !token.is_empty())
            .with_context(|| format!("the soundcloud track {track_id} is not playable"))?;
        let transcodings = data
            .get("media")
            .and_then(|media| media.get("transcodings"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut last = anyhow::anyhow!("no format was offered");
        for preset in presets_from(quality) {
            let Some(transcoding) = transcodings
                .iter()
                .find(|entry| entry.get("preset").and_then(Value::as_str) == Some(preset))
            else {
                continue;
            };
            let Some(media_url) = transcoding.get("url").and_then(Value::as_str) else {
                continue;
            };
            let url = transcoding_url(media_url, &self.inner.client_id, authorization);
            match self.stream_url(&url).await {
                Ok(m3u8) => {
                    log::debug!("soundcloud: {track_id} streams as {preset}");
                    return Ok(Opened {
                        m3u8,
                        duration,
                        served: preset,
                    });
                }
                Err(error) => last = error,
            }
        }
        let offered: Vec<&str> = transcodings
            .iter()
            .filter_map(|entry| entry.get("preset").and_then(Value::as_str))
            .collect();
        Err(last.context(format!(
            "the soundcloud track has no playable stream (offered: {offered:?})"
        )))
    }

    /// Resolves one transcoding url to a signed stream url.
    async fn stream_url(&self, url: &str) -> Result<String> {
        let token = self.bearer().await?;
        let answer: Value = self
            .inner
            .http
            .get(url)
            .header("Authorization", format!("OAuth {token}"))
            .send()
            .await
            .context("cannot resolve the soundcloud stream url")?
            .error_for_status()
            .context("soundcloud refused the stream url")?
            .json()
            .await
            .context("cannot read the soundcloud stream answer")?;
        answer
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("the stream answer names no url")
    }

    /// A write over the public api, as the registered app's grant. A cookie sign-in cannot
    /// write, which the app shows as not implemented rather than as a failure.
    async fn write(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let url = format!("{}{path}", self.inner.public);
        let token = self.bearer().await?;
        let mut request = self
            .inner
            .http
            .request(method.clone(), &url)
            .header("Authorization", format!("OAuth {token}"))
            .header("Accept", "application/json; charset=utf-8");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .with_context(|| format!("cannot reach {url}"))?;
        let status = response.status();
        let text = response
            .text()
            .await
            .context("cannot read the soundcloud answer")?;
        if status.as_u16() == 429 {
            return Err(anyhow::Error::new(crate::Throttled)
                .context(format!("soundcloud is refusing {method} {path} for now")));
        }
        if !status.is_success() {
            let trouble = json_text(&text).unwrap_or_else(|| status.to_string());
            bail!("soundcloud refused {method} {path}: {trouble}");
        }
        Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The urn the public api names `id` by, `soundcloud:{kind}:{number}`. An id that already
    /// is a urn, such as a system playlist's, stays as it is.
    async fn urn(&self, kind: &str, id: &str) -> Result<String> {
        if id.starts_with("soundcloud:") {
            return Ok(id.to_owned());
        }
        Ok(format!("soundcloud:{kind}:{}", self.numeric(id).await?))
    }

    /// Likes or unlikes a track or a playlist. SoundCloud's albums are playlists too.
    async fn like(&self, kind: &str, id: &str, liked: bool) -> Result<()> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let urn = self.urn(kind, id).await?;
        let method = match liked {
            true => reqwest::Method::POST,
            false => reqwest::Method::DELETE,
        };
        self.write(method, &format!("/likes/{kind}/{urn}"), None)
            .await
            .map(|_| ())
    }

    /// Changes one field of a playlist's metadata, such as its title or its sharing.
    async fn update_playlist(&self, playlist_id: &str, playlist: Value) -> Result<()> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let urn = self.urn("playlists", playlist_id).await?;
        self.write(
            reqwest::Method::PUT,
            &format!("/playlists/{urn}"),
            Some(json!({ "playlist": playlist })),
        )
        .await
        .map(|_| ())
    }

    /// The numeric ids of a playlist's tracks, in order, stubs included. The public api only
    /// replaces a playlist's tracks whole, so an edit starts from this.
    async fn track_ids(&self, playlist_id: &str) -> Result<Vec<String>> {
        let answer = self.playlist_data(playlist_id).await?;
        Ok(answer
            .get("tracks")
            .and_then(Value::as_array)
            .map(|tracks| {
                tracks
                    .iter()
                    .filter_map(|entry| entry.get("id").and_then(wire::id))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Replaces a playlist's tracks with `ids`, in that order.
    async fn set_tracks(&self, playlist_id: &str, ids: &[String]) -> Result<()> {
        let tracks: Vec<Value> = ids
            .iter()
            .map(|id| json!({ "urn": format!("soundcloud:tracks:{id}") }))
            .collect();
        self.update_playlist(playlist_id, json!({ "tracks": tracks }))
            .await
    }
}

#[async_trait]
impl MusicApi for SoundCloudClient {
    fn share_url(&self, _kind: MediaKind, _id: &str) -> Option<String> {
        // SoundCloud's share urls carry permalinks, which a numeric id cannot stand in for and
        // a sync call has no way to resolve. The open and copy-link controls hide instead.
        None
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(self.inner.profile.read().await.clone())
    }

    async fn user(&self, user_id: &str) -> Result<UserDetail> {
        let id = self.numeric(user_id).await?;
        let answer = self
            .get(&format!("/users/{}", escape::component(&id)))
            .await
            .with_context(|| format!("cannot load the soundcloud user {user_id}"))?;
        Ok(UserDetail {
            id: id.clone(),
            name: wire::text(&answer, &["username", "full_name"])
                .unwrap_or_default()
                .to_owned(),
            avatar: wire::image(&answer, &["avatar_url"], 300),
            followers: wire::number(&answer, &["followers_count"]),
            following: wire::number(&answer, &["followings_count"]),
            playlists: Vec::new(),
        })
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let id = self.numeric(artist_id).await?;
        let detail_path = format!("/users/{}", escape::component(&id));
        let top_path = format!("/users/{}/toptracks?limit=20", escape::component(&id));
        let albums_path = format!("/users/{}/albums?limit={PAGE}", escape::component(&id));
        let (detail, top, albums) = tokio::join!(
            self.get(&detail_path),
            self.get(&top_path),
            self.get(&albums_path),
        );
        let detail = detail.context("cannot load the artist")?;
        let name = wire::text(&detail, &["username", "full_name"])
            .unwrap_or_default()
            .to_owned();
        let own = ArtistRef {
            name: name.clone(),
            id: Some(id.clone()),
        };
        Ok(Artist {
            name,
            cover_large: wire::image(&detail, &["avatar_url"], 500),
            banner: None,
            biography: detail
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            monthly_listeners: wire::number(&detail, &["followers_count"]),
            top_tracks: top.map(|page| wire::track_list(&page)).unwrap_or_default(),
            albums: albums
                .map(|page| {
                    wire::album_list(&page)
                        .into_iter()
                        .map(|album| album.credit(&own))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let id = self.numeric(artist_id).await?;
        let answer = self
            .get(&format!("/users/{}", escape::component(&id)))
            .await
            .context("cannot load the artist")?;
        Ok(ArtistProfile {
            name: wire::text(&answer, &["username", "full_name"])
                .unwrap_or_default()
                .to_owned(),
            cover_large: wire::image(&answer, &["avatar_url"], 500),
            biography: answer
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let mut images = HashMap::new();
        let mut missing = Vec::new();
        {
            let known = lock(&self.inner.portraits);
            for id in ids {
                match known.get(&id) {
                    Some(cover) => {
                        images.insert(id, cover.clone());
                    }
                    None => missing.push(id),
                }
            }
        }
        for id in missing {
            let Ok(answer) = self
                .get(&format!("/users/{}", escape::component(&id)))
                .await
            else {
                continue;
            };
            let Some(cover) = wire::image(&answer, &["avatar_url"], 300) else {
                continue;
            };
            lock(&self.inner.portraits).insert(id.clone(), cover.clone());
            images.insert(id, cover);
        }
        Ok(images)
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let user_id = self.user_id().await;
        let items = self
            .list(&format!(
                "/users/{}/track_likes?limit={PAGE}",
                escape::component(&user_id)
            ))
            .await
            .context("cannot load the liked tracks")?;
        // Each entry wraps the liked track under a `track` field.
        Ok(items
            .iter()
            .filter_map(|item| item.get("track"))
            .filter_map(wire::track)
            .collect())
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.like("tracks", track_id, saved).await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let data = self.track_data(track_id).await?;
        wire::track(&data).with_context(|| format!("cannot read the soundcloud track {track_id}"))
    }

    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>> {
        let data = self.track_data(track_id).await?;
        Ok(wire::number(&data, &["playback_count"]))
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let user_id = self.user_id().await;
        let items = self
            .list(&format!("/me/library/all?limit={PAGE}"))
            .await
            .context("cannot load the playlists")?;
        Ok(items
            .iter()
            .filter(|item| item.get("playlist").is_some())
            .filter_map(|item| wire::playlist(&item["playlist"], &user_id))
            .collect())
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let answer = self
            .write(
                reqwest::Method::POST,
                "/playlists",
                Some(json!({ "playlist": { "title": name, "sharing": "private" } })),
            )
            .await
            .context("cannot create the soundcloud playlist")?;
        answer
            .get("id")
            .and_then(wire::id)
            .or_else(|| {
                let urn = wire::text(&answer, &["urn"])?;
                urn.rsplit(':').next().map(str::to_owned)
            })
            .context("soundcloud answered the new playlist with no id")
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.update_playlist(playlist_id, json!({ "title": name }))
            .await
            .context("cannot rename the soundcloud playlist")
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let urn = self.urn("playlists", playlist_id).await?;
        self.write(reqwest::Method::DELETE, &format!("/playlists/{urn}"), None)
            .await
            .context("cannot delete the soundcloud playlist")
            .map(|_| ())
    }

    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        self.like("playlists", playlist_id, false).await
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        self.like("playlists", playlist_id, true).await
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        let sharing = match public {
            true => "public",
            false => "private",
        };
        self.update_playlist(playlist_id, json!({ "sharing": sharing }))
            .await
            .context("cannot change who sees the soundcloud playlist")
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let mut ids = self.track_ids(playlist_id).await?;
        ids.push(self.numeric(track_id).await?);
        self.set_tracks(playlist_id, &ids)
            .await
            .context("cannot add the track to the soundcloud playlist")
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let track = self.numeric(track_id).await?;
        let mut ids = self.track_ids(playlist_id).await?;
        ids.retain(|id| *id != track);
        self.set_tracks(playlist_id, &ids)
            .await
            .context("cannot remove the track from the soundcloud playlist")
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.like("playlists", album_id, saved).await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let user_id = self.user_id().await;
        let items = self
            .list(&format!(
                "/users/{}/followings?limit={PAGE}",
                escape::component(&user_id)
            ))
            .await
            .context("cannot load the followed artists")?;
        Ok(items.iter().filter_map(wire::saved_artist).collect())
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        if !self.writable() {
            return Err(crate::Unsupported.into());
        }
        let urn = self.urn("users", artist_id).await?;
        let method = match saved {
            true => reqwest::Method::PUT,
            false => reqwest::Method::DELETE,
        };
        self.write(method, &format!("/me/followings/{urn}"), None)
            .await
            .map(|_| ())
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let id = self.numeric(album_id).await?;
        let answer = self
            .get(&format!(
                "/playlists/{}?representation=full",
                escape::component(&id)
            ))
            .await
            .with_context(|| format!("cannot load the album {album_id}"))?;
        let album =
            wire::album(&answer).with_context(|| format!("cannot read the album {album_id}"))?;
        let tracks = self.tracks_from(&answer).await?;
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn album_catalogue(
        &self,
        _album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<crate::AlbumCatalogue> {
        let Some(artist_id) = artist_id else {
            return Ok(crate::AlbumCatalogue::default());
        };
        let id = self.numeric(artist_id).await?;
        let page = self
            .get(&format!(
                "/users/{}/albums?limit={}",
                escape::component(&id),
                SUGGESTIONS + 1
            ))
            .await
            .with_context(|| format!("cannot load more from artist {artist_id}"))?;
        let own = ArtistRef {
            name: String::new(),
            id: Some(id),
        };
        let mut seen = std::collections::HashSet::new();
        let also_like = wire::album_list(&page)
            .into_iter()
            .map(|album| album.credit(&own))
            .filter(|album| seen.insert(album.id.clone()))
            .take(SUGGESTIONS)
            .collect();
        Ok(crate::AlbumCatalogue {
            also_like,
            similar: Vec::new(),
        })
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let answer = self
            .playlist_data(playlist_id)
            .await
            .with_context(|| format!("cannot load the playlist {playlist_id}"))?;
        let user_id = self.user_id().await;
        let mut playlist = wire::playlist(&answer, &user_id)
            .with_context(|| format!("cannot read the playlist {playlist_id}"))?;
        let tracks = self.tracks_from(&answer).await?;
        if playlist.track_count == 0 {
            playlist.track_count = tracks.len() as u32;
        }
        if playlist.cover.is_none() {
            playlist.cover = tracks.iter().find_map(|track| track.cover.clone());
        }
        Ok(PlaylistDetail {
            playlist,
            tracks,
            continuation: None,
        })
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self.playlist(playlist_id).await?.tracks)
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = self.playlist_tracks(playlist_id).await?;
        Ok(distinct_covers(&tracks, wanted))
    }

    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let id = self.numeric(track_id).await?;
        let page = self
            .get(&format!(
                "/tracks/{}/related?limit=25",
                escape::component(&id)
            ))
            .await;
        match page {
            Ok(page) => Ok((wire::track_list(&page), None)),
            Err(error) => {
                log::warn!("soundcloud: no track radio for {track_id}: {error:#}");
                Ok((Vec::new(), None))
            }
        }
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let page = self
            .get(&format!(
                "/search/tracks?q={}&limit=50",
                escape::component(query)
            ))
            .await
            .context("cannot search soundcloud")?;
        Ok(wire::track_list(&page))
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let page = self
            .get(&format!(
                "/search/albums?q={}&limit=30",
                escape::component(query)
            ))
            .await
            .context("cannot search soundcloud albums")?;
        Ok(wire::album_list(&page))
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let page = self
            .get(&format!(
                "/search/playlists?q={}&limit=30",
                escape::component(query)
            ))
            .await
            .context("cannot search soundcloud playlists")?;
        Ok(wire::playlist_list(&page, ""))
    }

    async fn home(&self) -> Result<HomeFeed> {
        // What the account played lately, newest first.
        let listen_again = match self.get("/me/play-history/tracks?limit=30").await {
            Ok(page) => wire::collection(&page)
                .iter()
                .filter_map(|item| item.get("track"))
                .filter_map(wire::track)
                .map(GenreItem::Track)
                .collect(),
            Err(error) => {
                log::debug!("soundcloud: no play history: {error:#}");
                Vec::new()
            }
        };

        let mut sections = Vec::new();

        // "Made for you": each mixed selection is a shelf of playlists. The api repeats the
        // same mix within a shelf, so each playlist is shown once per shelf, by id.
        if let Ok(page) = self.get("/mixed-selections?variant_ids=&limit=10").await {
            for folder in wire::collection(&page) {
                let title = folder
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let mut seen: HashSet<String> = HashSet::new();
                let items: Vec<GenreItem> = folder
                    .get("items")
                    .and_then(|items| items.get("collection"))
                    .map(wire::collection)
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|item| wire::playlist(&item, "").map(GenreItem::Playlist))
                    .filter(|item| match item {
                        GenreItem::Playlist(playlist) => seen.insert(playlist.id.clone()),
                        _ => true,
                    })
                    .collect();
                if !items.is_empty() {
                    sections.push(GenreSection { title, items });
                }
            }
        }

        // The user's Stream: tracks from accounts they follow.
        if let Ok(page) = self.get("/stream?limit=30").await {
            let tracks: Vec<GenreItem> = wire::collection(&page)
                .iter()
                .filter(|item| {
                    matches!(
                        item.get("type").and_then(Value::as_str),
                        Some("track") | Some("track-repost")
                    )
                })
                .filter_map(|item| item.get("track"))
                .filter_map(wire::track)
                .map(GenreItem::Track)
                .collect();
            if !tracks.is_empty() {
                sections.push(GenreSection {
                    title: "Stream".to_owned(),
                    items: tracks,
                });
            }
        }

        Ok(HomeFeed {
            listen_again,
            quick_picks: None,
            sections,
        })
    }
}

/// Locks a mutex whose guarded value is still sound after a panic elsewhere.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The presets to try for a level, from its own down to the lowest.
fn presets_from(quality: Quality) -> &'static [&'static str] {
    let first = match quality.within(QUALITIES) {
        Quality::Normal | Quality::High | Quality::Lossless | Quality::HiRes => 0,
        Quality::Low => 1,
    };
    &FORMATS[first..]
}

/// The preset a track is first asked for at `quality`, which is also the vault variant its
/// bytes are kept under.
pub(crate) fn served(quality: Quality) -> &'static str {
    presets_from(quality)[0]
}

/// The transcoding's own url, signed with the web app's `client_id` and the track's
/// `track_authorization`. Asking it answers the playlist url the audio streams from.
fn transcoding_url(media_url: &str, client_id: &str, authorization: &str) -> String {
    let separator = match media_url.contains('?') {
        true => '&',
        false => '?',
    };
    format!(
        "{media_url}{separator}client_id={}&track_authorization={}",
        escape::component(client_id),
        escape::component(authorization)
    )
}

/// The `error_message` a refused answer carries, when it is json.
fn json_text(text: &str) -> Option<String> {
    let answer: Value = serde_json::from_str(text).ok()?;
    answer
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
        .and_then(|error| error.get("error_message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// The web app's `client_id`, extracted from its script bundle, with the last known value as a
/// fallback so a bundle redesign degrades to a warning instead of killing every request.
async fn client_id(http: &reqwest::Client) -> String {
    match fetch_client_id(http).await {
        Ok(id) => id,
        Err(error) => {
            log::warn!(
                "soundcloud: cannot extract the client id ({error:#}); using the last known one"
            );
            FALLBACK_CLIENT_ID.to_owned()
        }
    }
}

async fn fetch_client_id(http: &reqwest::Client) -> Result<String> {
    let html = http
        .get(WEB)
        .send()
        .await
        .context("cannot reach soundcloud")?
        .error_for_status()
        .context("soundcloud refused the visit")?
        .text()
        .await
        .context("cannot read the soundcloud page")?;

    if let Some(id) = find_client_id(&html) {
        return Ok(id);
    }
    let bundle = find_bundle(&html).context("the page names no script bundle")?;
    let source = http
        .get(bundle.as_str())
        .send()
        .await
        .context("cannot fetch the soundcloud bundle")?
        .error_for_status()
        .context("the soundcloud bundle refused the visit")?
        .text()
        .await
        .context("cannot read the soundcloud bundle")?;
    find_client_id(&source).context("the bundle holds no client id")
}

/// A 32-character alphanumeric run after a `client_id` marker, without pulling in a regex
/// engine. The marker is the id's own name wherever the api is called from.
fn find_client_id(source: &str) -> Option<String> {
    for marker in ["client_id\":\"", "client_id=", "client_id\": \""] {
        if let Some(id) = after_marker(source, marker) {
            return Some(id);
        }
    }
    None
}

/// The next 32 alphanumeric characters after `marker`.
fn after_marker(source: &str, marker: &str) -> Option<String> {
    let rest = source.split_once(marker)?.1;
    let head = &rest[..rest.len().min(64)];
    let id: String = head
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (id.len() == 32).then_some(id)
}

/// The first `app-` or `assets` script url in the page, which carries the client id.
fn find_bundle(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(at) = rest.find(".js") {
        let tail = &rest[..at];
        let Some(from) = tail.rfind("https://") else {
            rest = &rest[at + 3..];
            continue;
        };
        let url = &rest[from..at + 3];
        if url.contains("app") || url.contains("assets") {
            return Some(url.to_owned());
        }
        rest = &rest[at + 3..];
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::soundcloud::mock::Mock;

    /// One track the way api-v2 lists it.
    const TRACK: &str = r#"{
        "id": 1001,
        "kind": "track",
        "title": "Night Drive",
        "duration": 215000,
        "streamable": true,
        "policy": "ALLOW",
        "artwork_url": "https://i1.sndcdn.com/artworks-abc-large.jpg",
        "user": {"id": 77, "username": "Lumen"},
        "playback_count": 12,
        "media": {"transcodings": [{
            "url": "https://api-v2.soundcloud.com/media/soundcloud:tracks:1001/abc/stream/hls",
            "preset": "aac_160k",
            "format": {"protocol": "hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}
        }]}
    }"#;

    /// A client whose every host is `mock`, signed in as user 42. `app` makes it writable.
    async fn client(mock: &Mock, app: Option<App>) -> SoundCloudClient {
        let client = SoundCloudClient::build(
            Mock::http(),
            Grant::lasting("2-token"),
            app,
            "web-client".to_owned(),
            mock.base(),
            &mock.url("/public"),
            &mock.url("/oauth/token"),
        );
        client.inner.profile.write().await.id = "42".to_owned();
        client
    }

    fn app() -> App {
        App {
            client_id: "app-id".to_owned(),
            secret: "app-secret".to_owned(),
            redirect_uri: "http://127.0.0.1:8991/soundcloud".to_owned(),
        }
    }

    #[tokio::test]
    async fn lists_liked_tracks_from_a_mocked_response() {
        let mock = Mock::start(|line| match line.contains("/users/42/track_likes") {
            true => (
                200,
                format!(r#"{{"collection": [{{"created_at": "2026-05-01T00:00:00Z", "track": {TRACK}}}], "next_href": null}}"#),
            ),
            false => (404, r#"{"error_message": "not here"}"#.to_owned()),
        })
        .await;
        let tracks = client(&mock, None).await.saved_tracks().await.unwrap();

        assert_eq!(tracks.len(), 1);
        let track = &tracks[0];
        assert_eq!(track.id.as_deref(), Some("1001"));
        assert_eq!(track.name, "Night Drive");
        assert_eq!(track.artists, "Lumen");
        assert_eq!(track.duration, Duration::from_millis(215_000));
        assert!(track.playable);
        assert_eq!(
            track.cover.as_deref(),
            Some("https://i1.sndcdn.com/artworks-abc-t300x300.jpg")
        );

        let sent = mock.requests();
        assert!(sent[0].line.contains("client_id=web-client"));
        assert!(
            sent[0]
                .headers
                .to_ascii_lowercase()
                .contains("authorization: oauth 2-token")
        );
    }

    #[tokio::test]
    async fn follows_next_href_through_every_page() {
        // The route only learns the server's address once it is up, so it reads it from here.
        let base = Arc::new(std::sync::OnceLock::<String>::new());
        let known = base.clone();
        let mock = Mock::start(move |line| match line.contains("page=2") {
            true => (200, format!(r#"{{"collection": [{{"track": {TRACK}}}]}}"#)),
            false => (
                200,
                format!(
                    r#"{{"collection": [{{"track": {TRACK}}}], "next_href": "{}/users/42/track_likes?page=2"}}"#,
                    known.get().unwrap()
                ),
            ),
        })
        .await;
        base.set(mock.base().to_owned()).unwrap();
        let tracks = client(&mock, None).await.saved_tracks().await.unwrap();

        assert_eq!(tracks.len(), 2);
        let lines: Vec<String> = mock.requests().into_iter().map(|sent| sent.line).collect();
        assert_eq!(lines.len(), 2);
        assert!(
            lines[1].starts_with("GET /users/42/track_likes?page=2 "),
            "{}",
            lines[1]
        );
    }

    #[tokio::test]
    async fn searches_a_mocked_catalogue() {
        let mock = Mock::start(|line| match line.starts_with("GET /search/tracks?") {
            true => (
                200,
                format!(r#"{{"collection": [{TRACK}], "total_results": 1}}"#),
            ),
            false => (404, String::new()),
        })
        .await;
        let found = client(&mock, None)
            .await
            .search("night drive & co")
            .await
            .unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Night Drive");
        let line = &mock.requests()[0].line;
        assert!(
            line.contains("q=night%20drive%20%26%20co&limit=50&client_id=web-client"),
            "{line}"
        );
    }

    #[tokio::test]
    async fn a_refused_search_names_soundcloud_s_reason() {
        let mock = Mock::start(|_| {
            (
                401,
                r#"{"errors": [{"error_message": "invalid token"}]}"#.to_owned(),
            )
        })
        .await;
        let error = client(&mock, None).await.search("x").await.unwrap_err();
        assert!(format!("{error:#}").contains("invalid token"), "{error:#}");
    }

    #[test]
    fn the_stream_url_carries_the_client_id_and_authorization() {
        let url = transcoding_url(
            "https://api-v2.soundcloud.com/media/soundcloud:tracks:1001/abc/stream/hls",
            "web-client",
            "eyJ.a+b/c=",
        );
        assert_eq!(
            url,
            "https://api-v2.soundcloud.com/media/soundcloud:tracks:1001/abc/stream/hls\
             ?client_id=web-client&track_authorization=eyJ.a%2Bb%2Fc%3D"
        );
        let signed = transcoding_url("https://example.com/hls?x=1", "id", "auth");
        assert_eq!(
            signed,
            "https://example.com/hls?x=1&client_id=id&track_authorization=auth"
        );
    }

    #[tokio::test]
    async fn resolves_a_transcoding_to_its_playlist() {
        let mock = Mock::start(|line| match line {
            line if line.starts_with("GET /tracks?ids=1001") => {
                let track = TRACK.replace("https://api-v2.soundcloud.com/media", "MEDIA");
                (
                    200,
                    track.replacen('{', r#"{"track_authorization": "auth-1", "#, 1),
                )
            }
            line if line.starts_with("GET /media/") => (
                200,
                r#"{"url": "https://cf-hls-media.sndcdn.com/playlist/abc.m3u8?Policy=p"}"#
                    .to_owned(),
            ),
            _ => (404, String::new()),
        })
        .await;
        let base = mock.base().to_owned();
        // The fixture's transcoding url is pointed back at the mock.
        let mock_media = format!("{base}/media");
        let client = client(&mock, None).await;
        let data = client.track_data("1001").await.unwrap();
        let media = data["media"]["transcodings"][0]["url"]
            .as_str()
            .unwrap()
            .replace("MEDIA", &mock_media);
        let url = transcoding_url(&media, "web-client", "auth-1");
        let m3u8 = client.stream_url(&url).await.unwrap();

        assert_eq!(
            m3u8,
            "https://cf-hls-media.sndcdn.com/playlist/abc.m3u8?Policy=p"
        );
        let sent = mock.requests();
        let media_line = &sent.last().unwrap().line;
        assert!(
            media_line.starts_with(
                "GET /media/soundcloud:tracks:1001/abc/stream/hls?client_id=web-client&track_authorization=auth-1 "
            ),
            "{media_line}"
        );
    }

    #[tokio::test]
    async fn a_cookie_sign_in_cannot_write() {
        let mock = Mock::start(|_| (200, "{}".to_owned())).await;
        let error = client(&mock, None)
            .await
            .set_track_saved("1001", true)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<crate::Unsupported>().is_some());
        assert!(mock.requests().is_empty());
    }

    #[tokio::test]
    async fn an_app_sign_in_likes_and_follows_over_the_public_api() {
        let mock = Mock::start(|_| (200, r#"{"status": "200 - OK"}"#.to_owned())).await;
        let client = client(&mock, Some(app())).await;
        client.set_track_saved("1001", true).await.unwrap();
        client.set_track_saved("1001", false).await.unwrap();
        client.set_artist_saved("77", true).await.unwrap();
        client
            .add_playlist_to_library("soundcloud:system-playlists:weekly")
            .await
            .unwrap();

        let lines: Vec<String> = mock
            .requests()
            .into_iter()
            .map(|sent| sent.line.rsplit_once(' ').unwrap().0.to_owned())
            .collect();
        assert_eq!(
            lines,
            [
                "POST /public/likes/tracks/soundcloud:tracks:1001",
                "DELETE /public/likes/tracks/soundcloud:tracks:1001",
                "PUT /public/me/followings/soundcloud:users:77",
                "POST /public/likes/playlists/soundcloud:system-playlists:weekly",
            ]
        );
    }

    #[tokio::test]
    async fn adding_a_track_sends_the_whole_playlist_back() {
        let mock = Mock::start(|line| match line {
            line if line.starts_with("GET /playlists/9?") => (
                200,
                r#"{"id": 9, "title": "Mix", "tracks": [{"id": 1, "kind": "track"}, {"id": 2, "kind": "track"}]}"#
                    .to_owned(),
            ),
            line if line.starts_with("PUT /public/playlists/soundcloud:playlists:9 ") => {
                (200, r#"{"id": 9}"#.to_owned())
            }
            _ => (404, String::new()),
        })
        .await;
        client(&mock, Some(app()))
            .await
            .add_track_to_playlist("9", "1001")
            .await
            .unwrap();

        let sent = mock.requests();
        let body: Value = serde_json::from_str(&sent.last().unwrap().body).unwrap();
        assert_eq!(
            body,
            json!({ "playlist": { "tracks": [
                { "urn": "soundcloud:tracks:1" },
                { "urn": "soundcloud:tracks:2" },
                { "urn": "soundcloud:tracks:1001" },
            ] } })
        );
    }

    #[tokio::test]
    async fn a_lapsing_grant_is_renewed_before_the_request() {
        let mock = Mock::start(|line| match line {
            line if line.starts_with("POST /oauth/token") => (
                200,
                r#"{"access_token": "2-renewed", "refresh_token": "r-2", "expires_in": 3600}"#
                    .to_owned(),
            ),
            _ => (200, format!(r#"{{"collection": [{TRACK}]}}"#)),
        })
        .await;
        let client = client(&mock, Some(app())).await;
        *client.inner.grant.lock().await = Grant {
            access: "2-old".to_owned(),
            refresh: Some("r-1".to_owned()),
            expires_at: Some(oauth::now()),
        };
        client.search("x").await.unwrap();

        let sent = mock.requests();
        assert!(sent[0].line.starts_with("POST /oauth/token"));
        assert!(sent[0].body.contains("refresh_token=r-1"));
        assert!(
            sent[1]
                .headers
                .to_ascii_lowercase()
                .contains("authorization: oauth 2-renewed")
        );
        assert_eq!(client.inner.grant.lock().await.access, "2-renewed");
    }
}
