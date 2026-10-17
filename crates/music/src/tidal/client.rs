//! Tidal data access over `api.tidal.com`, the api the official players use: `v1` for nearly
//! everything, and `v2` for the home feed and a playlist's visibility. Every call carries the
//! account's country, which decides what the catalog holds, and a bearer token renewed in
//! place before it runs out.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use futures::stream::{self, StreamExt as _};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::engine::Loudness;
use crate::tidal::auth::{self, Credentials};
use crate::tidal::manifest::Manifest;
use crate::tidal::{lock, wire};
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, GenreItem,
    GenreSection, HomeFeed, MediaKind, MusicApi, Page, Pages, Playlist, PlaylistDetail,
    SUGGESTIONS, SavedArtist, SignInFailure, SignInProblem, Track, UserProfile, distinct_covers,
    escape,
};

const V1: &str = "https://api.tidal.com/v1";
const V2: &str = "https://api.tidal.com/v2";
/// Tidal's published api, which removes a playlist row by its id rather than its position.
const OPENAPI: &str = "https://openapi.tidal.com/v2";

/// The web player version every call names. The `v2` home feed refuses a call without one
/// with a bare `400`; the rest of the api ignores it. It is the version tidalapi sends, and
/// when Tidal retires it the home feed is what stops loading: the current one is in the
/// `x-tidal-client-version` header the web player at listen.tidal.com sends.
const CLIENT_VERSION: &str = "2025.7.16";

/// The best quality there is. Tidal answers with the best the account and the track allow,
/// which may be less.
const QUALITY: &str = "HI_RES_LOSSLESS";

/// How many rows a listing asks for at once. Tidal refuses more than this on most listings.
const PAGE: usize = 100;
/// Favorites are listed a thousand at a time, which Tidal allows on those listings, so a
/// large library takes a handful of requests rather than dozens.
const FAVORITES_PAGE: usize = 1000;
/// The listing of the listener's own and followed playlists refuses more than fifty.
const PLAYLIST_PAGE: usize = 50;
/// The most pages one listing is read to, so an enormous library cannot run away.
const PAGES: usize = 200;
/// How many pages a library listing buffers ahead of the page reading them.
const FAN: usize = 4;

/// Favorites newest first. Without it Tidal lists them in no useful order.
const LATEST: &[(&str, &str)] = &[("order", "DATE"), ("orderDirection", "DESC")];

/// How many similar artists lend their releases to a thin album rail, and how many each lends.
const SIMILAR_ARTISTS: usize = 6;
const SIMILAR_RELEASES: usize = 2;

/// How many times a playlist write is run while it finds the playlist stale, and how much
/// longer each wait between runs is, so a write that followed the last within moments finds
/// Tidal caught up. Three runs wait three seconds at most.
const EDIT_ATTEMPTS: u32 = 3;
const EDIT_SETTLE: Duration = Duration::from_secs(1);

/// How many artist lookups run at once, for portraits and for an album's recommendations.
const LOOKUPS: usize = 4;
/// How many tracks a station asks for.
const RADIO: &str = "100";
/// How many results a search asks for.
const SEARCH: &str = "50";

/// The longest a call waits out a `429` before asking once more.
const BACKOFF_CEILING: Duration = Duration::from_secs(10);

/// How long a connection may take to open, and how long a request may wait between two reads.
const CONNECT: Duration = Duration::from_secs(10);
const STALL: Duration = Duration::from_secs(20);

/// The sub-statuses of a `401` that a renewed token clears: invalid, and expired.
const TOKEN_TROUBLE: [u64; 2] = [11002, 11003];

/// A signed-in Tidal session: the api calls behind [`MusicApi`] and the playback lookup. Clones
/// share one session, so a token renewed by one is the token every clone uses next.
#[derive(Clone)]
pub struct TidalClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    credentials: RwLock<Credentials>,
    /// Held while a token is renewed, so calls that find it stale at the same moment renew it
    /// once between them.
    renewing: tokio::sync::Mutex<()>,
    /// The sign-out count the session began at, so its renewals stop being kept once the
    /// listener signs out.
    generation: u64,
    /// Artist portraits already looked up, so a search that ranks the same artists on every
    /// keystroke does not ask for them again.
    portraits: Mutex<HashMap<String, String>>,
    /// Tidal has refused the session for good, which is what `alive` reports so the app can
    /// send the listener back to sign in.
    revoked: AtomicBool,
}

/// What a call sends in its body.
#[derive(Clone, Copy)]
enum Payload<'a> {
    Empty,
    Form(&'a [(&'a str, &'a str)]),
    /// A JSON:API document, which is what Tidal's official api reads.
    Json(&'a Value),
}

/// Tidal refusing a playlist write because the `ETag` it carried is no longer the playlist's.
#[derive(Debug)]
struct Stale;

/// What a playback lookup answers: where the bytes are, and what Tidal says about the track.
pub struct Playback {
    pub manifest: Manifest,
    /// The quality Tidal granted, which may be below the one asked for.
    pub quality: String,
    pub duration: Option<Duration>,
    pub loudness: Option<Loudness>,
}

impl std::fmt::Display for Stale {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the playlist changed since its etag was read")
    }
}

impl std::error::Error for Stale {}

