use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use base64::Engine as _;
use futures::{StreamExt as _, stream};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;

use crate::connect::Connect;
use crate::cover::{self, CoverLimits};
use crate::engine::Loudness;
use crate::escape;
use crate::jellyfin::auth::{self, Credentials};
use crate::jellyfin::session::{self, Session};
use crate::jellyfin::wire::{self, Covers, Item, Items, Permission, PlaylistInfo};
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, Codec, CoverSize,
    Encoding, Genre, GenreDetail, GenreItem, GenreSection, HomeFeed, MediaKind, MusicApi, Page,
    Pages, Playlist, PlaylistDetail, Report, SUGGESTIONS, SavedArtist, SignInFailure,
    SignInProblem, Track, Transcode, UserProfile, distinct_covers,
};

/// How many items one library request asks for, so a large library arrives in pages.
const LIBRARY_PAGE: usize = 500;
/// How many tracks, albums, artists and playlists a search lists.
const SEARCH_TRACKS: usize = 50;
const SEARCH_OTHERS: usize = 30;
/// The fields every listing asks for beyond what Jellyfin sends anyway.
const FIELDS: &str = "ChildCount,DateCreated,Genres";
/// The fields of a single track, which add who wrote it and what its file holds.
const TRACK_FIELDS: &str = "ChildCount,DateCreated,Genres,People,MediaStreams";
/// The fields of an album page, which add the label.
const ALBUM_FIELDS: &str = "ChildCount,DateCreated,Genres,Studios";
/// The fields an artist page asks for, which add the biography.
const ARTIST_FIELDS: &str = "ChildCount,DateCreated,Overview";
/// The disc and track order an album is heard in.
const ALBUM_ORDER: &str = "ParentIndexNumber,IndexNumber,SortName";
/// The order the songs list and an artist's songs come in.
const SONG_ORDER: &str = "AlbumArtist,Album,ParentIndexNumber,IndexNumber,SortName";
/// How many popular songs an artist page leads with.
const TOP_TRACKS: usize = 10;
/// How long a station is. Jellyfin's instant mix is one list, so this is the whole of it.
const RADIO_COUNT: usize = 50;
/// How many recently played tracks the history keeps.
const RECENT: usize = 50;
/// How many cards a home shelf holds, and how many songs the quick picks hold.
const HOME_SHELF: usize = 15;
const HOME_SONGS: usize = 25;
/// How many playlists have their permissions read at once.
const PERMISSION_READS: usize = 8;
/// A picture Jellyfin keeps as a playlist cover. It shows covers no larger than this.
const PLAYLIST_COVER: CoverLimits = CoverLimits {
    side: 1500,
    bytes: 4 * 1024 * 1024,
};
/// What every converted stream may decode to when the bitrate alone is capped: Jellyfin sends
/// the original when it fits under the cap and converts to Opus only when it does not.
const DECODABLE: &str = "flac,mp3,ogg,opus,m4a,mp4,aac,wav";

/// A signed-in Jellyfin account. Every request carries the `MediaBrowser` authorization header
/// with the access token, and none puts the token in a url.
#[derive(Clone)]
pub struct JellyfinClient {
    pub(crate) http: reqwest::Client,
    /// The server address with no trailing slash.
    server: String,
    user_id: String,
    username: String,
    /// The whole `Authorization` header, token included.
    pub(crate) header: String,
    covers: Covers,
    /// What this device last told the server it plays, shared by every clone.
    session: Session,
    /// The id this install signed in under, which names its session on the server.
    device_id: String,
    /// The remote control handle of a signed-in session, which the session's client hands out.
    remote: Option<Arc<Connect>>,
}

/// What the server records about a track that playback wants before the decoder can tell.
#[derive(Clone, Debug, Default)]
pub struct Details {
    pub duration: Option<Duration>,
    pub loudness: Option<Loudness>,
    pub encoding: Option<Encoding>,
    /// "Artist - Title", what a plain download is named after.
    pub label: Option<String>,
}

/// One item listing request, as its query parameters.
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

    /// Whatever sits directly under `parent`, such as an album's tracks.
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

    fn newest(self, by: &str) -> Self {
        self.with("SortBy", by).with("SortOrder", "Descending")
    }

    fn favorites(self) -> Self {
        self.with("Filters", "IsFavorite")
    }

    fn limit(self, limit: usize) -> Self {
        self.with("Limit", limit.to_string())
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
        return Err(SignInFailure(SignInProblem::Password).into());
    }
    if !status.is_success() {
        anyhow::bail!("the jellyfin server answered the sign-in with {status}");
    }
    let answer: wire::Authentication = response
        .json()
        .await
        .context("the jellyfin sign-in answered something other than an account")?;
    account(server, answer, username, device_id)
}

