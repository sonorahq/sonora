use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use reqwest::Method;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::engine::Loudness;
use crate::plex::auth::{self, Credentials};
use crate::plex::tv;
use crate::plex::wire::{Container, Envelope, Item, Stream};
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, ArtistRef, Genre,
    GenreDetail, GenreItem, GenreSection, HomeFeed, MediaKind, MusicApi, Page, Pages, Playlist,
    PlaylistDetail, ReleaseType, Report, SUGGESTIONS, SavedArtist, SignInFailure, SignInProblem,
    Track, UserProfile, distinct_covers, escape,
};

/// Plex's own name for the library every client reads and writes ratings and plays through.
const IDENTIFIER: &str = "com.plexapp.plugins.library";
/// The numbers Plex gives the kinds of item a music library lists.
const ARTISTS: &str = "8";
const ALBUMS: &str = "9";
const TRACKS: &str = "10";
/// A rating of five stars, which is what a favorite is.
const FAVORITE: &str = "10";
const LIBRARY_PAGE: usize = 500;
const PORTRAIT_LIMIT: usize = 24;
const RADIO_COUNT: &str = "25";
const HOME_SONGS: &str = "25";
const HOME_ALBUMS: &str = "12";
const SIMILAR_ARTISTS: &str = "10";
/// How long an ordinary request waits for the server. A stream has no such limit.
const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT: Duration = Duration::from_secs(5);
/// What a stream of lyrics is in Plex's numbering.
const AUDIO_STREAM: u32 = 2;
const LYRICS_STREAM: u32 = 4;

/// What the server records about a track that playback wants before the decoder can tell.
#[derive(Clone, Debug, Default)]
pub struct Details {
    pub duration: Option<Duration>,
    /// The track's loudness, when the server has analysed it.
    pub loudness: Option<Loudness>,
    /// Where the file is served from, on the server.
    pub part: Option<String>,
}

#[derive(Clone)]
pub struct PlexClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    client_id: String,
    stored: Credentials,
    /// The address of the server that last answered. It moves when the server does.
    uri: RwLock<String>,
    /// Set once the server refuses the token, so the session knows to sign in again.
    revoked: AtomicBool,
    /// The length of the track last reported, so a timeline update can carry it.
    reported: Mutex<Option<(String, u64)>>,
}

