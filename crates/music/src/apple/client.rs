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
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow, bail};
use async_trait::async_trait;
use futures::future::try_join_all;
use futures::stream::{self, StreamExt as _, TryStreamExt as _};
use serde::de::IgnoredAny;
use serde_json::Value;

use crate::apple::auth::{self, AGENT};
use crate::apple::recommend;
use crate::apple::wire::{self, Resource};
use crate::engine::Loudness;
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, Genre, GenreDetail,
    GenreItem, GenreSection, HomeFeed, MediaKind, MusicApi, Page, Pages, PinOutcome, PinTarget,
    PinTargetKind, Playlist, PlaylistDetail, SavedArtist, Track, UserProfile, escape,
};

/// The API the web player calls.
const API: &str = "https://amp-api.music.apple.com/v1";

/// Where the web player posts its play-activity beacon. It is a different host from the amp-api
/// gateway, and the one that makes a play count toward the account's recently played and play
/// history across devices.
const ACTIVITY: &str = "https://universal-activity-service.itunes.apple.com/play";

/// The client build string the web player stamps on every play beacon. The activity service
/// drops a beacon that does not look like an Apple Music client.
const BEACON_BUILD: &str = "AppleMusic/1.0 Linux/0.0 model/Linux x86_64 build/2638.11.0-external";

/// The user-agent the web player reports in the beacon body, alongside the build string.
const BEACON_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:156.0) Gecko/20100101 Firefox/156.0";

/// Container type 3 is an album: the surface a catalog play is reported from.
const CONTAINER_ALBUM: u8 = 3;

/// How many rows a library or track listing asks for at a time, which is Apple's maximum.
const PAGE: usize = 100;

/// How many search hits to ask for. Apple refuses a search page larger than this outright.
const HITS: usize = 25;

/// How many pages one listing will walk before it stops. A library of a hundred thousand songs
/// is not something to pull into memory in one go.
const PAGES: usize = 40;

/// How many pages of one listing are in flight at once, after the first.
///
/// A page of the songs library takes Apple over a second to answer whatever else is going on,
/// so a listing's wait is set by how many pages are asked for in turn, not by how many rows
/// there are. The first page goes alone, so a small library still costs one request; it comes
/// back with `meta.total`, and everything behind it goes out together, this many at a time.
/// The web player fires a whole library at once, so this is not more than Apple expects.
const FAN: usize = 12;

/// The library listings, and the includes each is read with. Named once so the pages and the
/// favorites drawn over them ask for the same listing and share one fetch.
///
/// A library song carries no date of its own, on any route or with any `extend`, while the
/// library album it sits under does, so the songs ask for that album cut down to the one
/// field. The include is scoped to the library rows on purpose: an untyped `include=catalog`
/// would expand the catalog of every included album too, and a page grows fivefold.
const SONGS: &str = "/me/library/songs";
const ALBUMS: &str = "/me/library/albums";
const ARTISTS: &str = "/me/library/artists";
const SONGS_QUERY: &[(&str, &str)] = &[
    ("include[library-songs]", "catalog,albums"),
    ("include[songs]", "artists,albums"),
    ("fields[library-albums]", "dateAdded"),
];
const CATALOG_QUERY: &[(&str, &str)] = &[("include", "catalog")];
const ALBUMS_QUERY: &[(&str, &str)] = &[("include", "catalog"), ALBUM_ARTISTS];

/// Asks for the artists behind every album in an answer, nested ones included. Apple otherwise
/// names them in one string with no id, and a card cannot link to an artist without one. Only
/// the typed form reaches albums inside a view or under a library row.
pub(crate) const ALBUM_ARTISTS: (&str, &str) = ("include[albums]", "artists");

/// The listener's pins with everything a sidebar row shows, as the web player asks for them.
/// The resources come back as one map rather than inline, and a library artist's artwork only
/// exists on the catalog artist behind it.
const PINS: &str = "/me/library/pins";
const PINS_QUERY: &[(&str, &str)] = &[
    ("format[resources]", "map"),
    ("include[library-albums]", "catalog"),
    ("include[library-artists]", "catalog"),
    ("fields[artists]", "artwork"),
];

/// How many pins Apple keeps, songs and videos included. It refuses the next one with a 400.
const PIN_LIMIT: usize = 6;

/// How long a fetched listing is kept for the next caller. Long enough for one library load,
/// whose pages and favorites read the same listings within seconds of each other.
const LISTING_TTL: Duration = Duration::from_secs(30);

/// How many artist ids one batch of portraits asks about.
const PORTRAITS: usize = 50;

/// How many library ids one ratings lookup may ask about. Apple sets the ceiling per resource
/// type and refuses a longer list outright: a hundred albums, but only twenty-five artists.
const RATED_ALBUMS: usize = 100;
const RATED_ARTISTS: usize = 25;
const RATED_SONGS: usize = 100;

/// What the listener is called on their own playlists, until Apple offers a name for them.
const OWNER: &str = "You";

/// How many station tracks one request may ask for. Apple refuses more than ten.
const STATION: usize = 10;

/// How many of those requests make a queue worth having behind a track.
const STATION_PULLS: usize = 3;

/// How many times one read is sent before its failure belongs to the caller.
///
/// Apple's gateway sheds load while a library is being pulled: it answers 503, or takes the
/// request and then cuts the body short of the length it announced. Neither says anything about
/// the account or the request, and asking again a moment later lands. Only reads are repeated,
/// since a write that broke on the way back may still have been applied.
const TRIES: u32 = 3;

/// How long to wait before sending a read again. Doubled after every attempt, and spread over
/// that much again on top, so the pages of a listing that failed together do not all come back
/// at the same instant.
const RETRY_WAIT: Duration = Duration::from_millis(400);

/// The longest a `Retry-After` is honoured for. Past this the wait costs more than the failure,
/// and a shelf that fails keeps showing its last snapshot anyway.
const RETRY_CAP: Duration = Duration::from_secs(5);

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
    /// Listings fetched or being fetched, by path and query, so the library pages and the
    /// favorites drawn over them walk each listing once between them.
    listings: Arc<Mutex<HashMap<String, Listing>>>,
    /// What the last pin list said, so an unpin needs no lookup and a pin past the limit is
    /// not sent.
    pinned: Arc<Mutex<Pinned>>,
}

/// What the catalog says about a song that playback wants before the decoder can tell.
#[derive(Clone, Copy, Debug, Default)]
pub struct Details {
    pub duration: Option<Duration>,
    pub loudness: Option<Loudness>,
}

/// Where a listing's pages go as they land, and how each row is read on the way: the channel
/// a paged caller listens on and the reader its rows are turned into items with.
type Sink<'a, T> = (
    &'a tokio::sync::mpsc::Sender<Result<Page<T>>>,
    &'a (dyn Fn(&Resource<'_>) -> Option<T> + Sync),
);

/// The last pin list as the client remembers it: the library id behind each pin Sonora shows,
/// by uri, and how many pins Apple holds in all.
#[derive(Default)]
struct Pinned {
    ids: HashMap<String, String>,
    count: usize,
}

/// One listing fetched, or still being fetched, and when it was first asked for.
struct Listing {
    at: Instant,
    bodies: Arc<tokio::sync::OnceCell<Arc<Bodies>>>,
}

/// The pages of a listing as the text Apple sent, and how many rows they hold between them.
///
/// A page body is one large block the allocator hands straight back to the system when it is
/// dropped. The same rows parsed would be tens of thousands of small blocks, which leave holes
/// in the heap the allocator keeps long after the listing is gone. A row is parsed only while a
/// reader looks at it.
#[derive(Default)]
struct Bodies {
    pages: Vec<Box<str>>,
    rows: usize,
}

