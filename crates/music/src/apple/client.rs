//! The Apple Music API, as the web player talks to it.
//!
//! Requests go to `amp-api.music.apple.com`, which is the host music.apple.com itself uses, and
//! not the documented `api.music.apple.com`. The difference matters: the bearer token read off
//! the page is scoped to the former, and the latter answers 401 to every write, so removing a
//! song from the library or deleting a playlist only works here.
//!
//! Two kinds of id run through all of this. A catalog id is a number and can be played, opened
//! and shared. A library id starts with `i.`, `l.`, `r.` or `p.` and belongs to the listener's
//! own copy. Library rows are read with `include=catalog` so both are in hand; the models carry
//! the catalog id, and the library id is looked up again on the rare write that needs it.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use serde_json::Value;

use crate::apple::auth::{self, AGENT};
use crate::apple::wire;
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, Genre, GenreDetail, GenreItem, GenreSection,
    HomeFeed, LibraryItem, LibraryOrder, MediaKind, MusicApi, Playlist, PlaylistDetail,
    SavedArtist, Track, UserProfile,
};

/// The API the web player calls.
const API: &str = "https://amp-api.music.apple.com/v1";

/// How many rows a library or track listing asks for at a time, which is Apple's maximum.
const PAGE: usize = 100;

/// How many search hits to ask for. Apple refuses a search page larger than this outright.
const HITS: usize = 25;

/// How many rows the mixed library landing takes, which is all Apple allows for that one.
const LANDING: usize = 25;

/// How many pages one listing will walk before it stops. A library of a hundred thousand songs
/// is not something to pull into memory in one go.
const PAGES: usize = 40;

/// How many artist ids one batch of portraits asks about.
const PORTRAITS: usize = 50;

/// What the listener is called on their own playlists, until Apple offers a name for them.
const OWNER: &str = "You";

/// How many station tracks one request may ask for. Apple refuses more than ten.
const STATION: usize = 10;

/// How many of those requests make a queue worth having behind a track.
const STATION_PULLS: usize = 3;

/// An Apple Music account.
#[derive(Clone)]
pub struct AppleClient {
    http: reqwest::Client,
    user_token: Arc<str>,
    bearer: Arc<str>,
    storefront: Arc<str>,
    /// The station last built, and the track it was seeded from. Asking twice for the radio of
    /// the same track carries on through that station instead of rolling a new one, which is
    /// what makes a queue that keeps being extended feel like one station rather than several.
    station: Arc<Mutex<Option<(String, String)>>>,
}

impl AppleClient {
    /// Reads the web player's bearer token and the account's storefront, which is also what
    /// proves the user token is still good.
    pub async fn connect(user_token: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(AGENT)
            .build()
            .context("cannot build the apple http client")?;
        let bearer = auth::bearer(&http).await?;
        let client = Self {
            http,
            user_token: user_token.into(),
            bearer: bearer.into(),
            storefront: "us".into(),
            station: Arc::default(),
        };
        let answered = client.get("/me/storefront", &[]).await?;
        let storefront = answered
            .pointer("/data/0/id")
            .and_then(Value::as_str)
            .context("apple named no storefront for this account")?;
        log::info!("apple: signed in, storefront {storefront}");
        Ok(Self {
            storefront: storefront.into(),
            ..client
        })
    }

    /// The client the playback path fetches with, so it shares this one's connections.
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn user_token(&self) -> &str {
        &self.user_token
    }

    pub fn storefront(&self) -> &str {
        &self.storefront
    }

    fn catalog(&self, path: &str) -> String {
        format!("/catalog/{}{path}", self.storefront)
    }