impl PlexClient {
    pub fn new(stored: Credentials, client_id: String) -> Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT)
            .build()
            .context("cannot build the plex http client")?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                client_id,
                uri: RwLock::new(stored.server.uri.clone()),
                stored,
                revoked: AtomicBool::new(false),
                reported: Mutex::new(None),
            }),
        })
    }

    /// The account and server this client plays from. A track id only means something on its
    /// own server, so this tells apart two servers' tracks that share one.
    pub fn identity(&self) -> String {
        format!(
            "{}@{}",
            self.inner.stored.user.id, self.inner.stored.server.id
        )
    }

    fn base(&self) -> String {
        self.inner
            .uri
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = tv::identity(&self.inner.client_id);
        if let Ok(token) = HeaderValue::from_str(&self.inner.stored.server.token) {
            headers.insert("x-plex-token", token);
        }
        headers
    }

    /// Looks for the server again through plex.tv, for when the address that worked has stopped
    /// answering: the server moved, or the listener left the network it was on.
    async fn reroute(&self) -> Result<()> {
        let stored = &self.inner.stored;
        let servers = tv::servers(&self.inner.http, &self.inner.client_id, &stored.token).await?;
        let server = servers
            .into_iter()
            .find(|server| server.client_identifier == stored.server.id)
            .context("the plex account no longer has this server")?;
        let uri = tv::reach(
            &self.inner.http,
            &self.inner.client_id,
            &stored.server.token,
            &server.connections,
        )
        .await?;
        log::info!("plex: reached {} at {uri}", stored.server.name);
        if let Some(mut remembered) = auth::load()
            && remembered.server.id == stored.server.id
            && remembered.server.uri != uri
        {
            remembered.server.uri = uri.clone();
            if let Err(error) = auth::store(&remembered) {
                log::warn!("plex: cannot keep the new address: {error:#}");
            }
        }
        *self
            .inner
            .uri
            .write()
            .unwrap_or_else(PoisonError::into_inner) = uri;
        Ok(())
    }

    /// Sends one request to the server. A server that does not answer is looked for again once,
    /// and a refused token marks the session revoked.
    async fn send(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response> {
        let mut rerouted = false;
        loop {
            let mut request = self
                .inner
                .http
                .request(method.clone(), format!("{}{path}", self.base()))
                .headers(self.headers())
                .query(query);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(error) if !rerouted && (error.is_connect() || error.is_timeout()) => {
                    rerouted = true;
                    log::info!("plex: the server did not answer for {path}: {error}");
                    self.reroute().await?;
                    continue;
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot reach plex for {path}"));
                }
            };
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                self.inner.revoked.store(true, Ordering::Relaxed);
                return Err(SignInFailure(SignInProblem::Credentials))
                    .with_context(|| format!("plex refused the token for {path}"));
            }
            return response
                .error_for_status()
                .with_context(|| format!("plex refused {path}"));
        }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<reqwest::Response> {
        self.send(method, path, query, Some(TIMEOUT)).await
    }

    async fn container(&self, path: &str, query: &[(&str, &str)]) -> Result<Container> {
        let envelope: Envelope = self
            .request(Method::GET, path, query)
            .await?
            .json()
            .await
            .with_context(|| format!("cannot read the plex answer for {path}"))?;
        Ok(envelope.container)
    }

    async fn items(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Item>> {
        Ok(self.container(path, query).await?.metadata)
    }

    /// The first item of a listing, which is what asking for one thing by its key gives.
    async fn one(&self, path: &str) -> Result<Item> {
        self.items(path, &[])
            .await?
            .into_iter()
            .next()
            .with_context(|| format!("plex has nothing at {path}"))
    }

    async fn metadata(&self, id: &str) -> Result<Item> {
        self.one(&format!("/library/metadata/{}", escape::component(id)))
            .await
    }

    /// One page of a listing, `LIBRARY_PAGE` items from `start`.
    async fn page(&self, path: &str, query: &[(&str, &str)], start: usize) -> Result<Container> {
        let start = start.to_string();
        let size = LIBRARY_PAGE.to_string();
        let mut paged = query.to_vec();
        paged.push(("X-Plex-Container-Start", start.as_str()));
        paged.push(("X-Plex-Container-Size", size.as_str()));
        self.container(path, &paged).await
    }

    /// Every item of a listing, read a page at a time.
    async fn listing(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Item>> {
        let mut items = Vec::new();
        loop {
            let page = self.page(path, query, items.len()).await?;
            let fetched = page.metadata.len();
            let total = page.total_size;
            items.extend(page.metadata);
            if fetched == 0 || total.is_none_or(|total| items.len() >= total) {
                return Ok(items);
            }
        }
    }

    /// A listing that reaches the screen a page at a time. `convert` runs on each item of a page
    /// as it arrives.
    fn paged<T: Send + 'static>(
        &self,
        path: String,
        query: Vec<(&'static str, String)>,
        convert: fn(&Self, Item) -> T,
    ) -> Pages<T> {
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        let client = self.clone();
        tokio::spawn(async move {
            let mut start = 0;
            loop {
                let borrowed: Vec<(&str, &str)> = query
                    .iter()
                    .map(|(key, value)| (*key, value.as_str()))
                    .collect();
                let page = match client.page(&path, &borrowed, start).await {
                    Ok(page) => page,
                    Err(error) => {
                        sender.send(Err(error)).await.ok();
                        return;
                    }
                };
                let fetched = page.metadata.len();
                let total = page.total_size;
                let items = page
                    .metadata
                    .into_iter()
                    .map(|item| convert(&client, item))
                    .collect();
                // The receiver is gone once the listing is no longer wanted.
                if sender.send(Ok(Page { total, items })).await.is_err() {
                    return;
                }
                start += fetched;
                if fetched == 0 || total.is_none_or(|total| start >= total) {
                    return;
                }
            }
        });
        receiver
    }

    fn section(&self, tail: &str) -> String {
        format!("/library/sections/{}{tail}", self.inner.stored.section.key)
    }

    /// Reads everything a paged listing has.
    async fn drain<T>(mut pages: Pages<T>) -> Result<Vec<T>> {
        let mut all = Vec::new();
        while let Some(page) = pages.recv().await {
            all.extend(page?.items);
        }
        Ok(all)
    }

    /// The address a playlist call names tracks by.
    fn uri_of(&self, ids: &[&str]) -> String {
        format!(
            "server://{}/{IDENTIFIER}/library/metadata/{}",
            self.inner.stored.server.id,
            ids.join(",")
        )
    }

    /// A picture from the server at `size`, scaled by the server so a cover is never the
    /// original scan.
    fn image(&self, path: Option<&str>, size: u32) -> Option<String> {
        let path = path.filter(|path| !path.is_empty())?;
        Some(format!(
            "{}/photo/:/transcode?width={size}&height={size}&minSize=1&upscale=1&url={}&X-Plex-Token={}",
            self.base(),
            escape::component(path),
            escape::component(&self.inner.stored.server.token),
        ))
    }

    fn track(&self, item: Item) -> Track {
        let album_artist = item.grandparent_title.clone().unwrap_or_default();
        let artist_refs = match album_artist.is_empty() {
            true => Vec::new(),
            false => vec![ArtistRef {
                name: album_artist.clone(),
                id: item
                    .grandparent_rating_key
                    .clone()
                    .filter(|id| !id.is_empty()),
            }],
        };
        let artists = item
            .original_title
            .clone()
            .filter(|credited| !credited.is_empty())
            .unwrap_or(album_artist);
        let cover = self.image(
            item.parent_thumb
                .as_deref()
                .or(item.thumb.as_deref())
                .or(item.grandparent_thumb.as_deref()),
            300,
        );
        Track {
            id: Some(item.rating_key),
            name: item.title,
            playable: true,
            artists,
            artist_refs,
            album: item.parent_title.unwrap_or_default(),
            album_id: item.parent_rating_key.filter(|id| !id.is_empty()),
            cover,
            duration: Duration::from_millis(item.duration.unwrap_or(0)),
            added_at: item.added_at,
            added_by: None,
            playcount: item.view_count,
            popularity: 0,
            explicit: false,
            track_number: item.index.unwrap_or(0),
            disc_number: item.parent_index.unwrap_or(1).max(1),
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        }
    }

    fn album(&self, item: Item) -> Album {
        let artist = item.parent_title.clone().unwrap_or_default();
        let artist_refs = match artist.is_empty() {
            true => Vec::new(),
            false => vec![ArtistRef {
                name: artist.clone(),
                id: item.parent_rating_key.clone().filter(|id| !id.is_empty()),
            }],
        };
        let formats: Vec<&str> = item.format.iter().map(|tag| tag.tag.as_str()).collect();
        let compilation = formats
            .iter()
            .any(|format| format.eq_ignore_ascii_case("compilation"));
        let year = item.year.unwrap_or(0);
        Album {
            cover: self.image(item.thumb.as_deref(), 300),
            cover_large: self.image(item.thumb.as_deref(), 600),
            release_type: match formats.is_empty() {
                true => ReleaseType::Album,
                false => ReleaseType::from_musicbrainz(formats, compilation),
            },
            release_date: item
                .originally_available_at
                .clone()
                .filter(|date| !date.is_empty())
                .unwrap_or_else(|| match year {
                    0 => String::new(),
                    _ => year.to_string(),
                }),
            label: item.studio.clone().unwrap_or_default(),
            id: item.rating_key,
            name: item.title,
            artists: artist,
            artist_refs,
            year,
            track_count: item.leaf_count.unwrap_or(0),
            copyrights: Vec::new(),
            added_at: item.added_at,
        }
    }

    fn artist_card(&self, item: Item) -> SavedArtist {
        SavedArtist {
            cover: self.image(item.thumb.as_deref(), 300),
            id: item.rating_key,
            name: item.title,
            added_at: item.added_at,
        }
    }

    fn playlist_card(&self, item: Item) -> Playlist {
        let owner = self.inner.stored.user.name.clone();
        Playlist {
            cover: self.image(item.composite.as_deref(), 300),
            id: item.rating_key,
            name: item.title,
            owner_id: self.inner.stored.user.id.clone(),
            owner,
            owned: true,
            collaborative: false,
            blend: false,
            public: false,
            track_count: item.leaf_count.unwrap_or(0),
            modified_at: item.updated_at,
        }
    }

    /// Sets how the listener rates an item. Five stars is a favorite, and `-1` takes a rating
    /// away.
    async fn rate(&self, id: &str, rating: &str) -> Result<()> {
        self.request(
            Method::PUT,
            "/:/rate",
            &[("key", id), ("identifier", IDENTIFIER), ("rating", rating)],
        )
        .await?;
        Ok(())
    }

    async fn change_saved(&self, id: &str, saved: bool) -> Result<()> {
        let rating = match saved {
            true => FAVORITE,
            false => "-1",
        };
        self.rate(id, rating).await
    }

    /// The query that lists what the listener has given five stars, of one kind of item.
    fn rated(kind: &'static str) -> [(&'static str, &'static str); 2] {
        [("type", kind), ("userRating>>", FAVORITE)]
    }

    /// The tracks of an album, in the order the album is heard.
    async fn album_items(&self, album_id: &str) -> Result<Vec<Track>> {
        let mut tracks: Vec<Track> = self
            .listing(
                &format!("/library/metadata/{}/children", escape::component(album_id)),
                &[],
            )
            .await?
            .into_iter()
            .map(|item| self.track(item))
            .collect();
        tracks.sort_by(|a, b| {
            (a.disc_number, a.track_number)
                .cmp(&(b.disc_number, b.track_number))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(tracks)
    }

    /// The releases of an artist.
    async fn artist_albums(&self, artist_id: &str) -> Result<Vec<Album>> {
        Ok(self
            .listing(
                &format!(
                    "/library/metadata/{}/children",
                    escape::component(artist_id)
                ),
                &[],
            )
            .await?
            .into_iter()
            .map(|item| self.album(item))
            .collect())
    }

    /// The tracks of an artist that have been played the most.
    async fn popular_tracks(&self, artist_id: &str) -> Result<Vec<Track>> {
        Ok(self
            .items(
                &self.section("/all"),
                &[
                    ("type", TRACKS),
                    ("artist.id", artist_id),
                    ("sort", "viewCount:desc"),
                    ("X-Plex-Container-Start", "0"),
                    ("X-Plex-Container-Size", "10"),
                ],
            )
            .await?
            .into_iter()
            .map(|item| self.track(item))
            .collect())
    }

    /// Up to `SUGGESTIONS` of the artist's own releases other than the one the page is on.
    async fn more_from_artist(&self, album_id: &str, artist_id: &str) -> Result<Vec<Album>> {
        Ok(self
            .artist_albums(artist_id)
            .await?
            .into_iter()
            .filter(|album| album.id != album_id)
            .take(SUGGESTIONS)
            .collect())
    }

    /// Artists the server lists as similar, when it has analysed the library.
    async fn similar_artists(&self, artist_id: &str) -> Result<Vec<SavedArtist>> {
        Ok(self
            .items(
                &format!("/library/metadata/{}/similar", escape::component(artist_id)),
                &[("count", SIMILAR_ARTISTS)],
            )
            .await?
            .into_iter()
            .filter(|item| !item.title.is_empty() && !item.rating_key.is_empty())
            .map(|item| self.artist_card(item))
            .take(SUGGESTIONS)
            .collect())
    }

    async fn random_tracks(&self, count: &str) -> Result<Vec<Track>> {
        Ok(self
            .items(
                &self.section("/all"),
                &[
                    ("type", TRACKS),
                    ("sort", "random"),
                    ("X-Plex-Container-Start", "0"),
                    ("X-Plex-Container-Size", count),
                ],
            )
            .await?
            .into_iter()
            .map(|item| self.track(item))
            .collect())
    }

    /// A listing of the section's albums, tracks or artists, ordered by `sort`, that the
    /// listener has played, newest first when the order is by play.
    async fn ordered(&self, kind: &str, sort: &str, count: &str) -> Result<Vec<Item>> {
        self.items(
            &self.section("/all"),
            &[
                ("type", kind),
                ("sort", sort),
                ("X-Plex-Container-Start", "0"),
                ("X-Plex-Container-Size", count),
            ],
        )
        .await
    }

    /// What the server records about a track: its length, its loudness where the server has
    /// measured it, and where its file is served from.
    pub async fn details(&self, track_id: &str) -> Result<Details> {
        let item = self.metadata(track_id).await?;
        Ok(details_of(&item))
    }

    /// Opens the file of a track and answers once the response headers are in. The body is
    /// still on its way, and the server sends the file as it is, with no conversion.
    pub async fn open_stream(&self, part: &str) -> Result<reqwest::Response> {
        self.send(Method::GET, part, &[], None).await
    }

    /// The lyrics the server holds for a track, as the text of the first lyrics stream, and
    /// whether that text is timed.
    pub(crate) async fn lyrics_text(&self, track_id: &str) -> Result<Option<String>> {
        let item = self.metadata(track_id).await?;
        let stream = item
            .media
            .iter()
            .flat_map(|media| &media.part)
            .flat_map(|part| &part.stream)
            .find(|stream| stream.stream_type == Some(LYRICS_STREAM) && stream.id.is_some());
        let Some(id) = stream.and_then(|stream| stream.id) else {
            return Ok(None);
        };
        let text = self
            .request(Method::GET, &format!("/library/streams/{id}"), &[])
            .await?
            .text()
            .await
            .context("cannot read the lyrics")?;
        Ok(Some(text))
    }

    /// The length of a track in milliseconds, kept for the one reported last so a timeline
    /// update does not ask for it every time.
    async fn length_of(&self, track_id: &str) -> u64 {
        let known = self
            .inner
            .reported
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some((id, length)) = known
            && id == track_id
        {
            return length;
        }
        let length = match self.metadata(track_id).await {
            Ok(item) => item.duration.unwrap_or(0),
            Err(error) => {
                log::warn!("plex: cannot read the length of {track_id}: {error:#}");
                0
            }
        };
        *self
            .inner
            .reported
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((track_id.to_owned(), length));
        length
    }
}

#[async_trait]
impl MusicApi for PlexClient {
    /// A server that refused the token is no use until the listener signs in again.
    fn alive(&self) -> bool {
        !self.inner.revoked.load(Ordering::Relaxed)
    }

    fn share_url(&self, _kind: MediaKind, _id: &str) -> Option<String> {
        None
    }

    /// Asks the server for the library, which is what tells a good session from a dead one.
    async fn profile(&self) -> Result<UserProfile> {
        self.request(
            Method::GET,
            &format!("/library/sections/{}", self.inner.stored.section.key),
            &[],
        )
        .await
        .context("cannot reach the plex server")?;
        let user = &self.inner.stored.user;
        Ok(UserProfile {
            id: user.id.clone(),
            display_name: user.name.clone(),
            avatar: user.avatar.clone(),
        })
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let (item, albums, top_tracks) = tokio::join!(
            self.metadata(artist_id),
            self.artist_albums(artist_id),
            self.popular_tracks(artist_id),
        );
        let item = item.context("cannot load the artist")?;
        Ok(Artist {
            cover_large: self.image(item.thumb.as_deref(), 600),
            biography: item.summary.filter(|text| !text.trim().is_empty()),
            name: item.title,
            monthly_listeners: None,
            top_tracks: top_tracks.unwrap_or_default(),
            albums: albums.context("cannot load the albums of the artist")?,
        })
    }

    async fn artist_catalogue(
        &self,
        _artist_id: &str,
        _known: &[Track],
    ) -> Result<ArtistCatalogue> {
        Ok(ArtistCatalogue::default())
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let item = self
            .metadata(artist_id)
            .await
            .context("cannot load the artist")?;
        Ok(ArtistProfile {
            cover_large: self.image(item.thumb.as_deref(), 600),
            biography: item.summary.filter(|text| !text.trim().is_empty()),
            name: item.title,
        })
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let wanted: Vec<String> = ids
            .iter()
            .take(PORTRAIT_LIMIT)
            .map(|id| escape::component(id))
            .collect();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let items = self
            .items(&format!("/library/metadata/{}", wanted.join(",")), &[])
            .await?;
        Ok(items
            .into_iter()
            .filter_map(|item| {
                let image = self.image(item.thumb.as_deref(), 300)?;
                Some((item.rating_key, image))
            })
            .collect())
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let query = Self::rated(TRACKS);
        Ok(self
            .listing(&self.section("/all"), &query)
            .await
            .context("cannot load the favorite songs")?
            .into_iter()
            .map(|item| self.track(item))
            .collect())
    }

    async fn all_tracks(&self) -> Result<Vec<Track>> {
        Self::drain(self.all_tracks_paged().await?).await
    }

    async fn all_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(self.paged(
            self.section("/all"),
            vec![("type", TRACKS.to_owned())],
            Self::track,
        ))
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.change_saved(track_id, saved)
            .await
            .with_context(|| format!("cannot change the rating of {track_id}"))
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let item = self
            .metadata(track_id)
            .await
            .with_context(|| format!("cannot load the song {track_id}"))?;
        Ok(self.track(item))
    }

    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>> {
        Ok(self
            .metadata(track_id)
            .await
            .ok()
            .and_then(|item| item.view_count))
    }

    /// Tells the server where the listener is in the track, which is what shows it as playing on
    /// the server's dashboard. Plex counts the listen itself through `played`.
    async fn report(&self, track_id: &str, report: Report, position: Duration) -> Result<()> {
        let state = match report {
            Report::Playing => "playing",
            Report::Paused => "paused",
            Report::Stopped => "stopped",
        };
        let key = format!("/library/metadata/{track_id}");
        let time = u64::try_from(position.as_millis())
            .unwrap_or(u64::MAX)
            .to_string();
        let duration = self.length_of(track_id).await.to_string();
        self.request(
            Method::GET,
            "/:/timeline",
            &[
                ("ratingKey", track_id),
                ("key", key.as_str()),
                ("identifier", IDENTIFIER),
                ("state", state),
                ("time", time.as_str()),
                ("duration", duration.as_str()),
            ],
        )
        .await
        .with_context(|| format!("cannot report {track_id} as {state}"))?;
        Ok(())
    }

    /// Counts a play. Plex stamps it with the moment it hears of it, so when the listen began is
    /// not sent.
    async fn played(&self, track_id: &str, _at: SystemTime) -> Result<()> {
        self.request(
            Method::GET,
            "/:/scrobble",
            &[("key", track_id), ("identifier", IDENTIFIER)],
        )
        .await
        .with_context(|| format!("cannot record a play of {track_id}"))?;
        Ok(())
    }

    async fn recently_played(&self) -> Result<Vec<Track>> {
        Ok(self
            .ordered(TRACKS, "lastViewedAt:desc", "50")
            .await
            .context("cannot load the recently played songs")?
            .into_iter()
            .filter(|item| item.last_viewed_at.is_some())
            .map(|item| self.track(item))
            .collect())
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        Ok(self
            .items("/playlists", &[("playlistType", "audio")])
            .await
            .context("cannot load the playlists")?
            .into_iter()
            .map(|item| self.playlist_card(item))
            .collect())
    }

    /// Plex has no empty playlist to make: a playlist is born with a track in it. One track of
    /// the library starts it, and is taken out again at once.
    async fn create_playlist(&self, name: &str) -> Result<String> {
        let seed = self
            .random_tracks("1")
            .await
            .context("cannot start the playlist")?
            .into_iter()
            .next()
            .and_then(|track| track.id)
            .context("the library has no song to start a playlist with")?;
        let uri = self.uri_of(&[seed.as_str()]);
        let created: Envelope = self
            .request(
                Method::POST,
                "/playlists",
                &[
                    ("type", "audio"),
                    ("title", name),
                    ("smart", "0"),
                    ("uri", &uri),
                ],
            )
            .await
            .context("cannot create the playlist")?
            .json()
            .await
            .context("cannot read the new playlist")?;
        let id = created
            .container
            .metadata
            .into_iter()
            .next()
            .map(|item| item.rating_key)
            .context("plex did not say which playlist it made")?;
        self.request(Method::DELETE, &format!("/playlists/{id}/items"), &[])
            .await
            .context("cannot empty the new playlist")?;
        Ok(id)
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.request(
            Method::PUT,
            &format!("/playlists/{}", escape::component(playlist_id)),
            &[("title", name)],
        )
        .await
        .context("cannot rename the playlist")?;
        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.request(
            Method::DELETE,
            &format!("/playlists/{}", escape::component(playlist_id)),
            &[],
        )
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

    async fn set_playlist_public(&self, _playlist_id: &str, _public: bool) -> Result<()> {
        anyhow::bail!("plex playlists cannot be made public")
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let uri = self.uri_of(&[track_id]);
        self.request(
            Method::PUT,
            &format!("/playlists/{}/items", escape::component(playlist_id)),
            &[("uri", &uri)],
        )
        .await
        .context("cannot add the track to the playlist")?;
        Ok(())
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let items = self
            .listing(
                &format!("/playlists/{}/items", escape::component(playlist_id)),
                &[],
            )
            .await
            .context("cannot load the playlist")?;
        let entry = items
            .iter()
            .find(|item| item.rating_key == track_id)
            .and_then(|item| item.playlist_item_id)
            .context("track is not in the playlist")?;
        self.request(
            Method::DELETE,
            &format!(
                "/playlists/{}/items/{entry}",
                escape::component(playlist_id)
            ),
            &[],
        )
        .await
        .context("cannot remove the track from the playlist")?;
        Ok(())
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let query = Self::rated(ALBUMS);
        Ok(self
            .listing(&self.section("/all"), &query)
            .await
            .context("cannot load the favorite albums")?
            .into_iter()
            .map(|item| self.album(item))
            .collect())
    }

    async fn all_albums(&self) -> Result<Vec<Album>> {
        Self::drain(self.all_albums_paged().await?).await
    }

    async fn all_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(self.paged(
            self.section("/all"),
            vec![("type", ALBUMS.to_owned())],
            Self::album,
        ))
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.change_saved(album_id, saved)
            .await
            .with_context(|| format!("cannot change the rating of album {album_id}"))
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let query = Self::rated(ARTISTS);
        Ok(self
            .listing(&self.section("/all"), &query)
            .await
            .context("cannot load the favorite artists")?
            .into_iter()
            .map(|item| self.artist_card(item))
            .collect())
    }

    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        Self::drain(self.all_artists_paged().await?).await
    }

    async fn all_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(self.paged(
            self.section("/all"),
            vec![("type", ARTISTS.to_owned())],
            Self::artist_card,
        ))
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.change_saved(artist_id, saved)
            .await
            .with_context(|| format!("cannot change the rating of artist {artist_id}"))
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let (item, tracks) = tokio::join!(self.metadata(album_id), self.album_items(album_id));
        let item = item.with_context(|| format!("cannot load the album {album_id}"))?;
        let tracks = tracks.with_context(|| format!("cannot load the tracks of {album_id}"))?;
        let mut album = self.album(item);
        if album.track_count == 0 {
            album.track_count = tracks.len() as u32;
        }
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        self.album_items(album_id).await
    }

    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        let Some(artist_id) = artist_id else {
            return Ok(AlbumCatalogue::default());
        };
        let (more_by, similar) = tokio::join!(
            self.more_from_artist(album_id, artist_id),
            self.similar_artists(artist_id),
        );
        // Nothing read at all is an error rather than an empty rail, so the catalog does not
        // keep the empty answer for the rest of the session.
        let (more_by, similar) = match (more_by, similar) {
            (Err(error), Err(_)) => return Err(error.context("cannot read any recommendations")),
            pair => pair,
        };
        if let Err(error) = &more_by {
            log::warn!("plex: cannot read more from this artist: {error:#}");
        }
        if let Err(error) = &similar {
            log::debug!("plex: no similar artists: {error:#}");
        }
        let mut seen = HashSet::new();
        let also_like = more_by
            .unwrap_or_default()
            .into_iter()
            .filter(|album| seen.insert(album.id.clone()))
            .collect();
        Ok(AlbumCatalogue {
            also_like,
            similar: similar.unwrap_or_default(),
        })
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let id = escape::component(playlist_id);
        let item_path = format!("/playlists/{id}");
        let entries_path = format!("/playlists/{id}/items");
        let (item, entries) = tokio::join!(self.one(&item_path), self.listing(&entries_path, &[]));
        let item = item.with_context(|| format!("cannot load the playlist {playlist_id}"))?;
        let tracks: Vec<Track> = entries
            .with_context(|| format!("cannot load the tracks of {playlist_id}"))?
            .into_iter()
            .map(|item| self.track(item))
            .collect();
        let mut playlist = self.playlist_card(item);
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

    /// The tracks that sound like `track_id` when the server has analysed the library, and a
    /// handful of random ones when it has not.
    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let similar = self
            .items(
                &format!("/library/metadata/{}/nearest", escape::component(track_id)),
                &[("limit", RADIO_COUNT)],
            )
            .await;
        if let Ok(items) = similar {
            let tracks: Vec<Track> = items.into_iter().map(|item| self.track(item)).collect();
            if !tracks.is_empty() {
                return Ok((tracks, None));
            }
        }
        let random = self
            .random_tracks(RADIO_COUNT)
            .await
            .context("cannot load a radio fallback")?;
        Ok((
            random
                .into_iter()
                .filter(|track| track.id.as_deref() != Some(track_id))
                .collect(),
            None,
        ))
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        Ok(self
            .items(
                &self.section("/search"),
                &[
                    ("type", TRACKS),
                    ("query", query),
                    ("X-Plex-Container-Start", "0"),
                    ("X-Plex-Container-Size", "50"),
                ],
            )
            .await
            .context("cannot search")?
            .into_iter()
            .map(|item| self.track(item))
            .collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        Ok(self
            .items(
                &self.section("/search"),
                &[
                    ("type", ALBUMS),
                    ("query", query),
                    ("X-Plex-Container-Start", "0"),
                    ("X-Plex-Container-Size", "30"),
                ],
            )
            .await
            .context("cannot search albums")?
            .into_iter()
            .map(|item| self.album(item))
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let needle = query.to_lowercase();
        Ok(self
            .playlists()
            .await?
            .into_iter()
            .filter(|playlist| playlist.name.to_lowercase().contains(&needle))
            .collect())
    }

    async fn home(&self) -> Result<HomeFeed> {
        let (recent, newest, random) = tokio::join!(
            self.ordered(TRACKS, "lastViewedAt:desc", HOME_SONGS),
            self.ordered(ALBUMS, "addedAt:desc", HOME_ALBUMS),
            self.random_tracks(HOME_SONGS),
        );
        let listen_again: Vec<GenreItem> = recent
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item.last_viewed_at.is_some())
            .take(10)
            .map(|item| GenreItem::Track(self.track(item)))
            .collect();
        let played = self
            .ordered(ALBUMS, "lastViewedAt:desc", HOME_ALBUMS)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|item| item.last_viewed_at.is_some())
            .map(|item| GenreItem::Album(self.album(item)))
            .collect::<Vec<_>>();
        let newest: Vec<GenreItem> = newest
            .unwrap_or_default()
            .into_iter()
            .map(|item| GenreItem::Album(self.album(item)))
            .collect();

        let mut sections = Vec::new();
        if !newest.is_empty() {
            sections.push(GenreSection {
                title: "Recently added albums".to_owned(),
                items: newest,
            });
        }
        if !played.is_empty() {
            sections.push(GenreSection {
                title: "Recently played albums".to_owned(),
                items: played,
            });
        }
        let random = random.unwrap_or_default();
        Ok(HomeFeed {
            listen_again,
            quick_picks: Some(random.into_iter().take(15).collect()),
            sections,
        })
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        let genres = self
            .container(&self.section("/genre"), &[("type", ALBUMS)])
            .await
            .context("cannot load the genres")?;
        Ok(genres
            .directory
            .into_iter()
            .map(|genre| Genre {
                id: genre.key,
                name: genre.title,
                cover: None,
            })
            .collect())
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        let name = self
            .genres()
            .await
            .ok()
            .and_then(|genres| genres.into_iter().find(|genre| genre.id == genre_id))
            .map_or_else(|| genre_id.to_owned(), |genre| genre.name);
        let albums = self
            .items(
                &self.section("/all"),
                &[
                    ("type", ALBUMS),
                    ("genre", genre_id),
                    ("X-Plex-Container-Start", "0"),
                    ("X-Plex-Container-Size", "50"),
                ],
            )
            .await
            .with_context(|| format!("cannot load the genre {genre_id}"))?;
        let items: Vec<GenreItem> = albums
            .into_iter()
            .map(|item| GenreItem::Album(self.album(item)))
            .collect();
        Ok(GenreDetail {
            sections: match items.is_empty() {
                true => Vec::new(),
                false => vec![GenreSection {
                    title: name.clone(),
                    items,
                }],
            },
            name,
        })
    }
}

/// The length, loudness and file of a track, from what the server says about it.
fn details_of(item: &Item) -> Details {
    let part = item.media.iter().flat_map(|media| &media.part).next();
    let audio = part
        .into_iter()
        .flat_map(|part| &part.stream)
        .find(|stream| stream.stream_type == Some(AUDIO_STREAM));
    Details {
        duration: item.duration.map(Duration::from_millis),
        loudness: audio.and_then(loudness_of),
        part: part
            .map(|part| part.key.clone())
            .filter(|key| !key.is_empty()),
    }
}

/// The loudness Plex measured for a stream. It is LUFS when the server has analysed it, and a
/// ReplayGain from the file's tags when it has only read those, the track's before the album's.
fn loudness_of(stream: &Stream) -> Option<Loudness> {
    if let Some(lufs) = stream.loudness {
        return Some(Loudness {
            lufs,
            peak: stream.peak.filter(|peak| *peak > 0.0),
        });
    }
    let (gain, peak) = match stream.gain {
        Some(track) => (track, stream.peak),
        None => (stream.album_gain?, stream.album_peak),
    };
    Some(Loudness::replay_gain(gain, peak))
}