impl Bodies {
    /// Every row of every page read through `read`, in order.
    fn read<T, F>(&self, read: &F) -> Vec<T>
    where
        F: Fn(&Resource<'_>) -> Option<T> + ?Sized,
    {
        self.pages
            .iter()
            .flat_map(|body| wire::outline(body).read(read))
            .collect()
    }
}

/// A failure the gateway is answering for rather than the request: a connection that broke
/// before the body was whole, or a status Apple gives while it is shedding load. It carries the
/// wait Apple asked for, when it named one. Any other failure is the account's or the request's
/// own, and sending it again would only collect the same refusal twice.
#[derive(Debug)]
struct Busy(Option<Duration>);

impl fmt::Display for Busy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the apple music gateway is busy")
    }
}

impl std::error::Error for Busy {}

/// A 404 from Apple. It also means an empty relationship, such as the tracks of a playlist
/// with nothing in it, so a caller that knows the parent exists reads it as no rows.
#[derive(Debug)]
pub(crate) struct Missing;

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("apple music has nothing there")
    }
}

impl std::error::Error for Missing {}

impl AppleClient {
    /// Reads the web player's bearer token and the account's storefront, which is also what
    /// proves the user token is still good.
    pub async fn connect(user_token: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(AGENT)
            .build()
            .context("cannot build the apple http client")?;
        let bearer = auth::bearer(&http).await?;
        let mut client = Self {
            http,
            user_token: user_token.into(),
            bearer: bearer.into(),
            storefront: "us".into(),
            station: Arc::default(),
            listings: Arc::default(),
            pinned: Arc::default(),
        };
        // A kept bearer token Apple has stopped taking is read off the page again, once.
        let body = match client.get("/me/storefront", &[]).await {
            Err(error) if format!("{error:#}").contains("answered 401") => {
                log::info!("apple: the bearer token was turned down, reading a new one");
                auth::forget_bearer();
                client.bearer = auth::bearer(&client.http).await?.into();
                client.get("/me/storefront", &[]).await?
            }
            answered => answered?,
        };
        let answered: wire::Answer = wire::answer(&body);
        let storefront = answered
            .data
            .first()
            .and_then(|row| row.id.as_str())
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

    pub(crate) fn catalog(&self, path: &str) -> String {
        format!("/catalog/{}{path}", escape::component(&self.storefront))
    }

    /// One request against the API, sent again with a growing wait while the gateway is what
    /// failed rather than the request. Answers with the body as Apple sent it.
    ///
    /// A read gets [`TRIES`] attempts. A write gets one: its answer may have been lost on the
    /// way back after Apple had already applied it, and adding a playlist twice is worse than
    /// reporting a failure once.
    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<String> {
        let tries = match method == reqwest::Method::GET {
            true => TRIES,
            false => 1,
        };
        let mut tried = 0;
        loop {
            let error = match self.send_once(&method, path, query, body.as_ref()).await {
                Ok(answered) => return Ok(answered),
                Err(error) => error,
            };
            let Some(Busy(after)) = error.downcast_ref::<Busy>() else {
                return Err(error);
            };
            tried += 1;
            if tried >= tries {
                return Err(error);
            }
            let wait = after.unwrap_or_else(|| backoff(tried - 1));
            log::debug!(
                "apple: {method} {path} failed, asking again in {}ms: {error:#}",
                wait.as_millis()
            );
            tokio::time::sleep(wait).await;
        }
    }

    /// One round trip, with nothing repeated, answering with the body text of a success. The
    /// account token rides on every request, and neither token is ever logged.
    async fn send_once(
        &self,
        method: &reqwest::Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<String> {
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
            Some(body) => request.json(body),
            // Apple's gateway refuses a write with no length at all.
            None => request.header(reqwest::header::CONTENT_LENGTH, "0"),
        };
        let started = Instant::now();
        // Nothing was answered, so nothing was applied either: this one is always worth asking
        // again, whatever the method.
        let response = request.send().await.map_err(|error| {
            anyhow::Error::new(error)
                .context(format!("cannot reach apple music at {path}"))
                .context(Busy(None))
        })?;

        let status = response.status();
        let after = retry_after(response.headers());
        // Every wait on a library load is a sum of these, so this is where a slow one shows.
        log::debug!(
            "apple: {method} {path} answered {status} in {} ms",
            started.elapsed().as_millis()
        );
        // A body cut short of the length it announced is the gateway giving up mid answer, not
        // an answer with anything wrong in it.
        let text = response.bytes().await.map(utf8).map_err(|error| {
            anyhow::Error::new(error)
                .context(format!("cannot read the apple music answer for {path}"))
                .context(Busy(after))
        })?;
        if method != reqwest::Method::GET {
            self.forget();
        }
        if !status.is_success() {
            let answered = parse(&text);
            // Apple says what it disliked in the body, which is the only way to tell a rejected
            // parameter from a rejected account.
            let detail = answered
                .pointer("/errors/0/detail")
                .or_else(|| answered.pointer("/errors/0/title"))
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            let refused = anyhow!("apple music answered {status} for {method} {path}: {detail}");
            return match (busy(status), status == reqwest::StatusCode::NOT_FOUND) {
                (true, _) => Err(refused.context(Busy(after))),
                (false, true) => Err(refused.context(Missing)),
                (false, false) => Err(refused),
            };
        }
        Ok(text)
    }

    /// A read, answered with the body as Apple sent it, for the caller to read with
    /// [`wire::answer`].
    pub(crate) async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<String> {
        self.send(reqwest::Method::GET, path, query, None).await
    }

    async fn post(
        &self,
        path: &str,
        query: &[(&str, &str)],
        body: Option<Value>,
    ) -> Result<String> {
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

    /// Posts a play-activity beacon to Apple's activity service, once. This is a different host
    /// from the amp-api gateway, so it does not go through [`send`](Self::send). It carries the
    /// same bearer and account token every other request does, and neither is ever logged.
    async fn play_activity(&self, body: &Value) -> Result<()> {
        let response = self
            .http
            .post(ACTIVITY)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Bearer {}", self.bearer),
            )
            .header("Music-User-Token", self.user_token.as_ref())
            .header(reqwest::header::ORIGIN, "https://music.apple.com")
            .header(reqwest::header::REFERER, "https://music.apple.com/")
            .json(body)
            .send()
            .await
            .context("cannot reach apple's play activity service")?;
        let status = response.status();
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            bail!("apple play activity answered {status}: {detail}");
        }
        Ok(())
    }

    /// The account's live recently played shelf, newest first: the albums and playlists the
    /// recent plays came from, read the way the web player's own shelf reads them.
    async fn recent_items(&self) -> Result<Vec<GenreItem>> {
        let body = self
            .get("/me/recent/played", &[("limit", "10"), ALBUM_ARTISTS])
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        Ok(answered
            .data
            .iter()
            .filter_map(|row| match row.kind.as_str() {
                Some("albums") => wire::album(row).map(GenreItem::Album),
                Some("library-albums") => wire::library_album(row).map(GenreItem::Album),
                Some("playlists") | Some("library-playlists") => {
                    wire::playlist(row).map(GenreItem::Playlist)
                }
                _ => None,
            })
            .collect())
    }

    /// A bounded shelf in Apple's addition order. Albums without a playable catalog copy
    /// are omitted, as they are in the rest of the library.
    async fn recently_added(&self) -> Result<Vec<GenreItem>> {
        let body = self
            .get(
                "/me/library/recently-added",
                &[("limit", "10"), ("include", "catalog"), ALBUM_ARTISTS],
            )
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        Ok(answered
            .data
            .iter()
            .filter_map(|row| match row.kind.as_str() {
                Some("library-albums") => wire::library_album(row).map(GenreItem::Album),
                Some("library-playlists") => {
                    wire::library_playlist(row, OWNER).map(GenreItem::Playlist)
                }
                _ => None,
            })
            .collect())
    }

