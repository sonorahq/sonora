use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use reqwest::StatusCode;
use reqwest::header::AUTHORIZATION;
use serde::de::DeserializeOwned;

use crate::engine::Loudness;
use crate::escape;
use crate::jellyfin::auth::{self, Credentials};
use crate::jellyfin::wire::{self, Covers, Item, Items};
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, MediaKind, MusicApi, Page, Pages, Playlist,
    PlaylistDetail, SavedArtist, SignInFailure, SignInProblem, Track, UserProfile, distinct_covers,
};

/// How many items one library request asks for, so a large library arrives in pages.
const LIBRARY_PAGE: usize = 500;
/// How many tracks, albums and playlists a search lists.
const SEARCH_TRACKS: usize = 50;
const SEARCH_OTHERS: usize = 30;
/// The fields every listing asks for beyond what Jellyfin sends anyway.
const FIELDS: &str = "ChildCount,DateCreated";
/// The fields an artist page asks for, which add the biography.
const ARTIST_FIELDS: &str = "ChildCount,DateCreated,Overview";
/// What a refused write says. The first version only reads the library.
const READ_ONLY: &str = "jellyfin is read-only in this version of sonora";

/// A signed-in Jellyfin account. Every request carries the `MediaBrowser` authorization header
/// with the access token, and none puts the token in a url.
#[derive(Clone)]
pub struct JellyfinClient {
    http: reqwest::Client,
    /// The server address with no trailing slash.
    server: String,
    user_id: String,
    username: String,
    /// The whole `Authorization` header, token included.
    header: String,
    covers: Covers,
}

/// What the server records about a track that playback wants before the decoder can tell.
#[derive(Clone, Copy, Debug, Default)]
pub struct Details {
    pub duration: Option<Duration>,
    pub loudness: Option<Loudness>,
}