impl TidalClient {
    /// Opens a session with stored or freshly approved credentials, renewing the token first
    /// when it is about to run out, and checks it with Tidal before handing it back.
    pub(crate) async fn connect(
        http: reqwest::Client,
        credentials: Credentials,
        generation: u64,
    ) -> Result<Self> {
        let credentials = match credentials.stale() {
            true => {
                let renewed = auth::refresh(&http, &credentials).await?;
                if let Err(error) = auth::keep(generation, &renewed) {
                    log::warn!("tidal: cannot keep the renewed session: {error:#}");
                }
                renewed
            }
            false => credentials,
        };
        let client = Self {
            inner: Arc::new(Inner {
                http,
                credentials: RwLock::new(credentials),
                renewing: tokio::sync::Mutex::new(()),
                generation,
                portraits: Mutex::new(HashMap::new()),
                revoked: AtomicBool::new(false),
            }),
        };
        let session = client
            .get("/sessions", &[])
            .await
            .context("tidal refused the session")?;
        if let Some(country) = wire::text(&session, "countryCode") {
            client.inner.credentials.write().await.country_code = country.to_owned();
        }
        Ok(client)
    }

    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    async fn user_id(&self) -> String {
        self.inner.credentials.read().await.user_id.clone()
    }

    /// The access token, renewed first when it is about to run out or when it is `rejected`,
    /// the token Tidal has just refused. A token another call already renewed is used as it is.
    async fn token(&self, rejected: Option<&str>) -> Result<String> {
        let usable = |credentials: &Credentials| {
            !credentials.stale() && rejected != Some(credentials.access_token.as_str())
        };
        {
            let credentials = self.inner.credentials.read().await;
            if usable(&credentials) {
                return Ok(credentials.access_token.clone());
            }
        }
        let _renewing = self.inner.renewing.lock().await;
        let current = self.inner.credentials.read().await.clone();
        // Someone else renewed it while this call waited for the lock.
        if usable(&current) {
            return Ok(current.access_token);
        }
        let renewed = auth::refresh(&self.inner.http, &current)
            .await
            .inspect_err(|error| self.revoke_if_refused(error))?;
        if let Err(error) = auth::keep(self.inner.generation, &renewed) {
            log::warn!("tidal: cannot keep the renewed session: {error:#}");
        }
        let token = renewed.access_token.clone();
        *self.inner.credentials.write().await = renewed;
        Ok(token)
    }