    /// Walks a paged listing and reads every row with `read`. The rows come from
    /// [`listing`](Self::listing), so two walks of one listing close together cost one fetch.
    async fn walk<T, F>(
        &self,
        path: &str,
        page: usize,
        query: &[(&str, &str)],
        read: F,
    ) -> Result<Vec<T>>
    where
        F: Fn(&Resource<'_>) -> Option<T>,
    {
        let bodies = self.listing(path, page, query).await?;
        Ok(bodies.read(&read))
    }

    /// Every page of a paged listing, fetched once and kept for [`LISTING_TTL`].
    ///
    /// The library pages and the favorites drawn over them read the same listings within
    /// seconds of each other, and a page costs about a second, so a second walk is the
    /// difference between a library and a wait. Callers asking at the same time share the one
    /// fetch in flight; a fetch that fails is forgotten, so the next caller tries again.
    async fn listing(
        &self,
        path: &str,
        page: usize,
        query: &[(&str, &str)],
    ) -> Result<Arc<Bodies>> {
        let cell = self.cell(path, page, query);
        cell.get_or_try_init(|| async {
            self.pages::<()>(path, page, query, None)
                .await
                .map(Arc::new)
        })
        .await
        .cloned()
    }

    /// The memo cell for one listing, made if it is not there or has gone stale.
    fn cell(
        &self,
        path: &str,
        page: usize,
        query: &[(&str, &str)],
    ) -> Arc<tokio::sync::OnceCell<Arc<Bodies>>> {
        let key = format!("{path}|{page}|{query:?}");
        let mut listings = self
            .listings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        listings.retain(|_, listing| listing.at.elapsed() < LISTING_TTL);
        listings
            .entry(key)
            .or_insert_with(|| {
                self.expire();
                Listing {
                    at: Instant::now(),
                    bodies: Arc::default(),
                }
            })
            .bodies
            .clone()
    }

    /// Drops the listings that have outlived [`LISTING_TTL`] once it has passed, so the pages
    /// of a startup load leave memory without waiting for the next listing to be asked for.
    fn expire(&self) {
        let listings = Arc::downgrade(&self.listings);
        tokio::spawn(async move {
            tokio::time::sleep(LISTING_TTL).await;
            let Some(listings) = listings.upgrade() else {
                return;
            };
            if let Ok(mut listings) = listings.lock() {
                listings.retain(|_, listing| listing.at.elapsed() < LISTING_TTL);
            }
        });
    }

    /// A listing handed out a page at a time, read through `read`, on a channel that stays
    /// open until the last page. The first page carries the total when Apple says one.
    ///
    /// The pages still go through the memo, so a caller right behind this one, the favorites
    /// pass behind a library page, reads what this fetched. A listing the memo already holds
    /// arrives as one page.
    fn paged<T: Send + 'static>(
        &self,
        path: &'static str,
        page: usize,
        query: &'static [(&'static str, &'static str)],
        read: fn(&Resource<'_>) -> Option<T>,
    ) -> Pages<T> {
        let (sink, pages) = tokio::sync::mpsc::channel(FAN);
        let client = self.clone();
        tokio::spawn(async move {
            let cell = client.cell(path, page, query);
            let streamed = std::sync::atomic::AtomicBool::new(false);
            let fetched = cell
                .get_or_try_init(|| async {
                    streamed.store(true, std::sync::atomic::Ordering::Relaxed);
                    client
                        .pages(path, page, query, Some((&sink, &read)))
                        .await
                        .map(Arc::new)
                })
                .await;
            match fetched {
                Ok(bodies) if !streamed.load(std::sync::atomic::Ordering::Relaxed) => {
                    let items = bodies.read(&read);
                    sink.send(Ok(Page {
                        total: Some(bodies.rows),
                        items,
                    }))
                    .await
                    .ok();
                }
                Ok(_) => {}
                Err(error) => {
                    sink.send(Err(error)).await.ok();
                }
            }
        });
        pages
    }

    /// Drops every kept listing. Every write goes through here, since any of them can change
    /// what a listing would say.
    fn forget(&self) {
        if let Ok(mut listings) = self.listings.lock() {
            listings.clear();
        }
    }

    /// Fetches every page of a listing and hands back their bodies in order. Stops at
    /// [`PAGES`] pages so one enormous library cannot run away with the process.
    ///
    /// Only the first page is asked for on its own. A library listing answers it with
    /// `meta.total`, which says exactly how many pages are left, and they all go out together,
    /// [`FAN`] in flight at a time. A listing that carries no total is asked for up to the cap
    /// the same way and stops at the first page that comes back short or without a `next`
    /// link, so at most a fan's worth of pages past the end is ever asked for.
    ///
    /// With a `sink`, every page is also read through the given reader and sent on as it
    /// lands, which is how a library page shows its first rows before its last have arrived.
    async fn pages<T>(
        &self,
        path: &str,
        page: usize,
        query: &[(&str, &str)],
        sink: Option<Sink<'_, T>>,
    ) -> Result<Bodies> {
        let limit = page.to_string();
        let mut asked: Vec<(&str, &str)> = query.to_vec();
        asked.push(("limit", &limit));
        let started = Instant::now();

        let first = self.body(path, &asked).await?;
        let (total, more, got, items) = {
            let outline = wire::outline(&first);
            let total = outline
                .meta
                .total
                .as_u64()
                .and_then(|total| usize::try_from(total).ok());
            let items = sink.map(|(_, read)| outline.read(read));
            (total, outline.next.is_present(), outline.data.len(), items)
        };
        if let (Some((sink, _)), Some(items)) = (sink, items) {
            sink.send(Ok(Page { total, items })).await.ok();
        }
        let mut bodies = Bodies {
            pages: vec![first],
            rows: got,
        };
        if got == page && more {
            let left = total.map_or(PAGES - 1, |total| {
                total.saturating_sub(got).div_ceil(page).min(PAGES - 1)
            });
            let mut answers = stream::iter(1..=left)
                .map(|step| {
                    let at = (step * page).to_string();
                    let asked = &asked;
                    async move {
                        let mut asked: Vec<(&str, &str)> = asked.clone();
                        asked.push(("offset", at.as_str()));
                        self.body(path, &asked).await
                    }
                })
                .buffered(FAN);
            while let Some(body) = answers.try_next().await? {
                let (rows, more, items) = {
                    let outline = wire::outline(&body);
                    let items = sink.map(|(_, read)| outline.read(read));
                    (outline.data.len(), outline.next.is_present(), items)
                };
                let last = rows < page || !more;
                if let (Some((sink, _)), Some(items)) = (sink, items) {
                    sink.send(Ok(Page { total, items })).await.ok();
                }
                bodies.rows += rows;
                bodies.pages.push(body);
                if last {
                    break;
                }
            }
        }
        log::debug!(
            "apple: walked {path}: {} rows over {} page(s) in {} ms",
            bodies.rows,
            bodies.pages.len(),
            started.elapsed().as_millis()
        );
        Ok(bodies)
    }

    /// One page of a listing as the text Apple sent, left unparsed.
    async fn body(&self, path: &str, query: &[(&str, &str)]) -> Result<Box<str>> {
        self.send(reqwest::Method::GET, path, query, None)
            .await
            .map(String::into_boxed_str)
    }

    /// Keeps the library resources of one kind the listener has favorited, out of `items`
    /// paired with their library ids.
    ///
    /// Apple keeps a favorite as a personal rating of 1 on the library resource and never
    /// inlines it into a listing, so it is a second request over the listing's ids, asked
    /// `batch` at a time: `/me/library?ids[library-albums]=…&fields[library-albums]=personalRating`,
    /// which is how the web player draws its stars. Only the rated resources come back.
    async fn rated<T>(&self, kind: &str, batch: usize, items: Vec<(String, T)>) -> Result<Vec<T>> {
        let key = format!("ids[library-{kind}]");
        let field = format!("fields[library-{kind}]");
        let batches: Vec<String> = items
            .chunks(batch)
            .map(|batch| {
                batch
                    .iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect();
        let answers = try_join_all(batches.iter().map(|ids| {
            let query = [
                (key.as_str(), ids.as_str()),
                (field.as_str(), "personalRating"),
            ];
            async move { self.get("/me/library", &query).await }
        }))
        .await?;
        let mut loved = HashSet::new();
        for body in &answers {
            let answered: wire::Answer = wire::answer(body);
            loved.extend(
                answered
                    .data
                    .iter()
                    .filter(|row| {
                        row.attributes
                            .as_ref()
                            .and_then(|attributes| attributes.personal_rating.as_i64())
                            == Some(1)
                    })
                    .filter_map(library_id),
            );
        }
        Ok(items
            .into_iter()
            .filter(|(id, _)| loved.contains(id))
            .map(|(_, item)| item)
            .collect())
    }

    /// Favorites or unfavorites one catalog resource, the way the web player's star does: the
    /// id is posted to `/me/favorites` and deleted from there, and neither answer has a body.
    /// Apple itself adds a favorited song to the library when the account's Add Favorite Songs
    /// to Library setting is on, which it is by default. Removing a favorite never touches it.
    async fn favorite(&self, kind: &str, id: &str, saved: bool) -> Result<()> {
        let key = format!("ids[{kind}]");
        let query = [(key.as_str(), id)];
        match saved {
            true => self.post("/me/favorites", &query, None).await.map(|_| ()),
            false => self.delete("/me/favorites", &query).await,
        }
    }

    /// The library id of a catalog resource, if the listener has it. Apple only answers this one
    /// way round, which is why removing something takes two requests.
    async fn mine(&self, kind: &str, id: &str) -> Result<Option<String>> {
        let path = self.catalog(&format!("/{kind}/{}/library", escape::component(id)));
        match self.get(&path, &[]).await {
            Ok(body) => Ok(wire::answer::<IgnoredAny>(&body)
                .data
                .first()
                .and_then(|row| row.id.as_str())
                .map(str::to_owned)),
            // Not in the library at all, which Apple reports as a missing relationship.
            Err(error) if error.is::<Missing>() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The pin list last read. The lock is never held across an await.
    fn remembered(&self) -> std::sync::MutexGuard<'_, Pinned> {
        self.pinned
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The library id of a catalog artist, if the listener has one. A catalog artist has no
    /// `library` relationship, so the library is searched for the artist's name and the hit
    /// whose catalog artist is this one is kept.
    async fn library_artist(&self, id: &str) -> Result<Option<String>> {
        let body = self
            .get(
                &self.catalog(&format!("/artists/{}", escape::component(id))),
                &[("fields[artists]", "name")],
            )
            .await?;
        let artist: wire::Answer = wire::answer(&body);
        let name = artist
            .data
            .first()
            .and_then(|row| row.attributes.as_ref())
            .and_then(|attributes| attributes.name.as_str())
            .with_context(|| format!("apple music has no artist {id}"))?;
        let limit = HITS.to_string();
        let found = self
            .get(
                "/me/library/search",
                &[
                    ("term", name),
                    ("types", "library-artists"),
                    ("include[library-artists]", "catalog"),
                    ("limit", &limit),
                ],
            )
            .await?;
        Ok(wire::library_artist(&wire::answer(&found), id))
    }

    /// The catalog's length and loudness for a song, from one lookup that asks for nothing
    /// else. Either is missing when the catalog does not say, and a failed lookup leaves both
    /// missing.
    ///
    /// The loudness is the catalog's own audio analysis: integrated LUFS and a true peak in
    /// dBFS. The same figures sit in the `ludt` box of Apple's enhanced HLS encodes, but the
    /// Widevine encode this path plays carries no `udta` at all.
    pub async fn playback_details(&self, id: &str) -> Details {
        let answered = self
            .get(
                &self.catalog(&format!("/songs/{}", escape::component(id))),
                &[
                    ("include[songs]", "audio-analysis"),
                    ("fields[songs]", "durationInMillis"),
                    ("omit[resource]", "autos"),
                ],
            )
            .await;
        let body = match answered {
            Ok(body) => body,
            Err(error) => {
                log::debug!("apple: cannot read the details of {id}: {error:#}");
                return Details::default();
            }
        };
        let answered: wire::Answer = wire::answer(&body);
        let song = answered.data.first();
        let duration = song
            .and_then(|song| song.attributes.as_ref())
            .and_then(|attributes| attributes.duration_in_millis.as_u64())
            .filter(|millis| *millis > 0)
            .map(Duration::from_millis);
        let main = song
            .and_then(|song| song.relationships.audio_analysis.rows().first())
            .and_then(|analysis| analysis.attributes.as_ref())
            .map(|attributes| &attributes.loudness.main);
        let lufs = main
            .and_then(|main| main.value.as_f64())
            .filter(|lufs| (-70.0..0.0).contains(lufs));
        let peak = main
            .and_then(|main| main.peak.as_f64())
            .map(|dbfs| 10f64.powf(dbfs / 20.0) as f32);
        let loudness = lufs.map(|lufs| Loudness {
            lufs: lufs as f32,
            peak,
        });
        Details { duration, loudness }
    }

    /// The tracks of a playlist, from the first page or from a continuation, with the
    /// continuation behind them and how many tracks the whole playlist has, when Apple says.
    async fn playlist_page(&self, path: &str) -> Result<(Vec<Track>, Option<String>, Option<u32>)> {
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
            .await;
        let body = match answered {
            Err(error) if error.is::<Missing>() => return Ok((Vec::new(), None, Some(0))),
            answered => answered?,
        };
        let outline = wire::outline(&body);
        let tracks = outline.read(&wire::playlist_track);
        let next = outline
            .next
            .as_str()
            .map(|next| next.trim_start_matches("/v1").to_owned());
        let total = outline
            .meta
            .total
            .as_u64()
            .and_then(|total| u32::try_from(total).ok());
        Ok((tracks, next, total))
    }

    /// The catalog id of an artist, looked up through the library when `artist_id` is a
    /// library id. Fails for a library artist the catalog has no page for.
    pub(crate) async fn catalog_artist(&self, artist_id: &str) -> Result<String> {
        if !Self::is_mine(artist_id) {
            return Ok(artist_id.to_owned());
        }
        let body = self
            .get(
                &format!("/me/library/artists/{}", escape::component(artist_id)),
                &[("include", "catalog")],
            )
            .await?;
        wire::answer::<IgnoredAny>(&body)
            .data
            .first()
            .and_then(wire::catalog)
            .and_then(|found| found.id.as_str())
            .map(str::to_owned)
            .context("this library artist has no page in the catalog")
    }

    /// Whether an id belongs to the listener's own library rather than the catalog.
    pub(crate) fn is_mine(id: &str) -> bool {
        let mut letters = id.chars();
        matches!(letters.next(), Some('i' | 'l' | 'r' | 'p')) && letters.next() == Some('.')
    }

    /// Builds the station a track seeds and reads its first tracks.
    ///
    /// This is what the web player's autoplay calls, and the only way Apple hands out the
    /// contents of a station: `/songs/{id}/station` names one but lists nothing. The artists
    /// and albums are asked for along with the songs, since a station row names them in text
    /// only and a suggestion without ids leads nowhere.
    async fn seed_station(&self, track_id: &str) -> Result<(String, Vec<Track>)> {
        let limit = STATION.to_string();
        let body = self
            .post(
                "/me/stations/continuous",
                &[
                    ("with", "tracks"),
                    ("limit[results:tracks]", &limit),
                    ("include[songs]", "artists,albums"),
                ],
                Some(serde_json::json!({
                    "data": [{ "id": track_id, "type": "songs" }]
                })),
            )
            .await?;
        let answered: wire::Answer<wire::Seeded> = wire::answer(&body);
        let station = answered
            .results
            .station
            .id
            .as_str()
            .context("apple built no station for that track")?
            .to_owned();
        let tracks = answered
            .results
            .tracks
            .iter()
            .filter_map(wire::song)
            .collect();
        Ok((station, tracks))
    }

    /// The next stretch of a station. A station never ends, so this can be asked again as long
    /// as the queue needs filling.
    async fn station_tracks(&self, station: &str) -> Result<Vec<Track>> {
        let limit = STATION.to_string();
        let body = self
            .post(
                &format!("/me/stations/next-tracks/{}", escape::component(station)),
                &[("limit", &limit), ("include[songs]", "artists,albums")],
                None,
            )
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        Ok(answered.data.iter().filter_map(wire::song).collect())
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
            ALBUM_ARTISTS,
        ];
        if let Some(genre) = genre {
            query.push(("genre", genre));
        }
        let body = self.get(&self.catalog("/charts"), &query).await?;
        let answered: wire::Answer<wire::Charts> = wire::answer(&body);
        let charts = &answered.results;
        let mut sections = Vec::new();
        for (kind, title, charts) in [
            ("playlists", "Top playlists", &charts.playlists),
            ("albums", "Top albums", &charts.albums),
            ("songs", "Top songs", &charts.songs),
        ] {
            let items: Vec<GenreItem> = charts
                .iter()
                .flat_map(wire::Relation::rows)
                .filter_map(|row| match kind {
                    "playlists" => wire::playlist(row).map(GenreItem::Playlist),
                    "songs" => wire::song(row).map(GenreItem::Track),
                    _ => wire::album(row).map(GenreItem::Album),
                })
                .collect();
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
                "https://music.apple.com/{}/{part}/{}",
                escape::component(&self.storefront),
                escape::component(id)
            )),
        }
    }

    /// The account, named and pictured from the listener's Apple Music profile when they have
    /// one; otherwise the service name with no picture. A missing profile never fails sign-in.
    async fn profile(&self) -> Result<UserProfile> {
        let body = match self.get("/me/social-profile", &[]).await {
            Ok(body) => body,
            // No social profile on the account: the service name and no picture still sign in.
            Err(error) => {
                log::debug!("apple: no social profile for this account: {error:#}");
                String::new()
            }
        };
        let social: wire::Answer = wire::answer(&body);
        let attributes = social.data.first().and_then(|row| row.attributes.as_ref());
        let display_name = attributes
            .and_then(|attributes| wire::text(&attributes.name))
            .unwrap_or_else(|| "Apple Music".to_owned());
        let avatar = attributes.and_then(|attributes| wire::artwork(attributes, wire::ART));
        Ok(UserProfile {
            id: self.storefront.to_string(),
            display_name,
            avatar,
        })
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let limit = HITS.to_string();
        let body = self
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
        let answered: wire::Answer<wire::Search> = wire::answer(&body);
        Ok(answered
            .results
            .songs
            .rows()
            .iter()
            .filter_map(wire::song)
            .collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let limit = HITS.to_string();
        let body = self
            .get(
                &self.catalog("/search"),
                &[
                    ("term", query),
                    ("types", "albums"),
                    ("limit", &limit),
                    ALBUM_ARTISTS,
                ],
            )
            .await?;
        let answered: wire::Answer<wire::Search> = wire::answer(&body);
        Ok(answered
            .results
            .albums
            .rows()
            .iter()
            .filter_map(wire::album)
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let limit = HITS.to_string();
        let body = self
            .get(
                &self.catalog("/search"),
                &[("term", query), ("types", "playlists"), ("limit", &limit)],
            )
            .await?;
        let answered: wire::Answer<wire::Search> = wire::answer(&body);
        Ok(answered
            .results
            .playlists
            .rows()
            .iter()
            .filter_map(wire::playlist)
            .collect())
    }

    /// A catalog song, with its artists and album so both are somewhere to go.
    async fn track(&self, track_id: &str) -> Result<Track> {
        let body = self
            .get(
                &self.catalog(&format!("/songs/{}", escape::component(track_id))),
                &[("include[songs]", "artists,albums")],
            )
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        let found = answered
            .data
            .first()
            .with_context(|| format!("apple music has no song {track_id}"))?;
        wire::song(found).with_context(|| format!("cannot read the apple song {track_id}"))
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    /// Reports a play to Apple's play-activity service, so the track reaches the account's
    /// recently played on every device. It mirrors the web player's `JSPLAY` PLAY_START event;
    /// only a catalog id counts, and a failure is logged and dropped, never breaking playback.
    ///
    /// The field shape is read off music.apple.com's MusicKit rather than documented: the
    /// service silently drops a beacon missing the tokens and client identity it expects.
    async fn report_play(&self, track_id: &str) -> Result<()> {
        if Self::is_mine(track_id) {
            return Ok(());
        }
        // Only a numeric catalog id names anything the catalog history keeps.
        if track_id.parse::<u64>().is_err() {
            return Ok(());
        }
        // Duration and album container come from one catalog lookup; a failed lookup still sends.
        let song = self
            .get(
                &self.catalog(&format!("/songs/{}", escape::component(track_id))),
                &[
                    ("include[songs]", "albums"),
                    ("fields[songs]", "durationInMillis"),
                    ("fields[albums]", "url"),
                ],
            )
            .await
            .unwrap_or_default();
        let answered: wire::Answer = wire::answer(&song);
        let data = answered.data.first();
        let duration = data
            .and_then(|song| song.attributes.as_ref())
            .and_then(|attributes| attributes.duration_in_millis.as_u64())
            .unwrap_or(0);
        let album = data
            .and_then(|song| song.relationships.albums.rows().first())
            .and_then(|album| album.id.as_str());
        // MusicKit mints a persistent id per play; a random 64-bit hex stands in the same way.
        let persistent = format!("{:016x}", fastrand::u64(..));
        let mut item = serde_json::json!({
            "build-version": BEACON_BUILD,
            "developer-token": self.bearer.as_ref(),
            // PLAY_START is event type 1 in the web player's enum; the beacon that lands the play.
            "event-type": 1,
            "event-reason-hint-type": 1,
            "type": 1,
            // A subscription stream reports its catalog id under `subscription-adam-id`.
            "ids": { "subscription-adam-id": track_id },
            "internal-build": false,
            "media-type": 0,
            "media-duration-in-milliseconds": duration,
            "milliseconds-since-play": 1,
            "offline": false,
            "persistent-id": persistent,
            "play-mode": {
                "auto-play-mode": 1,
                "repeat-play-mode": 1,
                "shuffle-play-mode": 1,
            },
            "private-enabled": false,
            "sb-enabled": true,
            "siri-initiated": false,
            "source-type": 16,
            "start-position-in-milliseconds": 0,
            "store-front": self.storefront.as_ref(),
            "user-agent": BEACON_AGENT,
            "user-token": self.user_token.as_ref(),
            "utc-offset-in-seconds": 0,
        });
        // The album container is set only when the lookup gave one.
        if let Some(album) = album {
            item["container-type"] = serde_json::json!(CONTAINER_ALBUM);
            item["container-ids"] = serde_json::json!({ "album-adam-id": album });
        }
        let body = serde_json::json!({
            "client_id": "JSCLIENT",
            "event_type": "JSPLAY",
            "data": [item],
        });
        self.play_activity(&body).await
    }

    /// The account's recently played songs across every device, newest first. Only songs are
    /// kept; stations and music videos the list can also carry are dropped.
    async fn recently_played(&self) -> Result<Vec<Track>> {
        let body = self
            .get(
                "/me/recent/played/tracks",
                &[
                    ("limit", "30"),
                    ("include[songs]", "artists,albums"),
                    ("types", "songs"),
                ],
            )
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        Ok(answered.data.iter().filter_map(wire::song).collect())
    }

    /// The songs the listener has added. Only the ones with a catalog id are listed: an upload
    /// has no catalog encode behind it, and this path plays catalog tracks.
    async fn all_tracks(&self) -> Result<Vec<Track>> {
        self.walk(SONGS, PAGE, SONGS_QUERY, wire::library_song)
            .await
    }

    async fn all_tracks_paged(&self) -> Result<Pages<Track>> {
        Ok(self.paged(SONGS, PAGE, SONGS_QUERY, wire::library_song))
    }

    async fn all_albums(&self) -> Result<Vec<Album>> {
        self.walk(ALBUMS, PAGE, ALBUMS_QUERY, wire::library_album)
            .await
    }

    async fn all_albums_paged(&self) -> Result<Pages<Album>> {
        Ok(self.paged(ALBUMS, PAGE, ALBUMS_QUERY, wire::library_album))
    }

    /// Every artist with music in the library. Apple derives this list itself from the songs
    /// and albums added, so there is no adding to it directly.
    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        self.walk(ARTISTS, PAGE, CATALOG_QUERY, wire::saved_artist)
            .await
    }

    async fn all_artists_paged(&self) -> Result<Pages<SavedArtist>> {
        Ok(self.paged(ARTISTS, PAGE, CATALOG_QUERY, wire::saved_artist))
    }

    /// The favorite songs: the library songs the listener has rated, read the same way as the
    /// albums and artists. The rating is the source of truth, not the Favorite Songs playlist,
    /// which only exists while the account adds favorites to its library.
    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let songs = self
            .walk(SONGS, PAGE, SONGS_QUERY, |row| {
                Some((library_id(row)?, wire::library_song(row)?))
            })
            .await?;
        self.rated("songs", RATED_SONGS, songs).await
    }

    /// The favorite albums: the library albums the listener has rated, since a favorite is a
    /// personal rating and a rating only exists on a library resource. An album favorited from
    /// the catalog without being added is therefore not here.
    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let albums = self
            .walk(ALBUMS, PAGE, ALBUMS_QUERY, |row| {
                Some((library_id(row)?, wire::library_album(row)?))
            })
            .await?;
        self.rated("albums", RATED_ALBUMS, albums).await
    }

    /// The favorite artists, read the same way as the albums.
    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let artists = self
            .walk(ARTISTS, PAGE, CATALOG_QUERY, |row| {
                Some((library_id(row)?, wire::saved_artist(row)?))
            })
            .await?;
        self.rated("artists", RATED_ARTISTS, artists).await
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        self.walk("/me/library/playlists", PAGE, &[], |row| {
            wire::library_playlist(row, OWNER)
        })
        .await
    }

    /// The listener's pins in pin order, all of them pinned. The rest of the library is left
    /// out, and `pin_uri` names anything else that can be pinned.
    async fn pin_targets(&self) -> Result<Option<Vec<PinTarget>>> {
        let limit = PAGE.to_string();
        let mut query = PINS_QUERY.to_vec();
        query.push(("limit", &limit));
        let body = self.get(PINS, &query).await?;
        let answered: wire::Answer = wire::answer(&body);
        let pins = wire::pins(&answered, OWNER);
        *self.remembered() = Pinned {
            ids: pins
                .iter()
                .map(|(id, target)| (target.uri.clone(), id.clone()))
                .collect(),
            count: answered.data.len(),
        };
        Ok(Some(pins.into_iter().map(|(_, target)| target).collect()))
    }

    /// Pins or unpins through Apple's own pins, which only hold what is in the library. A pin
    /// already listed reuses its library id, and a catalog id is resolved to one. Anything not
    /// in the library comes back as `Outside`, and a pin past Apple's limit as `LimitReached`.
    async fn set_pinned(&self, uri: &str, pinned: bool) -> Result<PinOutcome> {
        let (_, rest) = uri
            .split_once(':')
            .context("cannot pin an item without a provider")?;
        let (kind, id) = rest
            .split_once(':')
            .context("cannot pin an item without a kind")?;
        let kind = match kind {
            "playlist" => "playlists",
            "album" => "albums",
            "artist" => "artists",
            _ => bail!("that kind of item cannot be pinned"),
        };
        if pinned && self.remembered().count >= PIN_LIMIT {
            return Ok(PinOutcome::LimitReached);
        }
        let listed = self.remembered().ids.get(uri).cloned();
        let item = match (listed, Self::is_mine(id), kind) {
            (Some(item), _, _) => Some(item),
            (None, true, _) => Some(id.to_owned()),
            (None, false, "artists") => self.library_artist(id).await?,
            (None, false, _) => self.mine(kind, id).await?,
        };
        let Some(item) = item else {
            return Ok(PinOutcome::Outside);
        };
        let path = format!("{PINS}/{item}");
        if !pinned {
            return self.delete(&path, &[]).await.map(|_| PinOutcome::Updated);
        }
        match self.post(&path, &[], None).await {
            Ok(_) => Ok(PinOutcome::Updated),
            // A pin made elsewhere since the last look can fill the list, so a refusal is
            // measured against a fresh one.
            Err(error) => match self.pin_targets().await {
                Ok(_) if self.remembered().count >= PIN_LIMIT => Ok(PinOutcome::LimitReached),
                _ => Err(error),
            },
        }
    }

    fn pin_uri(&self, kind: PinTargetKind, id: &str) -> Option<String> {
        wire::pin_uri(kind, id)
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.favorite("songs", track_id, saved).await
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.favorite("albums", album_id, saved).await
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.favorite("artists", artist_id, saved).await
    }

    /// Adding works on a catalog id. Removing needs the library id, which only the catalog
    /// resource can point at, so it is one request to find and one to remove.
    async fn set_in_library(&self, kind: MediaKind, id: &str, present: bool) -> Result<()> {
        let kind = match kind {
            MediaKind::Track => "songs",
            MediaKind::Album => "albums",
            MediaKind::Artist => {
                bail!("apple music derives the library artists from the music added")
            }
            MediaKind::Playlist => bail!("a playlist joins the library through its own call"),
        };
        match present {
            true => {
                let key = format!("ids[{kind}]");
                self.post("/me/library", &[(key.as_str(), id)], None)
                    .await
                    .map(|_| ())
            }
            false => match self.mine(kind, id).await? {
                Some(mine) => {
                    self.delete(
                        &format!("/me/library/{kind}/{}", escape::component(&mine)),
                        &[],
                    )
                    .await
                }
                None => Ok(()),
            },
        }
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let (path, query): (String, Vec<(&str, &str)>) = match Self::is_mine(album_id) {
            true => (
                format!("/me/library/albums/{}", escape::component(album_id)),
                vec![
                    ("include", "tracks,catalog"),
                    ALBUM_ARTISTS,
                    ("include[songs]", "artists"),
                ],
            ),
            false => (
                self.catalog(&format!("/albums/{}", escape::component(album_id))),
                vec![
                    ("include", "tracks,artists"),
                    ("include[songs]", "artists,albums"),
                ],
            ),
        };
        let body = self.get(&path, &query).await?;
        let answered: wire::Answer = wire::answer(&body);
        let found = answered
            .data
            .first()
            .context("apple music has no such album")?;
        // A library album's own rows are library songs; a catalog album's are catalog songs.
        let tracks = found
            .relationships
            .tracks
            .rows()
            .iter()
            .filter_map(|row| match row.kind.as_str() {
                Some("library-songs") => wire::library_song(row),
                _ => wire::song(row),
            })
            .collect();
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

    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        recommend::album_catalogue(self, album_id, artist_id).await
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let id = self.catalog_artist(artist_id).await?;
        let body = self
            .get(
                &self.catalog(&format!("/artists/{}", escape::component(&id))),
                &[
                    (
                        "views",
                        "top-songs,full-albums,live-albums,compilation-albums,singles",
                    ),
                    ("include[songs]", "artists,albums"),
                    ALBUM_ARTISTS,
                    ("extend", "artistBio,editorialArtwork"),
                ],
            )
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        answered
            .data
            .first()
            .and_then(wire::artist)
            .with_context(|| format!("cannot read the apple artist {id}"))
    }

    async fn artist_catalogue(&self, artist_id: &str, _known: &[Track]) -> Result<ArtistCatalogue> {
        recommend::artist_catalogue(self, artist_id).await
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let body = self
            .get(
                &self.catalog(&format!("/artists/{}", escape::component(artist_id))),
                &[("extend", "artistBio")],
            )
            .await?;
        let answered: wire::Answer = wire::answer(&body);
        answered
            .data
            .first()
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
            let body = self
                .get(&self.catalog("/artists"), &[("ids", &joined)])
                .await?;
            let answered: wire::Answer = wire::answer(&body);
            for row in &answered.data {
                let Some(id) = row.id.as_str() else {
                    continue;
                };
                if let Some(art) = row
                    .attributes
                    .as_ref()
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
            true => format!("/me/library/playlists/{}", escape::component(playlist_id)),
            false => self.catalog(&format!("/playlists/{}", escape::component(playlist_id))),
        };
        let playlist = {
            let body = self.get(&path, &[]).await?;
            let answered: wire::Answer = wire::answer(&body);
            let found = answered
                .data
                .first()
                .context("apple music has no such playlist")?;
            match mine {
                true => wire::library_playlist(found, OWNER),
                false => wire::playlist(found),
            }
            .context("cannot read the apple playlist")?
        };
        let (tracks, continuation, total) = self.playlist_page(&format!("{path}/tracks")).await?;
        // The total is what the header shows while the rest of a long playlist is still on its
        // way, so the count does not climb a page at a time.
        Ok(PlaylistDetail {
            playlist: Playlist {
                track_count: total.unwrap_or(tracks.len() as u32),
                ..playlist
            },
            tracks,
            continuation,
        })
    }

    /// Every track of a playlist, paged the same way as a library listing rather than one
    /// `next` link at a time: a long playlist is hundreds of rows, and each page is a wait.
    /// The rows skip the listing memo, since startup reads every playlist at once and only the
    /// tracks read from them are used.
    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let path = match Self::is_mine(playlist_id) {
            true => format!(
                "/me/library/playlists/{}/tracks",
                escape::component(playlist_id)
            ),
            false => self.catalog(&format!(
                "/playlists/{}/tracks",
                escape::component(playlist_id)
            )),
        };
        let walked = self
            .pages::<Track>(
                &path,
                PAGE,
                &[
                    ("include[songs]", "artists,albums"),
                    ("include[library-songs]", "catalog"),
                ],
                None,
            )
            .await
            .map(|bodies| bodies.read(&wire::playlist_track));
        match walked {
            Err(error) if error.is::<Missing>() => Ok(Vec::new()),
            walked => walked,
        }
    }