/// One `GET /Users/{userId}/Items` request, as its query parameters.
#[derive(Clone, Debug, Default)]
pub(crate) struct Query(Vec<(&'static str, String)>);

impl Query {
    /// Items of one kind, from anywhere in the user's libraries.
    fn kind(kind: &str) -> Self {
        Self(vec![
            ("IncludeItemTypes", kind.to_owned()),
            ("Recursive", "true".to_owned()),
            ("Fields", FIELDS.to_owned()),
        ])
    }

    /// Whatever sits directly under `parent`: an album's tracks or a playlist's entries.
    fn children(parent: &str) -> Self {
        Self(vec![
            ("ParentId", parent.to_owned()),
            ("Fields", FIELDS.to_owned()),
        ])
    }

    /// The items with these ids.
    fn ids(ids: &str) -> Self {
        Self(vec![("Ids", ids.to_owned()), ("Fields", FIELDS.to_owned())])
    }

    fn with(mut self, key: &'static str, value: impl Into<String>) -> Self {
        self.0.retain(|(kept, _)| *kept != key);
        self.0.push((key, value.into()));
        self
    }

    fn sorted(self, by: &str) -> Self {
        self.with("SortBy", by).with("SortOrder", "Ascending")
    }

    fn page(self, start: usize, limit: usize) -> Self {
        self.with("StartIndex", start.to_string())
            .with("Limit", limit.to_string())
    }
}

/// Signs in with a username and password by `POST /Users/AuthenticateByName`, and answers what
/// the account is to be kept as. A refused password says so plainly, apart from an unreachable
/// server, so the sign-in page can tell the two apart.
pub async fn authenticate(
    server: &str,
    username: &str,
    password: &str,
    device: &str,
    device_id: &str,
) -> Result<Credentials> {
    let response = reqwest::Client::new()
        .post(format!("{server}/Users/AuthenticateByName"))
        .header(AUTHORIZATION, auth::header(device, device_id, None))
        .json(&serde_json::json!({ "Username": username, "Pw": password }))
        .send()
        .await
        .map_err(lost)?;
    let status = response.status();
    log::info!("jellyfin: the sign-in answered {status}");
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        anyhow::bail!("the jellyfin server did not accept that username and password");
    }
    if !status.is_success() {
        anyhow::bail!("the jellyfin server answered the sign-in with {status}");
    }
    let answer: wire::Authentication = response
        .json()
        .await
        .context("the jellyfin sign-in answered something other than an account")?;
    let user = answer.user.unwrap_or_default();
    let (Some(user_id), Some(token)) = (
        user.id.filter(|id| !id.is_empty()),
        answer.access_token.filter(|token| !token.is_empty()),
    ) else {
        anyhow::bail!("the jellyfin sign-in answered without a user id or access token");
    };
    Ok(Credentials {
        server: server.to_owned(),
        user_id,
        username: user.name.unwrap_or_else(|| username.to_owned()),
        token,
        device_id: device_id.to_owned(),
    })
}

impl JellyfinClient {
    pub fn new(credentials: &Credentials) -> Self {
        let server = credentials.server.trim_end_matches('/').to_owned();
        Self {
            http: reqwest::Client::new(),
            covers: Covers::new(&server),
            server,
            user_id: credentials.user_id.clone(),
            username: credentials.username.clone(),
            header: auth::header(
                &auth::device_name(),
                &credentials.device_id,
                Some(&credentials.token),
            ),
        }
    }

    /// The account and server this client plays from. A track id only means something on its
    /// own server, so this tells apart two servers' tracks that share one.
    pub fn identity(&self) -> String {
        format!("{}@{}", self.user_id, self.server)
    }

    /// Where a track's original file streams from: `GET /Audio/{itemId}/stream` with
    /// `static=true`, so the server sends the file as it is rather than converting it. The token
    /// goes in the header that `open_stream` sends, never in this url.
    pub fn stream_url(&self, track_id: &str) -> String {
        format!(
            "{}/Audio/{}/stream?static=true",
            self.server,
            escape::component(track_id)
        )
    }

    /// Opens the audio of a track and answers once the response headers are in. The body is
    /// still on its way.
    pub async fn open_stream(&self, track_id: &str) -> Result<reqwest::Response> {
        let response = self
            .http
            .get(self.stream_url(track_id))
            .header(AUTHORIZATION, &self.header)
            .send()
            .await
            .map_err(lost)?;
        checked(response, "the stream")
    }

    /// The length and loudness the server records for a track. Jellyfin's normalization gain is
    /// measured against -18 LUFS, the same reference as ReplayGain, so it reads as one.
    pub async fn details(&self, track_id: &str) -> Details {
        let Ok(Some(item)) = self.item(track_id).await else {
            return Details::default();
        };
        Details {
            duration: item
                .run_time_ticks
                .map(|ticks| Duration::from_micros(ticks / 10)),
            loudness: item
                .normalization_gain
                .map(|gain| Loudness::replay_gain(gain, None)),
        }
    }

    /// Sends a `GET` with the token and reads the json it answers with.
    async fn get<T: DeserializeOwned>(&self, path: &str, query: &Query) -> Result<T> {
        let response = self
            .http
            .get(format!("{}{path}", self.server))
            .query(&query.0)
            .header(AUTHORIZATION, &self.header)
            .send()
            .await
            .map_err(lost)?;
        checked(response, path)?
            .json()
            .await
            .with_context(|| format!("cannot read what jellyfin answered for {path}"))
    }

    /// One page of `GET /Users/{userId}/Items`.
    pub(crate) async fn items(&self, query: &Query) -> Result<Items> {
        let path = format!("/Users/{}/Items", escape::component(&self.user_id));
        self.get(&path, query).await
    }

    /// Every item a query matches, a page at a time, as they arrive. The first page tells how
    /// many there are, and the listing ends at that count or at the first short page.
    pub(crate) fn pages<T, F>(&self, query: Query, convert: F) -> Pages<T>
    where
        T: Send + 'static,
        F: Fn(&Covers, Item) -> Option<T> + Send + 'static,
    {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let client = self.clone();
        tokio::spawn(async move {
            let mut start = 0;
            loop {
                let page = client.items(&query.clone().page(start, LIBRARY_PAGE)).await;
                let page = match page {
                    Ok(page) => page,
                    Err(error) => {
                        sender.send(Err(error)).await.ok();
                        return;
                    }
                };
                let fetched = page.items.len();
                let total = page.total_record_count;
                let items = page
                    .items
                    .into_iter()
                    .filter_map(|item| convert(&client.covers, item))
                    .collect();
                if sender.send(Ok(Page { total, items })).await.is_err() {
                    return;
                }
                start += fetched;
                let done = fetched < LIBRARY_PAGE || total.is_some_and(|total| start >= total);
                if done {
                    return;
                }
            }
        });
        receiver
    }

    /// Every item a query matches, gathered from all of its pages.
    async fn all<T, F>(&self, query: Query, convert: F) -> Result<Vec<T>>
    where
        T: Send + 'static,
        F: Fn(&Covers, Item) -> Option<T> + Send + 'static,
    {
        let mut pages = self.pages(query, convert);
        let mut all = Vec::new();
        while let Some(page) = pages.recv().await {
            all.extend(page?.items);
        }
        Ok(all)
    }

    /// One item by its id, or `None` when the server knows no such item for this user.
    async fn item(&self, id: &str) -> Result<Option<Item>> {
        Ok(self.items(&Query::ids(id)).await?.items.into_iter().next())
    }

    /// The tracks directly under an album or playlist, in the order the server keeps them, or
    /// in disc and track order when `sort` asks for it.
    async fn tracks_under(&self, parent: &str, sort: bool) -> Result<Vec<Track>> {
        let query = match sort {
            true => Query::children(parent).sorted("ParentIndexNumber,IndexNumber,SortName"),
            false => Query::children(parent),
        };
        self.all(query, |covers, item| {
            item.is_audio().then(|| covers.track(item))
        })
        .await
    }

    /// Items of one kind whose names match `query`, by the server's own search.
    async fn search_kind(&self, kind: &str, query: &str, limit: usize) -> Result<Vec<Item>> {
        let found = self
            .items(
                &Query::kind(kind)
                    .with("SearchTerm", query)
                    .with("Limit", limit.to_string()),
            )
            .await
            .with_context(|| format!("cannot search jellyfin for {}", kind.to_lowercase()))?;
        Ok(found.items)
    }
}

#[async_trait]
impl MusicApi for JellyfinClient {
    fn share_url(&self, _kind: MediaKind, _id: &str) -> Option<String> {
        None
    }

    /// Asks the server who the token belongs to, which is also how a stored sign-in is checked.
    async fn profile(&self) -> Result<UserProfile> {
        let user: wire::User = self
            .get("/Users/Me", &Query::default())
            .await
            .context("cannot reach the jellyfin account")?;
        let name = user.name.unwrap_or_else(|| self.username.clone());
        Ok(wire::profile(&self.user_id, &name))
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let artist = self
            .items(&Query::ids(artist_id).with("Fields", ARTIST_FIELDS))
            .await
            .with_context(|| format!("cannot load the artist {artist_id}"))?
            .items
            .into_iter()
            .next()
            .with_context(|| format!("jellyfin has no artist {artist_id}"))?;
        let albums = self
            .all(
                Query::kind("MusicAlbum")
                    .with("ArtistIds", artist_id)
                    .sorted("ProductionYear,SortName"),
                |covers, item| Some(covers.album(item)),
            )
            .await
            .with_context(|| format!("cannot load the albums of {artist_id}"))?;
        Ok(Artist {
            cover_large: self.covers.artist_large(&artist),
            biography: artist.overview.clone().filter(|bio| !bio.trim().is_empty()),
            name: artist.name.unwrap_or_default(),
            banner: None,
            monthly_listeners: None,
            top_tracks: Vec::new(),
            albums,
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let artist = self
            .items(&Query::ids(artist_id).with("Fields", ARTIST_FIELDS))
            .await
            .with_context(|| format!("cannot load the artist {artist_id}"))?
            .items
            .into_iter()
            .next()
            .with_context(|| format!("jellyfin has no artist {artist_id}"))?;
        Ok(ArtistProfile {
            cover_large: self.covers.artist_large(&artist),
            biography: artist.overview.filter(|bio| !bio.trim().is_empty()),
            name: artist.name.unwrap_or_default(),
        })
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let found = self
            .items(&Query::ids(&ids.join(",")))
            .await
            .context("cannot load the artist pictures")?;
        Ok(found
            .items
            .into_iter()
            .filter_map(|item| {
                let artist = self.covers.artist(item);
                Some((artist.id, artist.cover?))
            })
            .collect())
    }

    /// Favorites are left for a later version, so there are none to draw.
    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn all_tracks(&self) -> Result<Vec<Track>> {
        self.all(
            Query::kind("Audio").sorted("AlbumArtist,Album,ParentIndexNumber,IndexNumber"),
            |covers, item| Some(covers.track(item)),
        )
        .await
        .context("cannot load the jellyfin songs")
    }

    async fn all_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(self.pages(
            Query::kind("Audio").sorted("AlbumArtist,Album,ParentIndexNumber,IndexNumber"),
            |covers, item| Some(covers.track(item)),
        ))
    }

    async fn set_track_saved(&self, _track_id: &str, _saved: bool) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let item = self
            .item(track_id)
            .await
            .with_context(|| format!("cannot load the song {track_id}"))?
            .with_context(|| format!("jellyfin has no song {track_id}"))?;
        Ok(self.covers.track(item))
    }

    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>> {
        Ok(self.track(track_id).await?.playcount)
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.all(
            Query::kind("Playlist")
                .with("MediaTypes", "Audio")
                .sorted("SortName"),
            |covers, item| Some(covers.playlist(item)),
        )
        .await
        .context("cannot load the jellyfin playlists")
    }

    async fn create_playlist(&self, _name: &str) -> Result<String> {
        anyhow::bail!(READ_ONLY)
    }

    async fn rename_playlist(&self, _playlist_id: &str, _name: &str) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn delete_playlist(&self, _playlist_id: &str) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn remove_playlist_from_library(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    async fn add_playlist_to_library(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    async fn set_playlist_public(&self, _playlist_id: &str, _public: bool) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn add_track_to_playlist(&self, _playlist_id: &str, _track_id: &str) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn remove_track_from_playlist(&self, _playlist_id: &str, _track_id: &str) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn all_albums(&self) -> Result<Vec<Album>> {
        self.all(
            Query::kind("MusicAlbum").sorted("SortName"),
            |covers, item| Some(covers.album(item)),
        )
        .await
        .context("cannot load the jellyfin albums")
    }

    async fn all_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(self.pages(
            Query::kind("MusicAlbum").sorted("SortName"),
            |covers, item| Some(covers.album(item)),
        ))
    }

    async fn set_album_saved(&self, _album_id: &str, _saved: bool) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        Ok(Vec::new())
    }

    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        self.all(
            Query::kind("MusicArtist").sorted("SortName"),
            |covers, item| Some(covers.artist(item)),
        )
        .await
        .context("cannot load the jellyfin artists")
    }

    async fn all_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(self.pages(
            Query::kind("MusicArtist").sorted("SortName"),
            |covers, item| Some(covers.artist(item)),
        ))
    }

    async fn set_artist_saved(&self, _artist_id: &str, _saved: bool) -> Result<()> {
        anyhow::bail!(READ_ONLY)
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let (album, tracks) = tokio::join!(self.item(album_id), self.tracks_under(album_id, true));
        let album = album
            .with_context(|| format!("cannot load the album {album_id}"))?
            .with_context(|| format!("jellyfin has no album {album_id}"))?;
        let tracks = tracks.with_context(|| format!("cannot load the songs of {album_id}"))?;
        let mut album = self.covers.album(album);
        if album.track_count == 0 {
            album.track_count = tracks.len() as u32;
        }
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        self.tracks_under(album_id, true)
            .await
            .with_context(|| format!("cannot load the songs of {album_id}"))
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let (playlist, tracks) = tokio::join!(
            self.item(playlist_id),
            self.tracks_under(playlist_id, false)
        );
        let playlist = playlist
            .with_context(|| format!("cannot load the playlist {playlist_id}"))?
            .with_context(|| format!("jellyfin has no playlist {playlist_id}"))?;
        let tracks = tracks.with_context(|| format!("cannot load the songs of {playlist_id}"))?;
        let mut playlist = self.covers.playlist(playlist);
        playlist.track_count = tracks.len() as u32;
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
        self.tracks_under(playlist_id, false)
            .await
            .with_context(|| format!("cannot load the songs of {playlist_id}"))
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = self.playlist_tracks(playlist_id).await?;
        Ok(distinct_covers(&tracks, wanted))
    }

    /// Stations are left for a later version, and the capability is off, so this is never asked.
    async fn track_radio(
        &self,
        _track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        Ok((Vec::new(), None))
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let found = self.search_kind("Audio", query, SEARCH_TRACKS).await?;
        Ok(found
            .into_iter()
            .map(|item| self.covers.track(item))
            .collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let found = self.search_kind("MusicAlbum", query, SEARCH_OTHERS).await?;
        Ok(found
            .into_iter()
            .map(|item| self.covers.album(item))
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let found = self.search_kind("Playlist", query, SEARCH_OTHERS).await?;
        Ok(found
            .into_iter()
            .map(|item| self.covers.playlist(item))
            .collect())
    }
}

/// Passes a successful response on and turns any other into an error. A 401 is the server
/// refusing the token, which the session reads as a sign-in to do again. Only the status is
/// logged, never the request's headers.
fn checked(response: reqwest::Response, what: &str) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    log::warn!("jellyfin: {what} answered {status}");
    match status {
        StatusCode::UNAUTHORIZED => Err(anyhow::Error::new(SignInFailure(
            SignInProblem::Credentials,
        ))
        .context("the jellyfin server asked to sign in again")),
        _ => Err(anyhow::anyhow!("the jellyfin server answered {status}")),
    }
}