    /// One call, answered with its response once it succeeds. A refused token is renewed and
    /// the call asked once more, and so is a call Tidal asks to be slowed down. A token Tidal
    /// still refuses after renewing is the sign-in being gone.
    async fn call(
        &self,
        method: Method,
        url: &str,
        query: &[(&str, &str)],
        payload: Payload<'_>,
        etag: Option<&str>,
    ) -> Result<reqwest::Response> {
        let country = self.inner.credentials.read().await.country_code.clone();
        let mut rejected: Option<String> = None;
        let mut slowed = false;
        loop {
            let token = self.token(rejected.as_deref()).await?;
            let mut request = self
                .inner
                .http
                .request(method.clone(), url)
                .bearer_auth(&token)
                .header("x-tidal-client-version", CLIENT_VERSION)
                .query(&[("countryCode", country.as_str())])
                .query(query);
            request = match payload {
                Payload::Empty => request,
                Payload::Form(form) => request.form(form),
                Payload::Json(document) => request
                    .header(reqwest::header::CONTENT_TYPE, "application/vnd.api+json")
                    .body(document.to_string()),
            };
            if let Some(etag) = etag {
                request = request.header("If-None-Match", etag);
            }
            let response = request
                .send()
                .await
                .with_context(|| format!("cannot reach tidal for {}", path_of(url)))?;
            let status = response.status();
            if status.is_success() {
                return Ok(response);
            }
            if status == StatusCode::TOO_MANY_REQUESTS && !slowed {
                let wait = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|after| after.to_str().ok()?.parse().ok())
                    .map_or(Duration::from_secs(2), Duration::from_secs)
                    .min(BACKOFF_CEILING);
                log::debug!("tidal: slowed down for {}ms", wait.as_millis());
                tokio::time::sleep(wait).await;
                slowed = true;
                continue;
            }
            let refusal: Value = response.json().await.unwrap_or(Value::Null);
            let sub_status = wire::number(&refusal, "subStatus").unwrap_or(0);
            if status == StatusCode::UNAUTHORIZED && TOKEN_TROUBLE.contains(&sub_status) {
                if rejected.is_none() {
                    rejected = Some(token);
                    continue;
                }
                self.inner.revoked.store(true, Ordering::Relaxed);
                return Err(SignInFailure(SignInProblem::Credentials))
                    .with_context(|| format!("tidal refused the session for {}", path_of(url)));
            }
            let message = wire::text(&refusal, "userMessage").unwrap_or("no reason given");
            if auth::transient(status) {
                return Err(auth::Unavailable).with_context(|| {
                    format!(
                        "tidal refused {}: {message} ({status}, {sub_status})",
                        path_of(url)
                    )
                });
            }
            if status == StatusCode::PRECONDITION_FAILED {
                return Err(Stale).with_context(|| {
                    format!(
                        "tidal refused {}: {message} ({status}, {sub_status})",
                        path_of(url)
                    )
                });
            }
            bail!(
                "tidal refused {}: {message} ({status}, {sub_status})",
                path_of(url)
            );
        }
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.read(V1, path, query).await
    }

    async fn get_v2(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.read(V2, path, query).await
    }

    async fn read(&self, base: &str, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.call(
            Method::GET,
            &format!("{base}{path}"),
            query,
            Payload::Empty,
            None,
        )
        .await?
        .json()
        .await
        .with_context(|| format!("cannot read the tidal answer for {path}"))
    }

    /// A write that answers with nothing worth reading.
    async fn send(
        &self,
        method: Method,
        path: &str,
        form: Option<&[(&str, &str)]>,
        etag: Option<&str>,
    ) -> Result<()> {
        let payload = form.map_or(Payload::Empty, Payload::Form);
        self.call(method, &format!("{V1}{path}"), &[], payload, etag)
            .await?;
        Ok(())
    }

    /// A playlist's current `ETag`, which every write to it has to carry.
    async fn etag(&self, playlist_id: &str) -> Result<String> {
        let response = self
            .call(
                Method::GET,
                &format!("{V1}/playlists/{}", escape::component(playlist_id)),
                &[],
                Payload::Empty,
                None,
            )
            .await?;
        response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|etag| etag.to_str().ok())
            .map(str::to_owned)
            .context("tidal answered the playlist with no etag")
    }

    /// Runs one write to a playlist, again after a growing wait while it finds the playlist
    /// stale. That happens when the playlist changed in between, on another device say, and
    /// when a write follows the last within moments: for a little while after a change,
    /// Tidal's reads all answer from before it. Each run reads the playlist afresh, and is told
    /// whether it is the last, since a write may treat what it still finds then as settled.
    async fn edit_playlist<F, Fut>(&self, write: F) -> Result<()>
    where
        F: Fn(bool) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        let mut attempt = 1;
        loop {
            match write(attempt == EDIT_ATTEMPTS).await {
                Err(error)
                    if attempt < EDIT_ATTEMPTS
                        && error.chain().any(|cause| cause.is::<Stale>()) =>
                {
                    log::debug!(
                        "tidal: the playlist looked stale to a write, asking again: {error:#}"
                    );
                    tokio::time::sleep(EDIT_SETTLE * attempt).await;
                    attempt += 1;
                }
                done => return done,
            }
        }
    }

    /// Takes every copy of a track out of a playlist, naming each by the id of its row rather
    /// than its position. Positions are unsafe here: Tidal applies a change a moment after
    /// answering for it, so a position read just after an edit can name whatever track has
    /// moved into it, where a row id only ever names its own row.
    ///
    /// Tidal's reads lag a moment behind a change too, so a track the listing does not show
    /// yet is taken for a stale listing until `last`, when it is taken as already gone.
    async fn remove_copies(&self, playlist_id: &str, track_id: &str, last: bool) -> Result<()> {
        let path = format!("/playlists/{}/items", escape::component(playlist_id));
        let rows = self
            .all(&path, &[], PAGE, |page| {
                wire::items(page)
                    .iter()
                    .map(wire::unwrap)
                    .filter(|item| item.get("id").and_then(wire::id).as_deref() == Some(track_id))
                    .filter_map(|item| wire::text(item, "itemUuid").map(str::to_owned))
                    .collect()
            })
            .await
            .context("cannot read the playlist")?;
        if rows.is_empty() {
            return match last {
                true => Ok(()),
                false => Err(Stale).context("the track is not in the playlist listing yet"),
            };
        }
        let data: Vec<Value> = rows
            .iter()
            .map(|row| json!({ "id": track_id, "type": "tracks", "meta": { "itemId": row } }))
            .collect();
        self.call(
            Method::DELETE,
            &format!(
                "{OPENAPI}/playlists/{}/relationships/items",
                escape::component(playlist_id)
            ),
            &[],
            Payload::Json(&json!({ "data": data })),
            None,
        )
        .await?;
        Ok(())
    }

    /// Every row of a listing, read through `read`, a page at a time up to `PAGES` pages.
    async fn all<T>(
        &self,
        path: &str,
        query: &[(&str, &str)],
        size: usize,
        read: impl Fn(&Value) -> Vec<T>,
    ) -> Result<Vec<T>> {
        let mut rows = Vec::new();
        let limit = size.to_string();
        for page in 0..PAGES {
            let offset = (page * size).to_string();
            let mut asked: Vec<(&str, &str)> = query.to_vec();
            asked.extend([("limit", limit.as_str()), ("offset", offset.as_str())]);
            let answer = self.get(path, &asked).await?;
            rows.extend(read(&answer));
            if exhausted(&answer, page, size) {
                break;
            }
        }
        Ok(rows)
    }

    /// A listing handed out `size` rows at a time on a channel that stays open until the last
    /// page. Every page carries the total Tidal reports.
    fn paged<T: Send + 'static>(
        &self,
        path: String,
        query: Vec<(&'static str, &'static str)>,
        size: usize,
        read: fn(&Value) -> Vec<T>,
    ) -> Pages<T> {
        let (sink, pages) = tokio::sync::mpsc::channel(FAN);
        let client = self.clone();
        tokio::spawn(async move {
            let limit = size.to_string();
            for page in 0..PAGES {
                let offset = (page * size).to_string();
                let mut asked: Vec<(&str, &str)> = query.clone();
                asked.extend([("limit", limit.as_str()), ("offset", offset.as_str())]);
                let answer = match client.get(&path, &asked).await {
                    Ok(answer) => answer,
                    Err(error) => {
                        sink.send(Err(error)).await.ok();
                        return;
                    }
                };
                let last = exhausted(&answer, page, size);
                let sent = sink
                    .send(Ok(Page {
                        total: wire::count(&answer, "totalNumberOfItems"),
                        items: read(&answer),
                    }))
                    .await;
                if sent.is_err() || last {
                    return;
                }
            }
        });
        pages
    }

    async fn favorites_path(&self, kind: &str) -> String {
        format!(
            "/users/{}/favorites/{kind}",
            escape::component(&self.user_id().await)
        )
    }

    /// Stars or unstars something. `field` is the form field Tidal reads the id from.
    async fn favorite(&self, kind: &str, field: &str, id: &str, saved: bool) -> Result<()> {
        let path = self.favorites_path(kind).await;
        match saved {
            true => {
                self.send(Method::POST, &path, Some(&[(field, id)]), None)
                    .await
            }
            false => {
                self.send(
                    Method::DELETE,
                    &format!("{path}/{}", escape::component(id)),
                    None,
                    None,
                )
                .await
            }
        }
        .with_context(|| format!("cannot change the tidal favorite {id}"))
    }

    /// The first `limit` of an artist's releases of one kind, for a rail that only shows a few.
    /// `filter` is as [`Self::discography`] takes it.
    async fn releases(
        &self,
        artist_id: &str,
        filter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Album>> {
        let limit = limit.to_string();
        let mut query = vec![("limit", limit.as_str())];
        query.extend(filter.map(|filter| ("filter", filter)));
        let page = self.get(&releases_path(artist_id), &query).await?;
        Ok(wire::album_list(&page))
    }

    /// Every one of an artist's releases of one kind: albums with no `filter`,
    /// `EPSANDSINGLES`, or `COMPILATIONS`, which is what Tidal lists as appearing on.
    async fn discography(&self, artist_id: &str, filter: Option<&str>) -> Result<Vec<Album>> {
        let query: Vec<(&str, &str)> = filter
            .map(|filter| ("filter", filter))
            .into_iter()
            .collect();
        self.all(&releases_path(artist_id), &query, PAGE, wire::album_list)
            .await
    }

    /// An artist's biography without Tidal's markup. Many artists have none, which Tidal answers
    /// with a 404, so a missing one is not an error.
    async fn biography(&self, artist_id: &str) -> Option<String> {
        let bio = self
            .get(
                &format!("/artists/{}/bio", escape::component(artist_id)),
                &[],
            )
            .await
            .ok()?;
        wire::text(&bio, "text").map(wire::plain_bio)
    }

    /// Up to `SUGGESTIONS` artists Tidal lists as similar.
    async fn similar_artists(&self, artist_id: &str) -> Result<Vec<SavedArtist>> {
        let page = self
            .get(
                &format!("/artists/{}/similar", escape::component(artist_id)),
                &[("limit", &SUGGESTIONS.to_string())],
            )
            .await?;
        Ok(wire::artist_list(&page))
    }

    /// One kind of search result: `types` is what Tidal is asked for, and `key` the list in its
    /// answer that holds them. An answer without that list is no results.
    async fn search_kind(&self, query: &str, types: &str, key: &str) -> Result<Value> {
        let mut found = self
            .get(
                "/search",
                &[("query", query), ("types", types), ("limit", SEARCH)],
            )
            .await
            .with_context(|| format!("cannot search tidal for {key}"))?;
        Ok(found.get_mut(key).map(Value::take).unwrap_or_default())
    }

    /// The mixes the listener saved, which Tidal keeps apart from playlists and lists a cursor
    /// at a time.
    async fn saved_mixes(&self) -> Result<Vec<Playlist>> {
        let mut mixes = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..PAGES {
            let mut query = vec![("limit", "50")];
            if let Some(cursor) = &cursor {
                query.push(("cursor", cursor.as_str()));
            }
            let page = self.get_v2("/favorites/mixes", &query).await?;
            mixes.extend(wire::items(&page).iter().filter_map(wire::mix));
            cursor = wire::text(&page, "cursor").map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(mixes)
    }

    /// Saves a mix to the listener's collection, or takes it out.
    async fn save_mix(&self, mix_id: &str, saved: bool) -> Result<()> {
        let change = match saved {
            true => "add",
            false => "remove",
        };
        self.call(
            Method::PUT,
            &format!("{V2}/favorites/mixes/{change}"),
            &[("mixIds", mix_id), ("onArtifactNotFound", "FAIL")],
            Payload::Empty,
            None,
        )
        .await
        .with_context(|| format!("cannot change the saved mix {mix_id}"))?;
        Ok(())
    }

    /// Marks the session as gone when Tidal refused to renew it.
    fn revoke_if_refused(&self, error: &anyhow::Error) {
        if auth::refused(error) {
            self.inner.revoked.store(true, Ordering::Relaxed);
        }
    }

    /// A mix, through the page Tidal draws it on: a header naming it, and its tracks.
    async fn mix(&self, mix_id: &str) -> Result<PlaylistDetail> {
        let page = self
            .get(
                "/pages/mix",
                &[("mixId", mix_id), ("deviceType", "BROWSER")],
            )
            .await
            .with_context(|| format!("cannot load the tidal mix {mix_id}"))?;
        let modules: Vec<&Value> = page
            .get("rows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| row.get("modules").and_then(Value::as_array))
            .flatten()
            .collect();
        let tracks: Vec<Track> = modules
            .iter()
            .filter(|module| wire::text(module, "type") == Some("TRACK_LIST"))
            .filter_map(|module| module.get("pagedList"))
            .flat_map(wire::track_list)
            .collect();
        let mut playlist = modules
            .iter()
            .find_map(|module| module.get("mix").and_then(wire::mix))
            .unwrap_or_else(|| Playlist {
                id: format!("{}{mix_id}", wire::MIX),
                name: wire::text(&page, "title").unwrap_or_default().to_owned(),
                owner: "TIDAL".to_owned(),
                owner_id: String::new(),
                owned: false,
                collaborative: false,
                blend: false,
                public: true,
                cover: None,
                track_count: 0,
                modified_at: None,
            });
        playlist.track_count = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        if playlist.cover.is_none() {
            playlist.cover = tracks.iter().find_map(|track| track.cover.clone());
        }
        Ok(PlaylistDetail {
            playlist,
            tracks,
            continuation: None,
        })
    }

    /// The tracks of the mix Tidal keeps for a track, for a track whose station it will not
    /// serve directly.
    async fn track_mix(&self, track_id: &str) -> Result<Vec<Track>> {
        let mix = self
            .get(&format!("/tracks/{}/mix", escape::component(track_id)), &[])
            .await?;
        let mix_id = wire::text(&mix, "id").context("tidal named no mix for the track")?;
        Ok(self.mix(mix_id).await?.tracks)
    }

    /// The home feed as the web player draws it: rows of cards, each a list of one kind or a
    /// mix of kinds.
    async fn home_feed(&self) -> Result<HomeFeed> {
        let answer = self
            .get_v2(
                "/home/feed/static",
                &[
                    ("deviceType", "BROWSER"),
                    ("locale", "en_US"),
                    ("platform", "WEB"),
                ],
            )
            .await?;
        let user_id = self.user_id().await;
        let sections = wire::items(&answer)
            .iter()
            .filter_map(|module| {
                let items: Vec<GenreItem> = wire::items(module)
                    .iter()
                    .filter_map(|card| {
                        home_card(wire::text(card, "type")?, card.get("data")?, &user_id)
                    })
                    .collect();
                let title = wire::text(module, "title")?.to_owned();
                (!items.is_empty()).then_some(GenreSection { title, items })
            })
            .collect();
        Ok(HomeFeed {
            sections,
            ..HomeFeed::default()
        })
    }

    /// Resolves a track to a stream at the best quality the account may play, and reads what
    /// Tidal says about its loudness on the way.
    pub async fn playback(&self, track_id: &str) -> Result<Playback> {
        let track_path = format!("/tracks/{}", escape::component(track_id));
        let info_path = format!("{track_path}/playbackinfopostpaywall");
        let (answer, track) = tokio::join!(
            self.get(
                &info_path,
                &[
                    ("audioquality", QUALITY),
                    ("playbackmode", "STREAM"),
                    ("assetpresentation", "FULL"),
                ],
            ),
            self.get(&track_path, &[]),
        );
        let answer = answer.with_context(|| format!("cannot play the tidal track {track_id}"))?;
        if wire::text(&answer, "assetPresentation") == Some("PREVIEW") {
            bail!("the tidal plan only plays a preview of {track_id}");
        }
        let mime = wire::text(&answer, "manifestMimeType").unwrap_or_default();
        let manifest = Manifest::read(
            mime,
            wire::text(&answer, "manifest").context("tidal sent no manifest")?,
        )?;
        let granted = wire::text(&answer, "audioQuality").unwrap_or("an unknown quality");
        let quality = match (
            wire::number(&answer, "bitDepth"),
            wire::number(&answer, "sampleRate"),
        ) {
            (Some(depth), Some(rate)) => format!("{granted} at {depth}-bit {rate} Hz"),
            _ => granted.to_owned(),
        };
        let loudness = wire::decimal(&answer, "trackReplayGain").map(|gain| {
            Loudness::replay_gain(
                gain as f32,
                wire::decimal(&answer, "trackPeakAmplitude").map(|peak| peak as f32),
            )
        });
        let duration = track
            .ok()
            .and_then(|track| wire::number(&track, "duration"))
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs);
        Ok(Playback {
            manifest,
            quality,
            duration,
            loudness,
        })
    }
}

#[async_trait]
impl MusicApi for TidalClient {
    /// False once Tidal has refused the session, so the app sends the listener back to sign in
    /// rather than reporting every track as unplayable.
    fn alive(&self) -> bool {
        !self.inner.revoked.load(Ordering::Relaxed)
    }

    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        if let Some(mix) = id.strip_prefix(wire::MIX) {
            return Some(format!(
                "https://tidal.com/browse/mix/{}",
                escape::component(mix)
            ));
        }
        let kind = match kind {
            MediaKind::Track => "track",
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
        };
        Some(format!(
            "https://tidal.com/browse/{kind}/{}",
            escape::component(id)
        ))
    }

    /// The name and picture on the listener's Tidal profile, or their `@handle` when they gave
    /// no name. Tidal's `username` is the account's email, so it is never what the app shows.
    async fn profile(&self) -> Result<UserProfile> {
        let user_id = self.user_id().await;
        let user = escape::component(&user_id);
        let public = self
            .get_v2(&format!("/profiles/{user}"), &[])
            .await
            .inspect_err(|error| {
                log::debug!("tidal: no public profile, reading the account instead: {error:#}");
            })
            .unwrap_or_default();
        let named = wire::text(&public, "name")
            .map(str::to_owned)
            .or_else(|| wire::text(&public, "handle").map(|handle| format!("@{handle}")));
        let display_name = match named {
            Some(name) => name,
            None => {
                let account = self
                    .get(&format!("/users/{user}"), &[])
                    .await
                    .context("cannot load the tidal profile")?;
                wire::text(&account, "profileName")
                    .unwrap_or("Tidal")
                    .to_owned()
            }
        };
        Ok(UserProfile {
            id: user_id,
            display_name,
            avatar: wire::image(wire::text(&public, "picture"), wire::AVATAR),
        })
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let detail_path = format!("/artists/{}", escape::component(artist_id));
        let top_path = format!("{detail_path}/toptracks");
        let (detail, top, albums, biography) = tokio::join!(
            self.get(&detail_path, &[]),
            self.get(&top_path, &[("limit", "20")]),
            self.releases(artist_id, None, 50),
            self.biography(artist_id),
        );
        let detail = detail.context("cannot load the artist")?;
        Ok(Artist {
            name: wire::text(&detail, "name").unwrap_or_default().to_owned(),
            cover_large: wire::image(wire::text(&detail, "picture"), wire::PORTRAIT_LARGE),
            biography,
            monthly_listeners: None,
            top_tracks: top.map(|page| wire::track_list(&page)).unwrap_or_default(),
            albums: albums.unwrap_or_default(),
        })
    }

    /// The releases the overview leaves out: EPs and singles beside the albums, and what Tidal
    /// lists the artist as appearing on.
    async fn artist_catalogue(&self, artist_id: &str, _known: &[Track]) -> Result<ArtistCatalogue> {
        let (albums, singles, appears_on) = tokio::join!(
            self.discography(artist_id, None),
            self.discography(artist_id, Some("EPSANDSINGLES")),
            self.discography(artist_id, Some("COMPILATIONS")),
        );
        let mut albums = albums.context("cannot load the artist's albums")?;
        albums.extend(singles.unwrap_or_default());
        Ok(ArtistCatalogue {
            albums,
            top_tracks: Vec::new(),
            appears_on: appears_on.unwrap_or_default(),
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let detail_path = format!("/artists/{}", escape::component(artist_id));
        let (detail, biography) =
            tokio::join!(self.get(&detail_path, &[]), self.biography(artist_id));
        let detail = detail.context("cannot load the artist")?;
        Ok(ArtistProfile {
            name: wire::text(&detail, "name").unwrap_or_default().to_owned(),
            cover_large: wire::image(wire::text(&detail, "picture"), wire::PORTRAIT_LARGE),
            biography,
        })
    }

    /// Answers from the portrait cache first and looks the rest up a few at a time. An artist
    /// Tidal will not answer for is left out.
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
        let found: Vec<(String, String)> = stream::iter(missing)
            .map(|id| async move {
                let detail = self
                    .get(&format!("/artists/{}", escape::component(&id)), &[])
                    .await
                    .ok()?;
                let cover = wire::image(wire::text(&detail, "picture"), wire::PORTRAIT)?;
                Some((id, cover))
            })
            .buffer_unordered(LOOKUPS)
            .filter_map(|found| async move { found })
            .collect()
            .await;
        let mut known = lock(&self.inner.portraits);
        for (id, cover) in found {
            known.insert(id.clone(), cover.clone());
            images.insert(id, cover);
        }
        Ok(images)
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let path = self.favorites_path("tracks").await;
        self.all(&path, LATEST, FAVORITES_PAGE, wire::track_list)
            .await
            .context("cannot load the favorite tracks")
    }

    async fn saved_tracks_paged(&self) -> Result<Pages<Track>> {
        let path = self.favorites_path("tracks").await;
        Ok(self.paged(path, LATEST.to_vec(), FAVORITES_PAGE, wire::track_list))
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.favorite("tracks", "trackIds", track_id, saved).await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let answer = self
            .get(&format!("/tracks/{}", escape::component(track_id)), &[])
            .await?;
        wire::track(&answer).with_context(|| format!("cannot read the tidal track {track_id}"))
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    /// The listener's own playlists and the ones they follow, in one listing.
    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let user_id = self.user_id().await;
        let path = format!(
            "/users/{}/playlistsAndFavoritePlaylists",
            escape::component(&user_id)
        );
        let (playlists, mixes) = tokio::join!(
            self.all(&path, LATEST, PLAYLIST_PAGE, |page| {
                wire::items(page)
                    .iter()
                    .filter_map(|row| wire::playlist(row.get("playlist")?, &user_id))
                    .collect()
            }),
            self.saved_mixes(),
        );
        let mut playlists = playlists.context("cannot load the playlists")?;
        match mixes {
            Ok(mixes) => playlists.extend(mixes),
            Err(error) => log::warn!("tidal: cannot load the saved mixes: {error:#}"),
        }
        Ok(playlists)
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let user_id = self.user_id().await;
        let created: Value = self
            .call(
                Method::POST,
                &format!("{V1}/users/{}/playlists", escape::component(&user_id)),
                &[],
                Payload::Form(&[("title", name), ("description", "")]),
                None,
            )
            .await
            .context("cannot create the playlist")?
            .json()
            .await
            .context("cannot read the created playlist")?;
        wire::text(&created, "uuid")
            .map(str::to_owned)
            .context("the created playlist came back with no id")
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        let path = format!("/playlists/{}", escape::component(playlist_id));
        self.edit_playlist(|_| async {
            let etag = self.etag(playlist_id).await?;
            self.send(Method::POST, &path, Some(&[("title", name)]), Some(&etag))
                .await
        })
        .await
        .context("cannot rename the playlist")
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let path = format!("/playlists/{}", escape::component(playlist_id));
        self.edit_playlist(|_| async {
            let etag = self.etag(playlist_id).await?;
            self.send(Method::DELETE, &path, None, Some(&etag)).await
        })
        .await
        .context("cannot delete the playlist")
    }

    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        match playlist_id.strip_prefix(wire::MIX) {
            Some(mix_id) => self.save_mix(mix_id, false).await,
            None => {
                self.favorite("playlists", "uuids", playlist_id, false)
                    .await
            }
        }
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        match playlist_id.strip_prefix(wire::MIX) {
            Some(mix_id) => self.save_mix(mix_id, true).await,
            None => self.favorite("playlists", "uuids", playlist_id, true).await,
        }
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        let change = match public {
            true => "set-public",
            false => "set-private",
        };
        self.call(
            Method::PUT,
            &format!("{V2}/playlists/{}/{change}", escape::component(playlist_id)),
            &[],
            Payload::Empty,
            None,
        )
        .await
        .context("cannot change the playlist's visibility")?;
        Ok(())
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let path = format!("/playlists/{}/items", escape::component(playlist_id));
        let form = [
            ("trackIds", track_id),
            ("onDupes", "ADD"),
            ("onArtifactNotFound", "SKIP"),
        ];
        self.edit_playlist(|_| async {
            let etag = self.etag(playlist_id).await?;
            self.send(Method::POST, &path, Some(&form), Some(&etag))
                .await
        })
        .await
        .context("cannot add the track to the playlist")
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.edit_playlist(|last| self.remove_copies(playlist_id, track_id, last))
            .await
            .context("cannot remove the track from the playlist")
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let path = self.favorites_path("albums").await;
        self.all(&path, LATEST, FAVORITES_PAGE, wire::album_list)
            .await
            .context("cannot load the favorite albums")
    }

    async fn saved_albums_paged(&self) -> Result<Pages<Album>> {
        let path = self.favorites_path("albums").await;
        Ok(self.paged(path, LATEST.to_vec(), FAVORITES_PAGE, wire::album_list))
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.favorite("albums", "albumIds", album_id, saved).await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let path = self.favorites_path("artists").await;
        self.all(&path, LATEST, FAVORITES_PAGE, wire::artist_list)
            .await
            .context("cannot load the favorite artists")
    }

    async fn saved_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        let path = self.favorites_path("artists").await;
        Ok(self.paged(path, LATEST.to_vec(), FAVORITES_PAGE, wire::artist_list))
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.favorite("artists", "artistIds", artist_id, saved)
            .await
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let detail_path = format!("/albums/{}", escape::component(album_id));
        let tracks_path = format!("{detail_path}/tracks");
        let (detail, tracks) = tokio::join!(
            self.get(&detail_path, &[]),
            self.all(&tracks_path, &[], PAGE, wire::track_list),
        );
        let detail = detail.with_context(|| format!("cannot load the album {album_id}"))?;
        let album =
            wire::album(&detail).with_context(|| format!("cannot read the album {album_id}"))?;
        Ok(AlbumDetail {
            album,
            tracks: tracks.with_context(|| format!("cannot load the tracks of {album_id}"))?,
        })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    /// More from the album's artist first, topped up with a release or two from each of the
    /// artists Tidal lists as similar.
    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        let Some(artist_id) = artist_id else {
            return Ok(AlbumCatalogue::default());
        };
        let (more_by, similar) = tokio::join!(
            self.releases(artist_id, None, SUGGESTIONS + 1),
            self.similar_artists(artist_id),
        );
        // Nothing read at all is an error rather than an empty rail, so the catalog does not
        // keep the empty answer for the rest of the session.
        let (more_by, similar) = match (more_by, similar) {
            (Err(error), Err(_)) => return Err(error.context("cannot read any recommendations")),
            (more_by, similar) => (more_by.unwrap_or_default(), similar.unwrap_or_default()),
        };
        let mut seen = HashSet::from([album_id.to_owned()]);
        let mut liked: Vec<Album> = more_by
            .into_iter()
            .filter(|album| seen.insert(album.id.clone()))
            .take(SUGGESTIONS)
            .collect();
        if liked.len() < SUGGESTIONS {
            let lenders: Vec<String> = similar
                .iter()
                .take(SIMILAR_ARTISTS)
                .map(|artist| artist.id.clone())
                .collect();
            let lent = stream::iter(lenders)
                .map(|id| async move { self.releases(&id, None, SIMILAR_RELEASES).await })
                .buffered(LOOKUPS)
                .collect::<Vec<_>>()
                .await;
            for album in lent.into_iter().flatten().flatten() {
                if liked.len() >= SUGGESTIONS {
                    break;
                }
                if seen.insert(album.id.clone()) {
                    liked.push(album);
                }
            }
        }
        Ok(AlbumCatalogue {
            also_like: liked,
            similar,
        })
    }

    /// A playlist, or a mix wearing a playlist's id.
    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        if let Some(mix_id) = playlist_id.strip_prefix(wire::MIX) {
            return self.mix(mix_id).await;
        }
        let detail_path = format!("/playlists/{}", escape::component(playlist_id));
        let items_path = format!("{detail_path}/items");
        let (detail, tracks) = tokio::join!(
            self.get(&detail_path, &[]),
            self.all(&items_path, &[], PAGE, wire::track_list),
        );
        let detail = detail.with_context(|| format!("cannot load the playlist {playlist_id}"))?;
        let tracks = tracks.with_context(|| format!("cannot load the playlist {playlist_id}"))?;
        let user_id = self.user_id().await;
        let mut playlist = wire::playlist(&detail, &user_id)
            .with_context(|| format!("cannot read the playlist {playlist_id}"))?;
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

    /// Read from the first page of tracks alone, which holds enough distinct covers for a
    /// mosaic, so a long playlist is not downloaded whole to draw four squares.
    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = match playlist_id.strip_prefix(wire::MIX) {
            Some(mix_id) => self.mix(mix_id).await?.tracks,
            None => {
                let limit = PAGE.to_string();
                let page = self
                    .get(
                        &format!("/playlists/{}/items", escape::component(playlist_id)),
                        &[("limit", limit.as_str())],
                    )
                    .await?;
                wire::track_list(&page)
            }
        };
        Ok(distinct_covers(&tracks, wanted))
    }

    /// The track's station, or the mix Tidal keeps for it when the station is not served.
    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let radio = self
            .get(
                &format!("/tracks/{}/radio", escape::component(track_id)),
                &[("limit", RADIO)],
            )
            .await
            .map(|page| wire::track_list(&page));
        let tracks = match radio {
            Ok(tracks) if !tracks.is_empty() => tracks,
            Ok(_) | Err(_) => self.track_mix(track_id).await.unwrap_or_else(|error| {
                log::warn!("tidal: no track radio for {track_id}: {error:#}");
                Vec::new()
            }),
        };
        Ok((tracks, None))
    }

    /// Leaves out what cannot play, which in a search is mostly Atmos copies listed beside the
    /// stereo track they duplicate.
    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let found = self.search_kind(query, "TRACKS", "tracks").await?;
        Ok(wire::track_list(&found)
            .into_iter()
            .filter(|track| track.playable)
            .collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let found = self.search_kind(query, "ALBUMS", "albums").await?;
        Ok(wire::album_list(&found))
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let found = self.search_kind(query, "PLAYLISTS", "playlists").await?;
        let user_id = self.user_id().await;
        Ok(wire::items(&found)
            .iter()
            .filter_map(|row| wire::playlist(row, &user_id))
            .collect())
    }

    async fn home(&self) -> Result<HomeFeed> {
        self.home_feed()
            .await
            .context("cannot load the tidal home feed")
    }
}