    /// One more page of a playlist. The continuation is the link Apple handed back.
    async fn playlist_continuation(
        &self,
        continuation: &str,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let (tracks, next, _) = self.playlist_page(continuation).await?;
        Ok((tracks, next))
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let path = match Self::is_mine(playlist_id) {
            true => format!(
                "/me/library/playlists/{}/tracks",
                escape::component(playlist_id)
            ),
            false => self.catalog(&format!(
                "/playlists/{}/tracks",
                escape::component(playlist_id)
            )),
        };
        let (tracks, _, _) = self.playlist_page(&path).await?;
        Ok(crate::distinct_covers(&tracks, wanted))
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let body = self
            .post(
                "/me/library/playlists",
                &[],
                Some(serde_json::json!({ "attributes": { "name": name } })),
            )
            .await?;
        wire::answer::<IgnoredAny>(&body)
            .data
            .first()
            .and_then(|row| row.id.as_str())
            .map(str::to_owned)
            .context("apple music named no new playlist")
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.patch(
            &format!("/me/library/playlists/{}", escape::component(playlist_id)),
            serde_json::json!({ "attributes": { "name": name } }),
        )
        .await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.delete(
            &format!("/me/library/playlists/{}", escape::component(playlist_id)),
            &[],
        )
        .await
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        self.patch(
            &format!("/me/library/playlists/{}", escape::component(playlist_id)),
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
                self.delete(
                    &format!("/me/library/playlists/{}", escape::component(&id)),
                    &[],
                )
                .await
            }
            None => Ok(()),
        }
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.post(
            &format!(
                "/me/library/playlists/{}/tracks",
                escape::component(playlist_id)
            ),
            &[],
            Some(serde_json::json!({
                "data": [{ "id": track_id, "type": "songs" }]
            })),
        )
        .await
        .map(|_| ())
    }

    /// Removing takes the playlist's own row id rather than the catalog id, so the playlist is
    /// read first to find which row carries this track. A row matches on its reporting id too,
    /// which is the id a song Apple delivered again is listed under.
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let limit = PAGE.to_string();
        let mut row = None;
        for page in 0..PAGES {
            let offset = (page * PAGE).to_string();
            let body = self
                .get(
                    &format!(
                        "/me/library/playlists/{}/tracks",
                        escape::component(playlist_id)
                    ),
                    &[("limit", &limit), ("offset", &offset)],
                )
                .await?;
            let outline = wire::outline(&body);
            row = outline
                .rows()
                .find(|row| {
                    row.attributes
                        .as_ref()
                        .and_then(|attributes| attributes.play_params.as_ref())
                        .is_some_and(|params| {
                            params.catalog_id.as_str() == Some(track_id)
                                || params.reporting_id.as_str() == Some(track_id)
                        })
                })
                .and_then(|row| row.id.as_str().map(str::to_owned));
            if row.is_some() || outline.data.is_empty() || !outline.next.is_present() {
                break;
            }
        }
        let Some(row) = row else {
            bail!("that track is not in the playlist any more");
        };
        self.delete(
            &format!(
                "/me/library/playlists/{}/tracks",
                escape::component(playlist_id)
            ),
            &[("ids[library-songs]", &row), ("mode", "all")],
        )
        .await
    }

    /// What Apple made for this listener: their recommendation groups, and the storefront
    /// charts underneath so the page is never empty.
    async fn home(&self) -> Result<HomeFeed> {
        let (mut sections, recents) = {
            let body = self
                .get("/me/recommendations", &[("limit", "12"), ALBUM_ARTISTS])
                .await?;
            let answered: wire::Answer = wire::answer(&body);
            let mut sections = Vec::new();
            let mut recents = Vec::new();
            for group in &answered.data {
                let title = group
                    .attributes
                    .as_ref()
                    .and_then(|attributes| attributes.title.string_for_display.as_str())
                    .unwrap_or("For you")
                    .to_owned();
                let items: Vec<GenreItem> = group
                    .relationships
                    .contents
                    .rows()
                    .iter()
                    .filter_map(|row| match row.kind.as_str() {
                        Some("playlists") => wire::playlist(row).map(GenreItem::Playlist),
                        Some("albums") => wire::album(row).map(GenreItem::Album),
                        _ => None,
                    })
                    .collect();
                match is_recents(group) {
                    true => recents = items,
                    false if !items.is_empty() => sections.push(GenreSection { title, items }),
                    false => {}
                }
            }
            (sections, recents)
        };
        if sections.is_empty() {
            sections = self.charts(None).await.unwrap_or_default();
        }
        // Apple's own recently played group is a cached ranking that lags real plays, so the live
        // list fills the Recently played shelf and leads Quick picks, and the group only stands
        // in when that list cannot be read.
        let (containers, songs, added) = futures::join!(
            self.recent_items(),
            <Self as MusicApi>::recently_played(self),
            self.recently_added()
        );
        match added {
            Ok(items) if !items.is_empty() => sections.insert(
                past_mixes(&sections),
                GenreSection {
                    title: "home-recently-added".to_owned(),
                    items,
                },
            ),
            Ok(_) => {}
            Err(error) => log::warn!("apple: cannot load recently added: {error:#}"),
        }
        let containers = match containers {
            Ok(live) if !live.is_empty() => live,
            Ok(_) => recents,
            Err(error) => {
                log::warn!("apple: cannot load recently played: {error:#}");
                recents
            }
        };
        if !containers.is_empty() {
            sections.insert(
                0,
                GenreSection {
                    title: "home-recently-played".to_owned(),
                    items: containers.clone(),
                },
            );
        }
        let songs = songs.unwrap_or_else(|error| {
            log::warn!("apple: cannot load recently played songs: {error:#}");
            Vec::new()
        });
        let listen_again = recent_mix(containers, songs);
        Ok(HomeFeed {
            listen_again,
            sections,
            ..HomeFeed::default()
        })
    }

    /// The genres of the storefront, without the root that parents them all.
    async fn genres(&self) -> Result<Vec<Genre>> {
        let genres = self
            .walk(&self.catalog("/genres"), PAGE, &[], |row| {
                let attributes = row.attributes.as_ref()?;
                // Every real genre hangs off "Music", which is not one itself.
                if !attributes.parent_id.is_present() {
                    return None;
                }
                Some(Genre {
                    id: row.id.as_str()?.to_owned(),
                    name: wire::text(&attributes.name)?,
                    cover: None,
                })
            })
            .await?;
        Ok(genres)
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        let body = self
            .get(
                &self.catalog(&format!("/genres/{}", escape::component(genre_id))),
                &[],
            )
            .await?;
        let name = wire::answer::<IgnoredAny>(&body)
            .data
            .first()
            .and_then(|row| row.attributes.as_ref())
            .and_then(|attributes| attributes.name.as_str())
            .unwrap_or_default()
            .to_owned();
        Ok(GenreDetail {
            name,
            sections: self.charts(Some(genre_id)).await?,
        })
    }

    /// The cover of the genre's top album, which Apple's genre list leaves out.
    async fn genre_cover(&self, genre_id: &str) -> Result<Option<String>> {
        let query = [("types", "albums"), ("limit", "1"), ("genre", genre_id)];
        let body = self.get(&self.catalog("/charts"), &query).await?;
        let answered: wire::Answer<wire::Charts> = wire::answer(&body);
        Ok(answered
            .results
            .albums
            .iter()
            .flat_map(wire::Relation::rows)
            .find_map(|row| wire::album(row)?.cover))
    }

    /// The station a track seeds, as a queue's worth of tracks.
    ///
    /// Asked again for the same track, this carries on through the station it already built
    /// rather than starting another, so a queue that keeps being extended stays one station.
    /// Apple hands out ten tracks a time, and repeats are dropped because a station is free to
    /// come back to a song this queue already holds.
    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
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
        Ok((tracks, None))
    }
}