/// The error for a request that never reached the server. Its wording is one the session
/// reads as a lost connection, which the app offers to try again.
fn lost(error: reqwest::Error) -> anyhow::Error {
    anyhow::Error::new(error).context("could not reach the jellyfin server")
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    /// One request the mock server saw.
    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        path: String,
        query: HashMap<String, String>,
        authorization: String,
        body: String,
    }

    type Route = dyn Fn(&Seen) -> (u16, String) + Send + Sync;

    /// A local HTTP server that answers every request through `route` and remembers it.
    struct Mock {
        base: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Mock {
        async fn start(route: impl Fn(&Seen) -> (u16, String) + Send + Sync + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let route: Arc<Route> = Arc::new(route);
            let kept = seen.clone();
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let route = route.clone();
                    let kept = kept.clone();
                    tokio::spawn(async move {
                        let Some(request) = read(&mut socket).await else {
                            return;
                        };
                        let (status, body) = route(&request);
                        kept.lock().unwrap().push(request);
                        let answer = format!(
                            "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                             content-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        socket.write_all(answer.as_bytes()).await.ok();
                        socket.shutdown().await.ok();
                    });
                }
            });
            Self { base, seen }
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        fn client(&self) -> JellyfinClient {
            JellyfinClient::new(&Credentials {
                server: self.base.clone(),
                user_id: "user-1".to_owned(),
                username: "liam".to_owned(),
                token: "secret-token".to_owned(),
                device_id: "sonora-test".to_owned(),
            })
        }
    }

    async fn read(socket: &mut tokio::net::TcpStream) -> Option<Seen> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 4096];
        let end = loop {
            let read = socket.read(&mut buffer).await.ok()?;
            if read == 0 {
                return None;
            }
            bytes.extend_from_slice(&buffer[..read]);
            if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
        let mut lines = head.lines();
        let mut start = lines.next()?.split(' ');
        let method = start.next()?.to_owned();
        let target = start.next()?.to_owned();
        let headers: HashMap<String, String> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
            .collect();
        let length: usize = headers
            .get("content-length")
            .and_then(|length| length.parse().ok())
            .unwrap_or(0);
        while bytes.len() < end + length {
            let read = socket.read(&mut buffer).await.ok()?;
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        let (path, query) = target.split_once('?').unwrap_or((&target, ""));
        Some(Seen {
            method,
            path: path.to_owned(),
            query: query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .map(|(key, value)| (decode(key), decode(value)))
                .collect(),
            authorization: headers.get("authorization").cloned().unwrap_or_default(),
            body: String::from_utf8_lossy(&bytes[end..]).into_owned(),
        })
    }

    fn decode(value: &str) -> String {
        percent_encoding::percent_decode_str(&value.replace('+', " "))
            .decode_utf8_lossy()
            .into_owned()
    }

    /// reqwest wants a rustls provider chosen before it builds a client.
    fn crypto() {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    }

    fn items(items: serde_json::Value, total: usize) -> String {
        serde_json::json!({ "Items": items, "TotalRecordCount": total, "StartIndex": 0 })
            .to_string()
    }

    #[tokio::test]
    async fn sign_in_posts_the_name_and_password_and_keeps_the_token() {
        crypto();
        let mock = Mock::start(|_| {
            (
                200,
                r#"{"User":{"Id":"user-1","Name":"Liam"},"AccessToken":"tok-1","ServerId":"s"}"#
                    .to_owned(),
            )
        })
        .await;

        let credentials = authenticate(&mock.base, "liam", "hunter2", "desk", "sonora-1")
            .await
            .unwrap();

        assert_eq!(credentials.user_id, "user-1");
        assert_eq!(credentials.username, "Liam");
        assert_eq!(credentials.token, "tok-1");
        assert_eq!(credentials.device_id, "sonora-1");
        assert_eq!(credentials.server, mock.base);
        let seen = mock.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "POST");
        assert_eq!(seen[0].path, "/Users/AuthenticateByName");
        let body: serde_json::Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({ "Username": "liam", "Pw": "hunter2" })
        );
        assert!(
            seen[0]
                .authorization
                .starts_with("MediaBrowser Client=\"Sonora\", Device=\"desk\"")
        );
        assert!(seen[0].authorization.contains("DeviceId=\"sonora-1\""));
        assert!(!seen[0].authorization.contains("Token="));
    }

    #[tokio::test]
    async fn refused_sign_in_says_the_password_was_not_accepted() {
        crypto();
        let mock = Mock::start(|_| (401, String::new())).await;

        let error = authenticate(&mock.base, "liam", "wrong", "desk", "sonora-1")
            .await
            .unwrap_err();

        let reason = format!("{error:#}");
        assert_eq!(
            reason,
            "the jellyfin server did not accept that username and password"
        );
        assert!(!crate::trouble::offline(&reason));
    }

    #[tokio::test]
    async fn sign_in_without_a_token_fails_instead_of_keeping_nothing() {
        crypto();
        let mock = Mock::start(|_| (200, r#"{"User":{"Id":"user-1"}}"#.to_owned())).await;

        let error = authenticate(&mock.base, "liam", "pw", "desk", "sonora-1")
            .await
            .unwrap_err();

        assert!(format!("{error:#}").contains("without a user id or access token"));
    }

    #[tokio::test]
    async fn unreachable_server_reads_as_a_lost_connection() {
        crypto();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);

        let error = authenticate(&base, "liam", "pw", "desk", "sonora-1")
            .await
            .unwrap_err();

        assert!(crate::trouble::offline(&format!("{error:#}")));
    }

    #[tokio::test]
    async fn expired_token_asks_to_sign_in_again() {
        crypto();
        let mock = Mock::start(|_| (401, String::new())).await;

        let error = mock.client().all_albums().await.unwrap_err();

        assert_eq!(
            error.downcast_ref::<SignInFailure>(),
            Some(&SignInFailure(SignInProblem::Credentials))
        );
    }

    #[tokio::test]
    async fn lists_artists_with_the_token_and_their_pictures() {
        crypto();
        let mock = Mock::start(|_| {
            (
                200,
                items(
                    serde_json::json!([
                        { "Id": "a1", "Name": "Björk", "Type": "MusicArtist",
                          "ImageTags": { "Primary": "t1" } },
                        { "Id": "a2", "Name": "Sigur Rós", "Type": "MusicArtist" },
                    ]),
                    2,
                ),
            )
        })
        .await;

        let artists = mock.client().all_artists().await.unwrap();

        assert_eq!(artists.len(), 2);
        assert_eq!(artists[0].id, "a1");
        assert_eq!(artists[0].name, "Björk");
        assert_eq!(
            artists[0].cover.as_deref(),
            Some(
                format!(
                    "{}/Items/a1/Images/Primary?maxWidth=300&maxHeight=300&tag=t1",
                    mock.base
                )
                .as_str()
            )
        );
        assert_eq!(artists[1].cover, None);
        let seen = mock.seen();
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].path, "/Users/user-1/Items");
        assert_eq!(seen[0].query["IncludeItemTypes"], "MusicArtist");
        assert_eq!(seen[0].query["Recursive"], "true");
        assert_eq!(seen[0].query["StartIndex"], "0");
        assert_eq!(seen[0].query["Limit"], LIBRARY_PAGE.to_string());
        assert!(seen[0].authorization.contains("Token=\"secret-token\""));
    }

    #[tokio::test]
    async fn lists_albums_with_their_artists_and_year() {
        crypto();
        let mock = Mock::start(|_| {
            (
                200,
                items(
                    serde_json::json!([{
                        "Id": "al1", "Name": "Homogenic", "Type": "MusicAlbum",
                        "AlbumArtist": "Björk",
                        "AlbumArtists": [{ "Name": "Björk", "Id": "a1" }],
                        "ProductionYear": 1997, "ChildCount": 10,
                        "ImageTags": { "Primary": "c1" },
                    }]),
                    1,
                ),
            )
        })
        .await;

        let albums = mock.client().all_albums().await.unwrap();

        assert_eq!(albums.len(), 1);
        let album = &albums[0];
        assert_eq!(album.id, "al1");
        assert_eq!(album.name, "Homogenic");
        assert_eq!(album.artists, "Björk");
        assert_eq!(album.artist_refs[0].id.as_deref(), Some("a1"));
        assert_eq!(album.year, 1997);
        assert_eq!(album.track_count, 10);
        assert!(
            album
                .cover_large
                .as_deref()
                .unwrap()
                .contains("maxWidth=1200")
        );
        assert_eq!(mock.seen()[0].query["IncludeItemTypes"], "MusicAlbum");
    }

    #[tokio::test]
    async fn a_large_library_arrives_in_pages() {
        crypto();
        let total = LIBRARY_PAGE + 1;
        let mock = Mock::start(move |seen| {
            let start: usize = seen.query["StartIndex"].parse().unwrap();
            let count = (total - start).min(LIBRARY_PAGE);
            let page: Vec<_> = (start..start + count)
                .map(|index| serde_json::json!({ "Id": format!("al{index}"), "Name": "x" }))
                .collect();
            (200, items(serde_json::Value::Array(page), total))
        })
        .await;

        let mut pages = mock.client().all_albums_paged().await.unwrap();
        let first = pages.recv().await.unwrap().unwrap();
        let second = pages.recv().await.unwrap().unwrap();

        assert_eq!(first.total, Some(total));
        assert_eq!(first.items.len(), LIBRARY_PAGE);
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].id, format!("al{LIBRARY_PAGE}"));
        assert!(pages.recv().await.is_none());
        let starts: Vec<String> = mock
            .seen()
            .iter()
            .map(|seen| seen.query["StartIndex"].clone())
            .collect();
        assert_eq!(starts, ["0".to_owned(), LIBRARY_PAGE.to_string()]);
    }

    #[tokio::test]
    async fn an_album_lists_its_tracks_by_parent_in_disc_order() {
        crypto();
        let mock = Mock::start(|seen| match seen.query.contains_key("Ids") {
            true => (
                200,
                items(serde_json::json!([{ "Id": "al1", "Name": "Homogenic" }]), 1),
            ),
            false => (
                200,
                items(
                    serde_json::json!([
                        { "Id": "t1", "Name": "Hunter", "Type": "Audio", "AlbumId": "al1",
                          "AlbumPrimaryImageTag": "c1", "RunTimeTicks": 2_550_000_000u64,
                          "IndexNumber": 1, "ParentIndexNumber": 1,
                          "ArtistItems": [{ "Name": "Björk", "Id": "a1" }] },
                        { "Id": "folder", "Name": "Scans", "Type": "Folder" },
                    ]),
                    2,
                ),
            ),
        })
        .await;

        let detail = mock.client().album("al1").await.unwrap();

        assert_eq!(detail.album.name, "Homogenic");
        assert_eq!(detail.tracks.len(), 1);
        let track = &detail.tracks[0];
        assert_eq!(track.id.as_deref(), Some("t1"));
        assert_eq!(track.duration, Duration::from_millis(255_000));
        assert_eq!(track.artists, "Björk");
        assert!(
            track
                .cover
                .as_deref()
                .unwrap()
                .contains("/Items/al1/Images/Primary")
        );
        assert_eq!(detail.album.track_count, 1);
        let children = mock
            .seen()
            .into_iter()
            .find(|seen| seen.query.contains_key("ParentId"))
            .unwrap();
        assert_eq!(children.query["ParentId"], "al1");
        assert_eq!(
            children.query["SortBy"],
            "ParentIndexNumber,IndexNumber,SortName"
        );
    }

    #[tokio::test]
    async fn an_artist_lists_its_albums_by_artist_id() {
        crypto();
        let mock = Mock::start(|seen| match seen.query.contains_key("Ids") {
            true => (
                200,
                items(
                    serde_json::json!([{ "Id": "a1", "Name": "Björk", "Overview": "Icelandic." }]),
                    1,
                ),
            ),
            false => (
                200,
                items(serde_json::json!([{ "Id": "al1", "Name": "Debut" }]), 1),
            ),
        })
        .await;

        let artist = mock.client().artist("a1").await.unwrap();

        assert_eq!(artist.name, "Björk");
        assert_eq!(artist.biography.as_deref(), Some("Icelandic."));
        assert_eq!(artist.albums.len(), 1);
        let albums = mock
            .seen()
            .into_iter()
            .find(|seen| seen.query.contains_key("ArtistIds"))
            .unwrap();
        assert_eq!(albums.query["ArtistIds"], "a1");
        assert_eq!(albums.query["IncludeItemTypes"], "MusicAlbum");
        assert_eq!(albums.query["Recursive"], "true");
    }

    #[tokio::test]
    async fn lists_playlists_of_audio() {
        crypto();
        let mock = Mock::start(|_| {
            (
                200,
                items(
                    serde_json::json!([{ "Id": "p1", "Name": "Road trip", "ChildCount": 12 }]),
                    1,
                ),
            )
        })
        .await;

        let playlists = mock.client().playlists().await.unwrap();

        assert_eq!(playlists[0].name, "Road trip");
        assert_eq!(playlists[0].track_count, 12);
        assert!(!playlists[0].owned);
        let seen = &mock.seen()[0];
        assert_eq!(seen.query["IncludeItemTypes"], "Playlist");
        assert_eq!(seen.query["MediaTypes"], "Audio");
    }

    #[tokio::test]
    async fn search_asks_the_server_for_each_kind() {
        crypto();
        let mock = Mock::start(|seen| {
            let found = match seen.query["IncludeItemTypes"].as_str() {
                "Audio" => serde_json::json!([{ "Id": "t1", "Name": "Jóga", "Type": "Audio" }]),
                "MusicAlbum" => serde_json::json!([{ "Id": "al1", "Name": "Homogenic" }]),
                _ => serde_json::json!([]),
            };
            (200, items(found, 1))
        })
        .await;
        let client = mock.client();

        let tracks = client.search("jó ga").await.unwrap();
        let albums = client.search_albums("homo").await.unwrap();
        let playlists = client.search_playlists("road").await.unwrap();

        assert_eq!(tracks[0].name, "Jóga");
        assert_eq!(albums[0].name, "Homogenic");
        assert!(playlists.is_empty());
        let seen = mock.seen();
        let track_search = seen
            .iter()
            .find(|seen| seen.query["IncludeItemTypes"] == "Audio")
            .unwrap();
        assert_eq!(track_search.query["SearchTerm"], "jó ga");
        assert_eq!(track_search.query["Recursive"], "true");
        assert_eq!(track_search.query["Limit"], SEARCH_TRACKS.to_string());
        assert!(
            seen.iter()
                .any(|seen| seen.query["IncludeItemTypes"] == "Playlist"
                    && seen.query["SearchTerm"] == "road")
        );
    }

    #[tokio::test]
    async fn stream_url_asks_for_the_original_file_without_the_token() {
        crypto();
        let mock = Mock::start(|_| (200, "audio".to_owned())).await;
        let client = mock.client();

        let url = client.stream_url("t 1");
        let response = client.open_stream("t1").await.unwrap();

        assert_eq!(url, format!("{}/Audio/t%201/stream?static=true", mock.base));
        assert!(!url.contains("secret-token"));
        assert_eq!(response.status(), StatusCode::OK);
        let seen = &mock.seen()[0];
        assert_eq!(seen.path, "/Audio/t1/stream");
        assert_eq!(seen.query["static"], "true");
        assert!(!seen.query.contains_key("ApiKey") && !seen.query.contains_key("api_key"));
        assert!(seen.authorization.contains("Token=\"secret-token\""));
    }

    #[tokio::test]
    async fn details_read_the_length_and_normalization_gain() {
        crypto();
        let mock = Mock::start(|_| {
            (
                200,
                items(
                    serde_json::json!([{ "Id": "t1", "RunTimeTicks": 1_800_000_000u64,
                                         "NormalizationGain": -6.5 }]),
                    1,
                ),
            )
        })
        .await;

        let details = mock.client().details("t1").await;

        assert_eq!(details.duration, Some(Duration::from_secs(180)));
        assert_eq!(details.loudness, Some(Loudness::replay_gain(-6.5, None)));
    }

    #[tokio::test]
    async fn an_item_missing_its_fields_still_reads() {
        crypto();
        let mock = Mock::start(|_| (200, r#"{"Items":[{"Id":"t1"}]}"#.to_owned())).await;

        let track = mock.client().track("t1").await.unwrap();

        assert_eq!(track.id.as_deref(), Some("t1"));
        assert_eq!(track.name, "");
        assert_eq!(track.duration, Duration::ZERO);
        assert_eq!(track.cover, None);
    }
}