/// One card of the home feed, by the kind the feed says it is. Videos are left out.
fn home_card(kind: &str, data: &Value, user_id: &str) -> Option<GenreItem> {
    match kind {
        "ALBUM" => wire::album(data).map(GenreItem::Album),
        "PLAYLIST" => wire::playlist(data, user_id).map(GenreItem::Playlist),
        "MIX" => wire::mix(data).map(GenreItem::Playlist),
        "TRACK" => wire::track(data).map(GenreItem::Track),
        "ARTIST" => wire::artist(data).map(GenreItem::Artist),
        _ => None,
    }
}

/// The path of an api url, for an error message that should not carry the query.
fn path_of(url: &str) -> &str {
    let path = url
        .strip_prefix(V1)
        .or_else(|| url.strip_prefix(V2))
        .unwrap_or(url);
    path.split('?').next().unwrap_or(path)
}

/// Where an artist's releases are listed.
fn releases_path(artist_id: &str) -> String {
    format!("/artists/{}/albums", escape::component(artist_id))
}

/// Whether the page at `page` of `size` rows was the last of a listing. Tidal leaves rows it
/// cannot serve in this country out of a page, so a short page proves nothing; the total it
/// reports does. A listing that reports none ends at the first empty page.
fn exhausted(answer: &Value, page: usize, size: usize) -> bool {
    match wire::count::<usize>(answer, "totalNumberOfItems") {
        Some(total) => (page + 1) * size >= total,
        None => wire::items(answer).is_empty(),
    }
}

/// The http client every Tidal request goes through, the stream downloads included. A read
/// that waits longer than `STALL` fails the request, so a dead connection surfaces as an error
/// rather than a track that never starts.
pub(crate) fn http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT)
        .read_timeout(STALL)
        .build()
        .context("cannot build the tidal http client")
}