/// Whether a status is Apple holding the request off rather than refusing it. A 5xx from the
/// gateway and a 429 both clear on their own; every 4xx below that is about the request.
fn busy(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// The wait Apple asked for, when it named one in seconds and it is short enough to sit through.
/// The HTTP-date form of the header is not read: Apple sends seconds, and a date is only ever a
/// longer wait than [`RETRY_CAP`] would allow anyway.
fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let asked = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let seconds = asked.trim().parse::<u64>().ok()?;
    Some(Duration::from_secs(seconds).min(RETRY_CAP))
}

/// How long to wait after `tried` attempts: the base doubled once per attempt, and up to that
/// much again on top. The spread comes off the wall clock's nanoseconds, which is enough to keep
/// the pages of one listing from failing and returning in lockstep.
fn backoff(tried: u32) -> Duration {
    let base = RETRY_WAIT * 2u32.pow(tried);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    base + base.mul_f32(nanos as f32 / 1e9)
}

/// The id of a library row, which is the library's own rather than the catalog's.
fn library_id(row: &Resource) -> Option<String> {
    row.id.as_str().map(str::to_owned)
}

/// A body as text, decoded the way reqwest's own `text` decodes one. Valid UTF-8 takes over the
/// buffer the body was collected into rather than being copied out of it.
fn utf8(body: bytes::Bytes) -> String {
    match String::from_utf8(Vec::from(body)) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}