    /// One request against the API. The account token rides on every one of them, and neither
    /// token is ever logged.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method.clone(), format!("{API}{path}"))
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.bearer),
            )
            .header("Music-User-Token", self.user_token.as_ref())
            .header(reqwest::header::ORIGIN, "https://music.apple.com")
            .header(reqwest::header::REFERER, "https://music.apple.com/")
            .query(query);
        request = match body {
            Some(body) => request.json(&body),
            // Apple's gateway refuses a write with no length at all.
            None => request.header(reqwest::header::CONTENT_LENGTH, "0"),
        };
        let response = request
            .send()
            .await
            .with_context(|| format!("cannot reach apple music at {path}"))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .with_context(|| format!("cannot read the apple music answer for {path}"))?;
        let answered: Value = match text.trim().is_empty() {
            true => Value::Null,
            false => serde_json::from_str(&text).unwrap_or(Value::Null),
        };
        if !status.is_success() {
            // Apple says what it disliked in the body, which is the only way to tell a rejected
            // parameter from a rejected account.
            let detail = answered
                .pointer("/errors/0/detail")
                .or_else(|| answered.pointer("/errors/0/title"))
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            bail!("apple music answered {status} for {method} {path}: {detail}");
        }
        Ok(answered)
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.send(reqwest::Method::GET, path, query, None).await
    }

    async fn post(&self, path: &str, query: &[(&str, &str)], body: Option<Value>) -> Result<Value> {
        self.send(reqwest::Method::POST, path, query, body).await
    }

    async fn patch(&self, path: &str, body: Value) -> Result<()> {
        self.send(reqwest::Method::PATCH, path, &[], Some(body))
            .await
            .map(|_| ())
    }

    async fn delete(&self, path: &str, query: &[(&str, &str)]) -> Result<()> {
        self.send(reqwest::Method::DELETE, path, query, None)
            .await
            .map(|_| ())
    }

    /// Walks a paged listing, following the `next` link Apple hands back, and reads every row
    /// with `read`. Stops at [`PAGES`] pages so one enormous library cannot run away with the
    /// process.
    async fn walk<T, F>(
        &self,
        path: &str,
        page: usize,
        query: &[(&str, &str)],
        read: F,
    ) -> Result<Vec<T>>
    where
        F: Fn(&Value) -> Option<T>,
    {
        let limit = page.to_string();
        let mut asked: Vec<(&str, &str)> = query.to_vec();
        asked.push(("limit", &limit));
        let mut collected = Vec::new();
        let mut offset = 0usize;
        for _ in 0..PAGES {
            let at = offset.to_string();
            let mut page = asked.clone();
            if offset > 0 {
                page.push(("offset", &at));
            }
            let answered = self.get(path, &page).await?;
            let rows = answered
                .get("data")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            collected.extend(rows.iter().filter_map(&read));
            offset += rows.len();
            if rows.is_empty() || answered.get("next").is_none() {
                break;
            }
        }
        Ok(collected)
    }

    /// The library id of a catalog resource, if the listener has it. Apple only answers this one
    /// way round, which is why removing something takes two requests.
    async fn mine(&self, kind: &str, id: &str) -> Result<Option<String>> {
        let path = self.catalog(&format!("/{kind}/{id}/library"));
        match self.get(&path, &[]).await {
            Ok(answered) => Ok(answered
                .pointer("/data/0/id")
                .and_then(Value::as_str)
                .map(str::to_owned)),
            // Not in the library at all, which Apple reports as a missing relationship.
            Err(error) if format!("{error}").contains("404") => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// A catalog song, with its artists and album so both are somewhere to go.
    async fn song(&self, id: &str) -> Result<Value> {
        let answered = self
            .get(
                &self.catalog(&format!("/songs/{id}")),
                &[("include[songs]", "artists,albums")],
            )
            .await?;
        answered
            .pointer("/data/0")
            .cloned()
            .with_context(|| format!("apple music has no song {id}"))
    }

    /// The tracks of a catalog playlist, from the first page or from a continuation.
    async fn playlist_page(&self, path: &str) -> Result<(Vec<Track>, Option<String>)> {
        let limit = PAGE.to_string();
        let answered = self
            .get(
                path,
                &[
                    ("limit", &limit),
                    ("include[songs]", "artists,albums"),
                    ("include[library-songs]", "catalog"),
                ],
            )
            .await?;
        let tracks = answered
            .get("data")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| match row.get("type").and_then(Value::as_str) {
                        Some("library-songs") => wire::library_song(row),
                        _ => wire::song(row),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let next = answered
            .get("next")
            .and_then(Value::as_str)
            .map(|next| next.trim_start_matches("/v1").to_owned());
        Ok((tracks, next))
    }

    /// Whether an id belongs to the listener's own library rather than the catalog.
    fn is_mine(id: &str) -> bool {
        let mut letters = id.chars();
        matches!(letters.next(), Some('i' | 'l' | 'r' | 'p')) && letters.next() == Some('.')
    }

    /// Builds the station a track seeds and reads its first tracks.
    ///
    /// This is what the web player's autoplay calls, and the only way Apple hands out the
    /// contents of a station: `/songs/{id}/station` names one but lists nothing.
    async fn seed_station(&self, track_id: &str) -> Result<(String, Vec<Track>)> {
        let limit = STATION.to_string();
        let answered = self
            .post(
                "/me/stations/continuous",
                &[("with", "tracks"), ("limit[results:tracks]", &limit)],
                Some(serde_json::json!({
                    "data": [{ "id": track_id, "type": "songs" }]
                })),
            )
            .await?;
        let station = answered
            .pointer("/results/station/id")
            .and_then(Value::as_str)
            .context("apple built no station for that track")?
            .to_owned();
        let tracks = answered
            .pointer("/results/tracks")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(wire::song).collect())
            .unwrap_or_default();
        Ok((station, tracks))
    }

    /// The next stretch of a station. A station never ends, so this can be asked again as long
    /// as the queue needs filling.
    async fn station_tracks(&self, station: &str) -> Result<Vec<Track>> {
        let limit = STATION.to_string();
        let answered = self
            .post(
                &format!("/me/stations/next-tracks/{station}"),
                &[("limit", &limit)],
                None,
            )
            .await?;
        Ok(answered
            .get("data")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(wire::song).collect())
            .unwrap_or_default())
    }

    /// The station this client is part way through, if it was seeded by `track_id`.
    fn playing_station(&self, track_id: &str) -> Option<String> {
        let held = self.station.lock().ok()?;
        held.as_ref()
            .filter(|(seed, _)| seed == track_id)
            .map(|(_, station)| station.clone())
    }

    fn remember_station(&self, track_id: &str, station: &str) {
        if let Ok(mut held) = self.station.lock() {
            *held = Some((track_id.to_owned(), station.to_owned()));
        }
    }

    /// Charts for the whole storefront or one genre, as sections a page can draw.
    async fn charts(&self, genre: Option<&str>) -> Result<Vec<GenreSection>> {
        let limit = "12";
        let mut query = vec![
            ("types", "songs,albums,playlists"),
            ("limit", limit),
            ("include[songs]", "artists,albums"),
        ];
        if let Some(genre) = genre {
            query.push(("genre", genre));
        }
        let answered = self.get(&self.catalog("/charts"), &query).await?;
        let mut sections = Vec::new();
        for (kind, title) in [
            ("playlists", "Top playlists"),
            ("albums", "Top albums"),
            ("songs", "Top songs"),
        ] {
            let items: Vec<GenreItem> = answered
                .pointer(&format!("/results/{kind}"))
                .and_then(Value::as_array)
                .map(|charts| {
                    charts
                        .iter()
                        .flat_map(|chart| {
                            chart
                                .get("data")
                                .and_then(Value::as_array)
                                .map(Vec::as_slice)
                                .unwrap_or_default()
                        })
                        .filter_map(|row| match kind {
                            "playlists" => wire::playlist(row).map(GenreItem::Playlist),
                            _ => wire::album(row).map(GenreItem::Album),
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !items.is_empty() {
                sections.push(GenreSection {
                    title: title.to_owned(),
                    items,
                });
            }
        }
        Ok(sections)
    }
}

#[async_trait]
impl MusicApi for AppleClient {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let part = match kind {
            MediaKind::Track => "song",
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
        };
        // A library id means nothing to anyone else.
        match Self::is_mine(id) {
            true => None,
            false => Some(format!(
                "https://music.apple.com/{}/{part}/{id}",
                self.storefront
            )),
        }
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(UserProfile {
            id: self.storefront.to_string(),
            display_name: "Apple Music".to_owned(),
        })
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let limit = HITS.to_string();
        let answered = self
            .get(
                &self.catalog("/search"),
                &[
                    ("term", query),
                    ("types", "songs"),
                    ("limit", &limit),
                    ("include[songs]", "artists,albums"),
                ],
            )
            .await?;
        Ok(answered
            .pointer("/results/songs/data")
            .and_then(Value::as_array)
            .map(|songs| songs.iter().filter_map(wire::song).collect())
            .unwrap_or_default())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let limit = HITS.to_string();
        let answered = self
            .get(
                &self.catalog("/search"),
                &[("term", query), ("types", "albums"), ("limit", &limit)],
            )
            .await?;
        Ok(answered
            .pointer("/results/albums/data")
            .and_then(Value::as_array)
            .map(|albums| albums.iter().filter_map(wire::album).collect())
            .unwrap_or_default())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let limit = HITS.to_string();
        let answered = self
            .get(
                &self.catalog("/search"),
                &[("term", query), ("types", "playlists"), ("limit", &limit)],
            )
            .await?;
        Ok(answered
            .pointer("/results/playlists/data")
            .and_then(Value::as_array)
            .map(|playlists| playlists.iter().filter_map(wire::playlist).collect())
            .unwrap_or_default())
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let found = self.song(track_id).await?;
        wire::song(&found).with_context(|| format!("cannot read the apple song {track_id}"))
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    /// The songs the listener has added. Only the ones with a catalog id are listed: an upload
    /// has no catalog encode behind it, and this path plays catalog tracks.
    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        self.walk(
            "/me/library/songs",
            PAGE,
            &[("include", "catalog"), ("include[songs]", "artists,albums")],
            wire::library_song,
        )
        .await
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        self.walk(
            "/me/library/albums",
            PAGE,
            &[("include", "catalog")],
            wire::library_album,
        )
        .await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        self.walk(
            "/me/library/artists",
            PAGE,
            &[("include", "catalog")],
            wire::saved_artist,
        )
        .await
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.walk("/me/library/playlists", PAGE, &[], |row| {
            wire::library_playlist(row, OWNER)
        })
        .await
    }

    /// The mixed library landing, newest first. Apple only orders it one way, so the other
    /// orders are left to the separate collections.
    async fn library_items(&self, order: LibraryOrder) -> Result<Option<Vec<LibraryItem>>> {
        if !matches!(order, LibraryOrder::Recents | LibraryOrder::RecentlyAdded) {
            return Ok(None);
        }
        let items = self
            .walk(
                "/me/library/recently-added",
                LANDING,
                &[("include", "catalog")],
                |row| wire::library_item(row, OWNER),
            )
            .await?;
        Ok(Some(items))
    }

    /// Adding works on a catalog id. Removing needs the library id, which only the catalog
    /// resource can point at, so it is one request to find and one to remove.
    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        match saved {
            true => self
                .post("/me/library", &[("ids[songs]", track_id)], None)
                .await
                .map(|_| ()),
            false => match self.mine("songs", track_id).await? {
                Some(mine) => self.delete(&format!("/me/library/songs/{mine}"), &[]).await,
                None => Ok(()),
            },
        }
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        match saved {
            true => self
                .post("/me/library", &[("ids[albums]", album_id)], None)
                .await
                .map(|_| ()),
            false => match self.mine("albums", album_id).await? {
                Some(mine) => {
                    self.delete(&format!("/me/library/albums/{mine}"), &[])
                        .await
                }
                None => Ok(()),
            },
        }
    }

    /// Apple has no followed artists. A library artist is one whose music the listener added,
    /// so there is nothing to set that adding an album does not already do.
    async fn set_artist_saved(&self, _artist_id: &str, _saved: bool) -> Result<()> {
        bail!("apple music has no followed artists, only artists whose music you added")
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let (path, query): (String, Vec<(&str, &str)>) = match Self::is_mine(album_id) {
            true => (
                format!("/me/library/albums/{album_id}"),
                vec![("include", "tracks,catalog")],
            ),
            false => (
                self.catalog(&format!("/albums/{album_id}")),
                vec![
                    ("include", "tracks,artists"),
                    ("include[songs]", "artists,albums"),
                ],
            ),
        };
        let answered = self.get(&path, &query).await?;
        let found = answered
            .pointer("/data/0")
            .context("apple music has no such album")?;
        // A library album's own rows are library songs; a catalog album's are catalog songs.
        let tracks = found
            .pointer("/relationships/tracks/data")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| match row.get("type").and_then(Value::as_str) {
                        Some("library-songs") => wire::library_song(row),
                        _ => wire::song(row),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let album = match Self::is_mine(album_id) {
            true => wire::library_album(found).or_else(|| wire::album(found)),
            false => wire::album(found),
        };
        Ok(AlbumDetail {
            album: album.context("cannot read the apple album")?,
            tracks,
        })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let id = match Self::is_mine(artist_id) {
            true => self
                .get(
                    &format!("/me/library/artists/{artist_id}"),
                    &[("include", "catalog")],
                )
                .await?
                .pointer("/data/0")
                .and_then(wire::catalog)
                .and_then(|found| found.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .context("this library artist has no page in the catalog")?,
            false => artist_id.to_owned(),
        };
        let answered = self
            .get(
                &self.catalog(&format!("/artists/{id}")),
                &[
                    ("views", "top-songs,full-albums,singles"),
                    ("include[songs]", "artists,albums"),
                ],
            )
            .await?;
        answered
            .pointer("/data/0")
            .and_then(wire::artist)
            .with_context(|| format!("cannot read the apple artist {id}"))
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let answered = self
            .get(&self.catalog(&format!("/artists/{artist_id}")), &[])
            .await?;
        answered
            .pointer("/data/0")
            .and_then(wire::artist_profile)
            .with_context(|| format!("cannot read the apple artist {artist_id}"))
    }

    /// Portraits for a batch of artists, which the library pages draw beside their names.
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let mut found = HashMap::new();
        for batch in ids
            .iter()
            .filter(|id| !Self::is_mine(id))
            .collect::<Vec<_>>()
            .chunks(PORTRAITS)
        {
            let joined = batch
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(",");
            let answered = self
                .get(&self.catalog("/artists"), &[("ids", &joined)])
                .await?;
            let rows = answered
                .get("data")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            for row in rows {
                let Some(id) = row.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(art) = row
                    .get("attributes")
                    .and_then(|attributes| wire::artwork(attributes, wire::ART))
                {
                    found.insert(id.to_owned(), art);
                }
            }
        }
        Ok(found)
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let mine = Self::is_mine(playlist_id);
        let path = match mine {
            true => format!("/me/library/playlists/{playlist_id}"),
            false => self.catalog(&format!("/playlists/{playlist_id}")),
        };
        let answered = self.get(&path, &[]).await?;
        let found = answered
            .pointer("/data/0")
            .context("apple music has no such playlist")?;
        let playlist = match mine {
            true => wire::library_playlist(found, OWNER),
            false => wire::playlist(found),
        }
        .context("cannot read the apple playlist")?;
        let (tracks, continuation) = self.playlist_page(&format!("{path}/tracks")).await?;
        Ok(PlaylistDetail {
            playlist: Playlist {
                track_count: tracks.len() as u32,
                ..playlist
            },
            tracks,
            continuation,
        })
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let path = match Self::is_mine(playlist_id) {
            true => format!("/me/library/playlists/{playlist_id}/tracks"),
            false => self.catalog(&format!("/playlists/{playlist_id}/tracks")),
        };
        let mut collected = Vec::new();
        let mut next = Some(path);
        for _ in 0..PAGES {
            let Some(path) = next else { break };
            let (tracks, following) = self.playlist_page(&path).await?;
            collected.extend(tracks);
            next = following;
        }
        Ok(collected)
    }

    /// One more page of a playlist. The continuation is the link Apple handed back.
    async fn playlist_continuation(
        &self,
        continuation: &str,
    ) -> Result<(Vec<Track>, Option<String>)> {
        self.playlist_page(continuation).await
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let path = match Self::is_mine(playlist_id) {
            true => format!("/me/library/playlists/{playlist_id}/tracks"),
            false => self.catalog(&format!("/playlists/{playlist_id}/tracks")),
        };
        let (tracks, _) = self.playlist_page(&path).await?;
        Ok(crate::distinct_covers(&tracks, wanted))
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let answered = self
            .post(
                "/me/library/playlists",
                &[],
                Some(serde_json::json!({ "attributes": { "name": name } })),
            )
            .await?;
        answered
            .pointer("/data/0/id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .context("apple music named no new playlist")
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.patch(
            &format!("/me/library/playlists/{playlist_id}"),
            serde_json::json!({ "attributes": { "name": name } }),
        )
        .await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.delete(&format!("/me/library/playlists/{playlist_id}"), &[])
            .await
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        self.patch(
            &format!("/me/library/playlists/{playlist_id}"),
            serde_json::json!({ "attributes": { "isPublic": public } }),
        )
        .await
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        self.post("/me/library", &[("ids[playlists]", playlist_id)], None)
            .await
            .map(|_| ())
    }

    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        let id = match Self::is_mine(playlist_id) {
            true => Some(playlist_id.to_owned()),
            false => self.mine("playlists", playlist_id).await?,
        };
        match id {
            Some(id) => {
                self.delete(&format!("/me/library/playlists/{id}"), &[])
                    .await
            }
            None => Ok(()),
        }
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.post(
            &format!("/me/library/playlists/{playlist_id}/tracks"),
            &[],
            Some(serde_json::json!({
                "data": [{ "id": track_id, "type": "songs" }]
            })),
        )
        .await
        .map(|_| ())
    }

    /// Removing takes the playlist's own row id rather than the catalog id, so the playlist is
    /// read first to find which row carries this track.
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let limit = PAGE.to_string();
        let mut row = None;
        for page in 0..PAGES {
            let offset = (page * PAGE).to_string();
            let answered = self
                .get(
                    &format!("/me/library/playlists/{playlist_id}/tracks"),
                    &[("limit", &limit), ("offset", &offset)],
                )
                .await?;
            let rows = answered
                .get("data")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default();
            row = rows
                .iter()
                .find(|row| {
                    row.pointer("/attributes/playParams/catalogId")
                        .and_then(Value::as_str)
                        == Some(track_id)
                })
                .and_then(|row| row.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            if row.is_some() || rows.is_empty() || answered.get("next").is_none() {
                break;
            }
        }
        let Some(row) = row else {
            bail!("that track is not in the playlist any more");
        };
        self.delete(
            &format!("/me/library/playlists/{playlist_id}/tracks"),
            &[("ids[library-songs]", &row), ("mode", "all")],
        )
        .await
    }

    /// What Apple made for this listener: their recommendation groups, and the storefront
    /// charts underneath so the page is never empty.
    async fn home(&self) -> Result<HomeFeed> {
        let answered = self.get("/me/recommendations", &[("limit", "12")]).await?;
        let mut sections = Vec::new();
        for group in answered
            .get("data")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let title = group
                .pointer("/attributes/title/stringForDisplay")
                .and_then(Value::as_str)
                .unwrap_or("For you")
                .to_owned();
            let items: Vec<GenreItem> = group
                .pointer("/relationships/contents/data")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| match row.get("type").and_then(Value::as_str) {
                            Some("playlists") => wire::playlist(row).map(GenreItem::Playlist),
                            Some("albums") => wire::album(row).map(GenreItem::Album),
                            _ => None,
                        })
                        .collect()
                })
                .unwrap_or_default();
            if !items.is_empty() {
                sections.push(GenreSection { title, items });
            }
        }
        if sections.is_empty() {
            sections = self.charts(None).await.unwrap_or_default();
        }
        Ok(HomeFeed {
            listen_again: Vec::new(),
            quick_picks: None,
            sections,
        })
    }

    /// The genres of the storefront, without the root that parents them all.
    async fn genres(&self) -> Result<Vec<Genre>> {
        let genres = self
            .walk(&self.catalog("/genres"), PAGE, &[], |row| {
                let attributes = row.get("attributes")?;
                // Every real genre hangs off "Music", which is not one itself.
                attributes.get("parentId")?;
                Some(Genre {
                    id: row.get("id")?.as_str()?.to_owned(),
                    name: wire::text(attributes, "name")?,
                    cover: None,
                })
            })
            .await?;
        Ok(genres)
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        let answered = self
            .get(&self.catalog(&format!("/genres/{genre_id}")), &[])
            .await?;
        let name = answered
            .pointer("/data/0/attributes/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(GenreDetail {
            name,
            sections: self.charts(Some(genre_id)).await?,
        })
    }

    /// The station a track seeds, as a queue's worth of tracks.
    ///
    /// Asked again for the same track, this carries on through the station it already built
    /// rather than starting another, so a queue that keeps being extended stays one station.
    /// Apple hands out ten tracks a time, and repeats are dropped because a station is free to
    /// come back to a song this queue already holds.
    async fn track_radio(&self, track_id: &str) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        let station = match self.playing_station(track_id) {
            Some(station) => station,
            None => {
                let (station, first) = self.seed_station(track_id).await?;
                self.remember_station(track_id, &station);
                tracks = first;
                station
            }
        };
        for _ in 0..STATION_PULLS {
            if tracks.len() >= STATION * STATION_PULLS {
                break;
            }
            let more = self.station_tracks(&station).await?;
            if more.is_empty() {
                break;
            }
            tracks.extend(more);
        }
        // A station is free to come back to a song, and the seed is already playing.
        let mut seen = HashSet::new();
        tracks.retain(|track| match track.id.as_deref() {
            Some(id) => id != track_id && seen.insert(id.to_owned()),
            None => false,
        });
        log::debug!("apple: station {station} gave {} tracks", tracks.len());
        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_ids_are_told_apart_from_catalog_ones() {
        assert!(AppleClient::is_mine("i.AWPV19dTN2g507K"));
        assert!(AppleClient::is_mine("l.xBnV5i4"));
        assert!(AppleClient::is_mine("r.CrpxQdf"));
        assert!(AppleClient::is_mine("p.XMrmpaOfO8bWAm4"));
        assert!(!AppleClient::is_mine("1440857781"));
        assert!(!AppleClient::is_mine("pl.u-gxblkm0TbL9xYjk"));
        assert!(!AppleClient::is_mine(""));
        assert!(!AppleClient::is_mine("i"));
    }
}