/// The account a sign-in answered with, as it is kept. Fails when the answer lacks the user id
/// or the token, rather than keeping an account that cannot ask for anything.
pub(crate) fn account(
    server: &str,
    answer: wire::Authentication,
    username: &str,
    device_id: &str,
) -> Result<Credentials> {
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
            session: Session::default(),
            device_id: credentials.device_id.clone(),
            remote: None,
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

    /// Hands out `remote` as this client's remote control, for the client of a signed-in
    /// session.
    pub(crate) fn with_remote(mut self, remote: Arc<Connect>) -> Self {
        self.remote = Some(remote);
        self
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(crate) fn server(&self) -> &str {
        &self.server
    }

    pub(crate) fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Sends a request whose path already holds its query, and reads nothing back.
    pub(crate) async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<()> {
        self.send(method, path, &Query::default(), body)
            .await
            .map(drop)
    }

    /// Reads the json at a path that already holds its query.
    pub(crate) async fn read<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.get(path, &Query::default()).await
    }

    /// The songs that items stand for, in order: a song is itself, and an album, a playlist or
    /// an artist is the songs in it. Items the server does not know are left out.
    pub(crate) async fn songs_of(&self, ids: &[String]) -> Result<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let items = self.list(Query::ids(&ids.join(","))).await?;
        let mut songs = Vec::new();
        for id in ids {
            let Some(item) = items.iter().find(|item| &item.id == id) else {
                continue;
            };
            let tracks = match item.kind.as_deref() {
                Some("Audio") => {
                    songs.push(item.id.clone());
                    continue;
                }
                Some("MusicAlbum") => self.album_songs(id).await?,
                Some("Playlist") => self.playlist_tracks(id).await?,
                Some("MusicArtist") => self.artist_tracks(id).await?,
                _ => continue,
            };
            songs.extend(tracks.into_iter().filter_map(|track| track.id));
        }
        Ok(songs)
    }

    /// The instant mix of an item, as song ids with the seed first.
    pub(crate) async fn instant_mix(&self, seed: &str) -> Result<Vec<String>> {
        let (mix, _) = self.track_radio(seed, None).await?;
        Ok(std::iter::once(seed.to_owned())
            .chain(mix.into_iter().filter_map(|track| track.id))
            .collect())
    }

    /// Which of these ids are songs.
    pub(crate) async fn audio_ids(&self, ids: &[String]) -> Result<HashSet<String>> {
        Ok(self
            .list(Query::ids(&ids.join(",")))
            .await?
            .into_iter()
            .filter(Item::is_audio)
            .map(|item| item.id)
            .collect())
    }

    /// The account and server this client plays from. A track id only means something on its
    /// own server, so this tells apart two servers' tracks that share one.
    pub fn identity(&self) -> String {
        format!("{}@{}", self.user_id, self.server)
    }

    /// Where a track's audio streams from. With nothing to convert it is `GET
    /// /Audio/{itemId}/stream` with `static=true`, the file as it is. A chosen format converts
    /// to it at the chosen bitrate, and a bitrate alone goes through `universal`, which sends
    /// the original when it fits under the cap. The token goes in the header that
    /// `open_stream` sends, never in this url.
    pub fn stream_url(&self, track_id: &str, transcode: Transcode) -> String {
        let id = escape::component(track_id);
        let bits = transcode.bitrate.map(|kbps| kbps * 1000);
        match (transcode.codec, bits) {
            (None, None) => format!("{}/Audio/{id}/stream?static=true", self.server),
            (Some(codec), bits) => {
                let (extension, name) = match codec {
                    Codec::Mp3 => ("mp3", "mp3"),
                    Codec::Opus => ("ogg", "opus"),
                    Codec::Aac => ("aac", "aac"),
                };
                let mut url = format!(
                    "{}/Audio/{id}/stream.{extension}?audioCodec={name}",
                    self.server
                );
                if let Some(bits) = bits {
                    url.push_str(&format!("&audioBitRate={bits}"));
                }
                url
            }
            (None, Some(bits)) => format!(
                "{}/Audio/{id}/universal?userId={}&container={DECODABLE}&maxStreamingBitrate={bits}\
                 &transcodingContainer=ogg&audioCodec=opus&transcodingProtocol=http",
                self.server,
                escape::component(&self.user_id),
            ),
        }
    }

    /// Opens the audio of a track, converted as `transcode` asks, and answers once the response
    /// headers are in. The body is still on its way.
    pub async fn open_stream(
        &self,
        track_id: &str,
        transcode: Transcode,
    ) -> Result<reqwest::Response> {
        let response = self
            .http
            .get(self.stream_url(track_id, transcode))
            .header(AUTHORIZATION, &self.header)
            .send()
            .await
            .map_err(lost)?;
        checked(response, "the stream")
    }

    /// The length, loudness and encoding the server records for a track. Jellyfin's
    /// normalization gain is measured against -18 LUFS, the same reference as ReplayGain, so it
    /// reads as one.
    pub async fn details(&self, track_id: &str) -> Details {
        let query = Query::ids(track_id).with("Fields", TRACK_FIELDS);
        let Ok(Some(item)) = self
            .items(&query)
            .await
            .map(|found| found.items.into_iter().next())
        else {
            return Details::default();
        };
        let title = item.name.clone().unwrap_or_default();
        let label = match item.artists.first().map(|artist| artist.trim()) {
            Some(artist) if !artist.is_empty() => format!("{artist} - {}", title.trim()),
            _ => title.trim().to_owned(),
        };
        Details {
            duration: item.run_time_ticks.map(wire::ticks),
            loudness: item
                .normalization_gain
                .map(|gain| Loudness::replay_gain(gain, None)),
            encoding: item.encoding(),
            label: Some(label),
        }
    }

    /// The lyrics the server keeps for a track, from a sidecar file, the file's tags or a
    /// lyrics plugin. A track with none answers 404, which reads as `None`.
    pub(crate) async fn lyrics(&self, track_id: &str) -> Result<Option<wire::LyricsAnswer>> {
        let path = format!("/Audio/{}/Lyrics", escape::component(track_id));
        let response = self
            .http
            .get(format!("{}{path}", self.server))
            .header(AUTHORIZATION, &self.header)
            .send()
            .await
            .map_err(lost)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let answer = checked(response, &path)?
            .json()
            .await
            .context("cannot read the jellyfin lyrics")?;
        Ok(Some(answer))
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

    /// Sends a request that changes something, with an optional json body, and reads nothing
    /// back.
    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &Query,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.server))
            .query(&query.0)
            .header(AUTHORIZATION, &self.header);
        if let Some(body) = body {
            request = request.json(&body);
        }
        checked(request.send().await.map_err(lost)?, path)
    }

    /// The user id every per-user listing is asked for.
    fn user(&self) -> Query {
        Query::default().with("userId", self.user_id.clone())
    }

    /// One page of `GET /Items` for the signed-in user.
    pub(crate) async fn items(&self, query: &Query) -> Result<Items> {
        let query = query.clone().with("userId", self.user_id.clone());
        self.get("/Items", &query).await
    }

    /// Every item a query matches on `path`, a page at a time, as they arrive. The first page
    /// tells how many there are, and the listing ends at that count or at the first short page.
    fn pages_of<T, F>(&self, path: &'static str, query: Query, convert: F) -> Pages<T>
    where
        T: Send + 'static,
        F: Fn(&Covers, Item) -> Option<T> + Send + 'static,
    {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let client = self.clone();
        let query = query.with("userId", self.user_id.clone());
        tokio::spawn(async move {
            let mut start = 0;
            loop {
                let page: Result<Items> = client
                    .get(path, &query.clone().page(start, LIBRARY_PAGE))
                    .await;
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

    /// Every item of `GET /Items` a query matches, a page at a time.
    pub(crate) fn pages<T, F>(&self, query: Query, convert: F) -> Pages<T>
    where
        T: Send + 'static,
        F: Fn(&Covers, Item) -> Option<T> + Send + 'static,
    {
        self.pages_of("/Items", query, convert)
    }

    /// Every item a paged listing holds, gathered from all of its pages.
    async fn gather<T: Send + 'static>(mut pages: Pages<T>) -> Result<Vec<T>> {
        let mut all = Vec::new();
        while let Some(page) = pages.recv().await {
            all.extend(page?.items);
        }
        Ok(all)
    }

    /// Every item of `GET /Items` a query matches, gathered from all of its pages.
    async fn all<T, F>(&self, query: Query, convert: F) -> Result<Vec<T>>
    where
        T: Send + 'static,
        F: Fn(&Covers, Item) -> Option<T> + Send + 'static,
    {
        Self::gather(self.pages(query, convert)).await
    }

    /// A short list of items of `GET /Items`, in one request.
    async fn list(&self, query: Query) -> Result<Vec<Item>> {
        Ok(self.items(&query).await?.items)
    }

    /// One item by its id, or `None` when the server knows no such item for this user.
    async fn item(&self, id: &str) -> Result<Option<Item>> {
        Ok(self.items(&Query::ids(id)).await?.items.into_iter().next())
    }

    /// One item with extra fields, which must exist.
    async fn item_with(&self, id: &str, fields: &str, what: &str) -> Result<Item> {
        self.items(&Query::ids(id).with("Fields", fields))
            .await
            .with_context(|| format!("cannot load the {what} {id}"))?
            .items
            .into_iter()
            .next()
            .with_context(|| format!("jellyfin has no {what} {id}"))
    }

    /// The tracks of an album in disc and track order.
    async fn album_songs(&self, album: &str) -> Result<Vec<Track>> {
        self.all(
            Query::children(album).sorted(ALBUM_ORDER),
            |covers, item| item.is_audio().then(|| covers.track(item)),
        )
        .await
    }

    /// The entries of a playlist in the playlist's own order. Jellyfin dates an entry by when its
    /// file joined the library rather than the playlist, so the entries carry no date.
    async fn playlist_entries(&self, playlist: &str) -> Result<Vec<Item>> {
        let path = format!("/Playlists/{}/Items", escape::component(playlist));
        let found: Items = self.get(&path, &self.user().with("Fields", FIELDS)).await?;
        Ok(found
            .items
            .into_iter()
            .filter(Item::is_audio)
            .map(|mut item| {
                item.date_created = None;
                item
            })
            .collect())
    }

    /// Who may change a playlist: the owner and any user it is shared with for editing. A
    /// playlist the server will not say anything about is read-only to this account.
    async fn playlist_rights(&self, mut playlist: Playlist) -> Playlist {
        let id = escape::component(&playlist.id);
        let user = escape::component(&self.user_id);
        let (info_path, permission_path) = (
            format!("/Playlists/{id}"),
            format!("/Playlists/{id}/Users/{user}"),
        );
        let none = Query::default();
        let (info, permission) = tokio::join!(
            self.get::<PlaylistInfo>(&info_path, &none),
            self.get::<Permission>(&permission_path, &none),
        );
        let info = info.unwrap_or_default();
        let editable = permission.is_ok_and(|permission| permission.can_edit);
        let shared = info
            .shares
            .iter()
            .any(|share| share.user_id.as_deref() == Some(self.user_id.as_str()));
        playlist.public = info.open_access;
        playlist.owned = editable && !shared;
        playlist.collaborative = editable && shared;
        // Jellyfin has no user pages to link an owner to, so the owner is a name alone.
        if playlist.owned {
            playlist.owner = self.username.clone();
        }
        playlist
    }

    /// Items of one kind whose names match `query`, by the server's own search.
    async fn search_kind(&self, kind: &str, query: &str, limit: usize) -> Result<Vec<Item>> {
        self.list(Query::kind(kind).with("SearchTerm", query).limit(limit))
            .await
            .with_context(|| format!("cannot search jellyfin for {}", kind.to_lowercase()))
    }

    /// Marks an item a favorite of the user's, or takes the mark off.
    async fn favorite(&self, id: &str, favorite: bool) -> Result<()> {
        let method = match favorite {
            true => Method::POST,
            false => Method::DELETE,
        };
        let path = format!("/UserFavoriteItems/{}", escape::component(id));
        self.send(method, &path, &self.user(), None)
            .await
            .with_context(|| format!("cannot change the favorite mark of {id}"))?;
        Ok(())
    }

    /// The album artists of the library, which is what Jellyfin's own client lists as artists.
    fn album_artists(&self, query: Query) -> Pages<SavedArtist> {
        self.pages_of("/Artists/AlbumArtists", query, |covers, item| {
            Some(covers.artist(item))
        })
    }

    /// Albums an artist is credited with, by album artist or as a contributor.
    async fn albums_by(&self, key: &'static str, artist_id: &str) -> Result<Vec<Album>> {
        self.all(
            Query::kind("MusicAlbum")
                .with(key, artist_id)
                .with("Fields", ALBUM_FIELDS)
                .newest("ProductionYear,PremiereDate,SortName"),
            |covers, item| Some(covers.album(item)),
        )
        .await
    }

    /// The artist's songs the user plays most, ranked by the server's play counts.
    async fn top_songs(&self, artist_id: &str) -> Result<Vec<Track>> {
        let found = self
            .list(
                Query::kind("Audio")
                    .with("ArtistIds", artist_id)
                    .newest("PlayCount,CommunityRating,SortName")
                    .limit(TOP_TRACKS),
            )
            .await?;
        Ok(found
            .into_iter()
            .map(|item| self.covers.track(item))
            .collect())
    }

    /// The artists the server finds similar, from shared genres and tags. Artists with no songs
    /// are left out, since the server also keeps one for every artist folder and stale tag.
    async fn similar_artists(&self, artist_id: &str) -> Result<Vec<SavedArtist>> {
        let path = format!("/Artists/{}/Similar", escape::component(artist_id));
        let query = self
            .user()
            .with("Fields", "ItemCounts")
            .limit(SUGGESTIONS + 4);
        let found: Items = self.get(&path, &query).await?;
        Ok(found
            .items
            .into_iter()
            .filter(|item| item.song_count != Some(0))
            .map(|item| self.covers.artist(item))
            .filter(|artist| !wire::is_various(&artist.name))
            .take(SUGGESTIONS)
            .collect())
    }

    /// The albums the server finds similar to one album.
    async fn similar_albums(&self, album_id: &str) -> Result<Vec<Album>> {
        let path = format!("/Items/{}/Similar", escape::component(album_id));
        let found: Items = self.get(&path, &self.user().limit(SUGGESTIONS)).await?;
        Ok(found
            .items
            .into_iter()
            .map(|item| self.covers.album(item))
            .collect())
    }

    /// The albums of `tracks`, once each and in the order their tracks come. Jellyfin marks an
    /// album played only once every track of it is, so the albums of the most played songs are
    /// what the user plays most.
    async fn albums_of(&self, tracks: &[Track]) -> Vec<GenreItem> {
        let mut ids: Vec<&str> = Vec::new();
        for id in tracks.iter().filter_map(|track| track.album_id.as_deref()) {
            if !ids.contains(&id) && ids.len() < HOME_SHELF {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return Vec::new();
        }
        let found = self
            .list(Query::ids(&ids.join(",")))
            .await
            .unwrap_or_default();
        let mut albums: HashMap<String, Album> = found
            .into_iter()
            .map(|item| (item.id.clone(), self.covers.album(item)))
            .collect();
        ids.into_iter()
            .filter_map(|id| albums.remove(id))
            .map(GenreItem::Album)
            .collect()
    }

    /// The newest albums in the library, by `GET /Items/Latest`.
    async fn latest_albums(&self) -> Result<Vec<Album>> {
        let found: Vec<Item> = self
            .get(
                "/Items/Latest",
                &self
                    .user()
                    .with("IncludeItemTypes", "MusicAlbum")
                    .limit(HOME_SHELF),
            )
            .await?;
        Ok(found
            .into_iter()
            .map(|item| self.covers.album(item))
            .collect())
    }
}

#[async_trait]
impl MusicApi for JellyfinClient {
    fn connect(&self) -> Option<Arc<Connect>> {
        self.remote.clone()
    }

    fn cover_at(&self, cover: &str, size: CoverSize) -> Option<String> {
        self.covers.resized(cover, size)
    }

    fn cover_original(&self, cover: &str) -> Option<String> {
        self.covers.resized(cover, CoverSize::Full)
    }

    /// The item's page in Jellyfin's web client on this server.
    fn share_url(&self, _kind: MediaKind, id: &str) -> Option<String> {
        Some(format!(
            "{}/web/#/details?id={}",
            self.server,
            escape::component(id)
        ))
    }

    /// Asks the server who the token belongs to, which is also how a stored sign-in is checked.
    async fn profile(&self) -> Result<UserProfile> {
        let user: wire::User = self
            .get("/Users/Me", &Query::default())
            .await
            .context("cannot reach the jellyfin account")?;
        let name = user.name.unwrap_or_else(|| self.username.clone());
        let avatar = self
            .covers
            .avatar(&self.user_id, user.primary_image_tag.as_deref());
        Ok(wire::profile(&self.user_id, &name, avatar))
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let (artist, albums, top) = tokio::join!(
            self.item_with(artist_id, ARTIST_FIELDS, "artist"),
            self.albums_by("AlbumArtistIds", artist_id),
            self.top_songs(artist_id),
        );
        let artist = artist?;
        let albums = albums.with_context(|| format!("cannot load the albums of {artist_id}"))?;
        let top_tracks = top.unwrap_or_else(|error| {
            log::warn!("jellyfin: cannot rank the songs of {artist_id}: {error:#}");
            Vec::new()
        });
        Ok(Artist {
            cover_large: self.covers.artist_large(&artist),
            biography: artist.overview.clone().filter(|bio| !bio.trim().is_empty()),
            name: artist.name.unwrap_or_default(),
            banner: None,
            monthly_listeners: None,
            followers: None,
            top_tracks,
            albums,
        })
    }

    /// The albums the artist only guests on, apart from the ones credited to them.
    async fn artist_catalogue(&self, artist_id: &str, _known: &[Track]) -> Result<ArtistCatalogue> {
        let (own, guest) = tokio::join!(
            self.albums_by("AlbumArtistIds", artist_id),
            self.albums_by("ContributingArtistIds", artist_id),
        );
        let own = own?;
        let mine: HashSet<String> = own.iter().map(|album| album.id.clone()).collect();
        let appears_on = guest
            .unwrap_or_default()
            .into_iter()
            .filter(|album| !mine.contains(&album.id))
            .collect();
        Ok(ArtistCatalogue {
            albums: Vec::new(),
            top_tracks: Vec::new(),
            appears_on,
        })
    }

    /// Popular songs first, then every other song of the artist's albums, newest album first.
    async fn artist_tracks(&self, artist_id: &str) -> Result<Vec<Track>> {
        let (top, all) = tokio::join!(
            self.top_songs(artist_id),
            self.all(
                Query::kind("Audio")
                    .with("ArtistIds", artist_id)
                    .newest("ProductionYear,Album")
                    .with(
                        "SortBy",
                        "ProductionYear,Album,ParentIndexNumber,IndexNumber"
                    ),
                |covers, item| Some(covers.track(item)),
            ),
        );
        let mut tracks = top.unwrap_or_default();
        tracks.extend(all.with_context(|| format!("cannot load the songs of {artist_id}"))?);
        Ok(crate::artist_songs(tracks, artist_id))
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let artist = self.item_with(artist_id, ARTIST_FIELDS, "artist").await?;
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

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        Self::gather(self.saved_tracks_paged().await?)
            .await
            .context("cannot load the favorite songs")
    }

    async fn saved_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(self.pages(
            Query::kind("Audio").favorites().sorted(SONG_ORDER),
            |covers, item| Some(covers.track(item)),
        ))
    }

    async fn all_tracks(&self) -> Result<Vec<Track>> {
        Self::gather(self.all_tracks_paged().await?)
            .await
            .context("cannot load the jellyfin songs")
    }

    async fn all_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(
            self.pages(Query::kind("Audio").sorted(SONG_ORDER), |covers, item| {
                Some(covers.track(item))
            }),
        )
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.favorite(track_id, saved).await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let item = self.item_with(track_id, TRACK_FIELDS, "song").await?;
        Ok(self.covers.track(item))
    }

    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>> {
        Ok(self.track(track_id).await?.playcount)
    }

    /// Tells the server what plays, through the session reports every Jellyfin client sends.
    /// The dashboard shows it as this device's now playing, and the server counts a song as
    /// played when it stops.
    async fn report(&self, track_id: &str, report: Report, position: Duration) -> Result<()> {
        let client = self.clone();
        let send: session::Sender = Arc::new(move |method, path, body| {
            let client = client.clone();
            Box::pin(async move {
                client
                    .send(method, path, &Query::default(), Some(body))
                    .await
                    .map(drop)
            })
        });
        self.session
            .report(send, track_id, report, position)
            .await
            .with_context(|| format!("cannot report {track_id} to jellyfin"))
    }

    /// Jellyfin counts a song as played when its session report stops, so a second count from
    /// here would play every song twice. Nothing is sent.
    async fn played(&self, _track_id: &str, _at: SystemTime) -> Result<()> {
        Ok(())
    }

    async fn recently_played(&self) -> Result<Vec<Track>> {
        let found = self
            .list(
                Query::kind("Audio")
                    .with("Filters", "IsPlayed")
                    .newest("DatePlayed")
                    .limit(RECENT),
            )
            .await
            .context("cannot load the recently played songs")?;
        Ok(found
            .into_iter()
            .map(|item| self.covers.track(item))
            .collect())
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let listed = self
            .all(
                Query::kind("Playlist")
                    .with("MediaTypes", "Audio")
                    .sorted("SortName"),
                |covers, item| Some(covers.playlist(item)),
            )
            .await
            .context("cannot load the jellyfin playlists")?;
        Ok(stream::iter(listed)
            .map(|playlist| self.playlist_rights(playlist))
            .buffered(PERMISSION_READS)
            .collect()
            .await)
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let body = serde_json::json!({
            "Name": name,
            "UserId": self.user_id,
            "MediaType": "Audio",
            "IsPublic": false,
        });
        let created: wire::Created = self
            .send(Method::POST, "/Playlists", &Query::default(), Some(body))
            .await
            .context("cannot create the playlist")?
            .json()
            .await
            .context("cannot read the new playlist")?;
        Ok(created.id)
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        let path = format!("/Playlists/{}", escape::component(playlist_id));
        self.send(
            Method::POST,
            &path,
            &Query::default(),
            Some(serde_json::json!({ "Name": name })),
        )
        .await
        .context("cannot rename the playlist")?;
        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let path = format!("/Items/{}", escape::component(playlist_id));
        self.send(Method::DELETE, &path, &Query::default(), None)
            .await
            .context("cannot delete the playlist")?;
        Ok(())
    }

    async fn remove_playlist_from_library(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    async fn add_playlist_to_library(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    /// A public playlist is open to every user of the server.
    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        let path = format!("/Playlists/{}", escape::component(playlist_id));
        self.send(
            Method::POST,
            &path,
            &Query::default(),
            Some(serde_json::json!({ "IsPublic": public })),
        )
        .await
        .context("cannot change the playlist visibility")?;
        Ok(())
    }

    /// Uploads the picture as the playlist's primary image, base64 in the body as Jellyfin
    /// takes it. Taking it off brings back the collage Jellyfin draws from the tracks.
    async fn set_playlist_cover(&self, playlist_id: &str, image: Option<&[u8]>) -> Result<()> {
        let path = format!("/Items/{}/Images/Primary", escape::component(playlist_id));
        let url = format!("{}{path}", self.server);
        let request = match image {
            Some(image) => {
                let jpeg = cover::square_jpeg(image, PLAYLIST_COVER)?;
                self.http
                    .post(url)
                    .header(CONTENT_TYPE, "image/jpeg")
                    .body(base64::engine::general_purpose::STANDARD.encode(jpeg))
            }
            None => self.http.delete(url),
        };
        let response = request
            .header(AUTHORIZATION, &self.header)
            .send()
            .await
            .map_err(lost)?;
        checked(response, &path).context("cannot change the playlist cover")?;
        Ok(())
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.add_tracks_to_playlist(playlist_id, &[track_id.to_owned()])
            .await
    }

    async fn add_tracks_to_playlist(&self, playlist_id: &str, track_ids: &[String]) -> Result<()> {
        if track_ids.is_empty() {
            return Ok(());
        }
        let path = format!("/Playlists/{}/Items", escape::component(playlist_id));
        self.send(
            Method::POST,
            &path,
            &self.user().with("ids", track_ids.join(",")),
            None,
        )
        .await
        .context("cannot add to the playlist")?;
        Ok(())
    }

    /// Removes the track by its entry in the playlist. Jellyfin names an entry after the track
    /// it holds, so a track the playlist holds twice leaves it entirely.
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let entries = self
            .playlist_entries(playlist_id)
            .await
            .context("cannot load the playlist")?;
        let entry = entries
            .iter()
            .find(|item| item.id == track_id)
            .and_then(|item| item.playlist_item_id.clone())
            .unwrap_or_else(|| track_id.to_owned());
        let path = format!("/Playlists/{}/Items", escape::component(playlist_id));
        self.send(
            Method::DELETE,
            &path,
            &Query::default().with("entryIds", entry),
            None,
        )
        .await
        .context("cannot remove the track from the playlist")?;
        Ok(())
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        Self::gather(self.saved_albums_paged().await?)
            .await
            .context("cannot load the favorite albums")
    }

    async fn saved_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(self.pages(
            Query::kind("MusicAlbum").favorites().sorted("SortName"),
            |covers, item| Some(covers.album(item)),
        ))
    }

    async fn all_albums(&self) -> Result<Vec<Album>> {
        Self::gather(self.all_albums_paged().await?)
            .await
            .context("cannot load the jellyfin albums")
    }

    async fn all_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(self.pages(
            Query::kind("MusicAlbum").sorted("SortName"),
            |covers, item| Some(covers.album(item)),
        ))
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.favorite(album_id, saved).await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        Self::gather(self.saved_artists_paged().await?)
            .await
            .context("cannot load the favorite artists")
    }

    /// Favorite artists come from every artist, not only album artists, since a favorite may
    /// be one the user only heard as a guest.
    async fn saved_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(self.pages_of(
            "/Artists",
            Query::default()
                .with("isFavorite", "true")
                .sorted("SortName"),
            |covers, item| Some(covers.artist(item)),
        ))
    }

    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        Self::gather(self.all_artists_paged().await?)
            .await
            .context("cannot load the jellyfin artists")
    }

    async fn all_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(self.album_artists(Query::default().sorted("SortName")))
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.favorite(artist_id, saved).await
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let (album, tracks) = tokio::join!(
            self.item_with(album_id, ALBUM_FIELDS, "album"),
            self.album_songs(album_id)
        );
        let tracks = tracks.with_context(|| format!("cannot load the songs of {album_id}"))?;
        let mut album = self.covers.album(album?);
        if album.track_count == 0 {
            album.track_count = tracks.len() as u32;
        }
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        self.album_songs(album_id)
            .await
            .with_context(|| format!("cannot load the songs of {album_id}"))
    }

    /// More from the album's artist first, then what the server finds similar, and the
    /// similar artists beside them.
    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        let (more_by, similar_albums, similar) = tokio::join!(
            async {
                match artist_id {
                    Some(artist) => self.albums_by("AlbumArtistIds", artist).await,
                    None => Ok(Vec::new()),
                }
            },
            self.similar_albums(album_id),
            async {
                match artist_id {
                    Some(artist) => self.similar_artists(artist).await,
                    None => Ok(Vec::new()),
                }
            },
        );
        if let (Err(error), Err(_)) = (&more_by, &similar_albums) {
            return Err(anyhow::anyhow!("{error:#}").context("cannot read any recommendations"));
        }
        let mut seen = HashSet::from([album_id.to_owned()]);
        let also_like = more_by
            .unwrap_or_default()
            .into_iter()
            .chain(similar_albums.unwrap_or_default())
            .filter(|album| seen.insert(album.id.clone()))
            .take(SUGGESTIONS)
            .collect();
        Ok(AlbumCatalogue {
            also_like,
            similar: similar.unwrap_or_default(),
        })
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let (playlist, entries) =
            tokio::join!(self.item(playlist_id), self.playlist_entries(playlist_id));
        let playlist = playlist
            .with_context(|| format!("cannot load the playlist {playlist_id}"))?
            .with_context(|| format!("jellyfin has no playlist {playlist_id}"))?;
        let tracks: Vec<Track> = entries
            .with_context(|| format!("cannot load the songs of {playlist_id}"))?
            .into_iter()
            .map(|item| self.covers.track(item))
            .collect();
        let mut playlist = self.playlist_rights(self.covers.playlist(playlist)).await;
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
        let entries = self
            .playlist_entries(playlist_id)
            .await
            .with_context(|| format!("cannot load the songs of {playlist_id}"))?;
        Ok(entries
            .into_iter()
            .map(|item| self.covers.track(item))
            .collect())
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = self.playlist_tracks(playlist_id).await?;
        Ok(distinct_covers(&tracks, wanted))
    }

    /// Jellyfin's instant mix: songs that share the seed's genres, artists and album. It comes
    /// in one list, so there is never a continuation.
    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let path = format!("/Items/{}/InstantMix", escape::component(track_id));
        let found: Items = self
            .get(
                &path,
                &self.user().with("Fields", FIELDS).limit(RADIO_COUNT),
            )
            .await
            .context("cannot load the instant mix")?;
        Ok((
            found
                .items
                .into_iter()
                .filter(|item| item.id != track_id)
                .map(|item| self.covers.track(item))
                .collect(),
            None,
        ))
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

    async fn search_artists(&self, query: &str) -> Result<Vec<SavedArtist>> {
        let found: Items = self
            .get(
                "/Artists",
                &self.user().with("searchTerm", query).limit(SEARCH_OTHERS),
            )
            .await
            .context("cannot search jellyfin for artists")?;
        Ok(found
            .items
            .into_iter()
            .map(|item| self.covers.artist(item))
            .collect())
    }

    /// What the user played lately, songs picked from what they play most and at random, and
    /// shelves of new, favorite and most played albums and their playlists.
    async fn home(&self) -> Result<HomeFeed> {
        let songs = |query: Query| async move {
            self.list(query)
                .await
                .map(|items| {
                    items
                        .into_iter()
                        .map(|item| self.covers.track(item))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let albums = |query: Query| async move {
            self.list(query)
                .await
                .map(|items| {
                    items
                        .into_iter()
                        .map(|item| GenreItem::Album(self.covers.album(item)))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let (recent, frequent, random, latest, favorites, playlists) = tokio::join!(
            songs(
                Query::kind("Audio")
                    .with("Filters", "IsPlayed")
                    .newest("DatePlayed")
                    .limit(HOME_SHELF)
            ),
            songs(
                Query::kind("Audio")
                    .with("Filters", "IsPlayed")
                    .newest("PlayCount")
                    .limit(HOME_SONGS)
            ),
            songs(
                Query::kind("Audio")
                    .with("SortBy", "Random")
                    .limit(HOME_SONGS)
            ),
            self.latest_albums(),
            albums(
                Query::kind("MusicAlbum")
                    .favorites()
                    .with("SortBy", "Random")
                    .limit(HOME_SHELF)
            ),
            self.playlists(),
        );
        let most_played = self.albums_of(&frequent).await;
        let mut picks: Vec<Track> = frequent.into_iter().take(HOME_SONGS / 2).collect();
        let mut seen: HashSet<Option<String>> =
            picks.iter().map(|track| track.id.clone()).collect();
        picks.extend(
            random
                .into_iter()
                .filter(|track| seen.insert(track.id.clone())),
        );
        picks.truncate(HOME_SONGS);
        fastrand::shuffle(&mut picks);

        let shelf = |title: &str, items: Vec<GenreItem>| {
            (!items.is_empty()).then(|| GenreSection {
                title: title.to_owned(),
                items,
            })
        };
        let latest = latest
            .unwrap_or_default()
            .into_iter()
            .map(GenreItem::Album)
            .collect();
        let playlists = playlists
            .unwrap_or_default()
            .into_iter()
            .take(HOME_SHELF)
            .map(GenreItem::Playlist)
            .collect();
        let sections = [
            shelf("home-recently-added", latest),
            shelf("home-most-played", most_played),
            shelf("home-favorite-albums", favorites),
            shelf("home-playlists", playlists),
        ]
        .into_iter()
        .flatten()
        .collect();
        Ok(HomeFeed {
            listen_again: recent.into_iter().map(GenreItem::Track).collect(),
            quick_picks: Some(picks),
            sections,
        })
    }

    /// The genres that hold songs. Jellyfin also lists genres it read for an artist from online
    /// metadata, which have nothing in them to play.
    async fn genres(&self) -> Result<Vec<Genre>> {
        let found: Items = self
            .get(
                "/MusicGenres",
                &self.user().with("Fields", "ItemCounts").sorted("SortName"),
            )
            .await
            .context("cannot load the genres")?;
        Ok(found
            .items
            .into_iter()
            .filter(|item| item.song_count != Some(0))
            .map(|item| self.covers.genre(item))
            .collect())
    }

    /// The genre's albums, most played first, and a shelf of its songs to start from.
    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        let (genre, albums, songs) = tokio::join!(
            self.item(genre_id),
            self.list(
                Query::kind("MusicAlbum")
                    .with("GenreIds", genre_id)
                    .newest("PlayCount,ProductionYear,SortName")
                    .limit(SEARCH_TRACKS)
            ),
            self.list(
                Query::kind("Audio")
                    .with("GenreIds", genre_id)
                    .with("SortBy", "Random")
                    .limit(HOME_SHELF)
            ),
        );
        let name = genre
            .ok()
            .flatten()
            .and_then(|genre| genre.name)
            .unwrap_or_else(|| genre_id.to_owned());
        let albums: Vec<GenreItem> = albums
            .with_context(|| format!("cannot load the genre {genre_id}"))?
            .into_iter()
            .map(|item| GenreItem::Album(self.covers.album(item)))
            .collect();
        let songs: Vec<GenreItem> = songs
            .unwrap_or_default()
            .into_iter()
            .map(|item| GenreItem::Track(self.covers.track(item)))
            .collect();
        let sections = [
            ("nav-albums".to_owned(), albums),
            ("home-quick-picks".to_owned(), songs),
        ]
        .into_iter()
        .filter(|(_, items)| !items.is_empty())
        .map(|(title, items)| GenreSection { title, items })
        .collect();
        Ok(GenreDetail { name, sections })
    }

    /// The genre's own picture when it has one, else the cover of its most played album.
    async fn genre_cover(&self, genre_id: &str) -> Result<Option<String>> {
        if let Some(cover) = self
            .item(genre_id)
            .await?
            .and_then(|genre| self.covers.genre(genre).cover)
        {
            return Ok(Some(cover));
        }
        let found = self
            .list(
                Query::kind("MusicAlbum")
                    .with("GenreIds", genre_id)
                    .newest("PlayCount")
                    .limit(1),
            )
            .await
            .with_context(|| format!("cannot load the genre {genre_id}"))?;
        Ok(found
            .into_iter()
            .find_map(|item| self.covers.album(item).cover))
    }
}

/// Passes a successful response on and turns any other into an error. A 401 is the server
/// refusing the token, which the session reads as a sign-in to do again. Only the status is
/// logged, never the request's headers, and a 404 quietly, since asking after a playlist the
/// account may not change answers one.
fn checked(response: reqwest::Response, what: &str) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    match status {
        StatusCode::NOT_FOUND => log::info!("jellyfin: {what} answered {status}"),
        _ => log::warn!("jellyfin: {what} answered {status}"),
    }
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
pub(crate) fn lost(error: reqwest::Error) -> anyhow::Error {
    anyhow::Error::new(error.without_url()).context("could not reach the jellyfin server")
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

        assert_eq!(
            error.downcast_ref::<SignInFailure>(),
            Some(&SignInFailure(SignInProblem::Password))
        );
        assert!(!crate::trouble::offline(&format!("{error:#}")));
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
                    "{}/Items/a1/Images/Primary?tag=t1&maxWidth=300&maxHeight=300",
                    mock.base
                )
                .as_str()
            )
        );
        assert_eq!(artists[1].cover, None);
        let seen = mock.seen();
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].path, "/Artists/AlbumArtists");
        assert_eq!(seen[0].query["userId"], "user-1");
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
    async fn an_artist_lists_the_albums_credited_to_it() {
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
            .find(|seen| seen.query.contains_key("AlbumArtistIds"))
            .unwrap();
        assert_eq!(albums.query["AlbumArtistIds"], "a1");
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

        let url = client.stream_url("t 1", Transcode::default());
        let response = client
            .open_stream("t1", Transcode::default())
            .await
            .unwrap();

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