/// An answer body as a tree, or null when it is empty or not JSON.
fn parse(text: &str) -> Value {
    match text.trim().is_empty() {
        true => Value::Null,
        false => serde_json::from_str(text).unwrap_or(Value::Null),
    }
}

/// The recently played part of Quick picks: the albums and playlists recent plays came from in
/// Apple's order, then the recent songs no album there covers. An album only one of those songs
/// came from stands as that song, since a lone song from it is a song play rather than an
/// album play.
fn recent_mix(containers: Vec<GenreItem>, songs: Vec<Track>) -> Vec<GenreItem> {
    let played = |album: &str| {
        songs
            .iter()
            .filter(|song| song.album_id.as_deref() == Some(album))
            .count()
    };
    let mut mixed: Vec<GenreItem> = containers
        .into_iter()
        .map(|item| match &item {
            GenreItem::Album(album) if played(&album.id) == 1 => songs
                .iter()
                .find(|song| song.album_id.as_deref() == Some(album.id.as_str()))
                .map(|song| GenreItem::Track(song.clone()))
                .unwrap_or(item),
            _ => item,
        })
        .collect();
    let covered: HashSet<String> = mixed
        .iter()
        .filter_map(|item| match item {
            GenreItem::Album(album) => Some(album.id.clone()),
            GenreItem::Track(track) => track.album_id.clone(),
            _ => None,
        })
        .collect();
    let mut seen = HashSet::new();
    mixed.extend(
        songs
            .into_iter()
            .filter(|song| {
                song.album_id
                    .as_ref()
                    .is_none_or(|album| !covered.contains(album))
            })
            .filter(|song| song.id.clone().is_some_and(|id| seen.insert(id)))
            .map(GenreItem::Track),
    );
    mixed
}

/// The index right below the last shelf holding one of Apple's personal mixes, the colored
/// playlists made for the listener, or 0 when no shelf holds one. Personal mix ids start with
/// `pl.pm-`.
fn past_mixes(sections: &[GenreSection]) -> usize {
    sections
        .iter()
        .rposition(|section| {
            section.items.iter().any(|item| {
                matches!(item, GenreItem::Playlist(playlist) if playlist.id.starts_with("pl.pm-"))
            })
        })
        .map_or(0, |at| at + 1)
}

/// Whether a recommendation group is the account's recently played one, which the live
/// Recently played shelf replaces. Its title is in the storefront's language, so only its kind
/// is checked.
fn is_recents(group: &Resource) -> bool {
    group
        .attributes
        .as_ref()
        .and_then(|attributes| attributes.kind.as_str())
        == Some("recently-played")
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
