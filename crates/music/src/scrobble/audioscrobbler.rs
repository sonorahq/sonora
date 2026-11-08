use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use async_trait::async_trait;
use futures::StreamExt as _;
use md5::{Digest, Md5};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

use super::{Account, Link, Play, Secret, Service};
use crate::{
    Album, ListeningStats, Milestone, Overview, Ranked, SavedArtist, Track, compare,
    milestone_numbers,
};
use crate::{escape, tag_key};
use serde_json::Value;

/// The loopback address the browser hands the token back on.
const PORT: u16 = 8990;
const PATH: &str = "/scrobble";
/// How long the user has to approve in the browser before the link is given up on.
const WAIT: Duration = Duration::from_secs(300);
/// How many listens last.fm's `track.scrobble` takes in one call.
const LASTFM_BATCH: usize = 50;
/// How many plays one page of recent tracks holds, and how many pages are read for the hours
/// someone listens at.
const ACTIVITY_PAGE: &str = "200";
const ACTIVITY_PAGES: usize = 4;
/// How many of those slices are asked about at once.
const DENSITY_AT_ONCE: usize = 8;
/// How many slices the account's history is cut into to show when it was played.
const SLICES: usize = 24;
/// How many ranked artists, songs and albums are kept, and how many rows a weekly chart can hold.
const RANKED: usize = 50;
const CHART_ROWS: usize = 1000;

const PAGE: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
    <html><body>Sonora is connected. You can close this window.</body></html>";

/// Libre.fm issues no API accounts, so the key and secret its protocol still asks for are ours
/// and constant.
const ANONYMOUS: &str = "sonora";

/// The last.fm 2.0 API. Libre.fm runs GNU FM, which answers the same methods at its own urls, so
/// the two services are one type with different addresses.
pub struct AudioScrobbler {
    id: &'static str,
    endpoint: &'static str,
    authorize: &'static str,
    signup: Option<&'static str>,
    link: Link,
    /// How many listens one `track.scrobble` carries. One means the server only implements the
    /// single unindexed form, which is what GNU FM does.
    batch: usize,
}

pub const LASTFM: AudioScrobbler = AudioScrobbler {
    id: "lastfm",
    endpoint: "https://ws.audioscrobbler.com/2.0/",
    authorize: "https://www.last.fm/api/auth/",
    signup: Some("https://www.last.fm/api/account/create"),
    link: Link::Keys,
    batch: LASTFM_BATCH,
};

pub const LIBREFM: AudioScrobbler = AudioScrobbler {
    id: "librefm",
    endpoint: "https://libre.fm/2.0/",
    authorize: "https://libre.fm/api/auth/",
    signup: None,
    link: Link::Browser,
    batch: 1,
};

#[async_trait]
impl Service for AudioScrobbler {
    fn id(&self) -> &'static str {
        self.id
    }

    fn link(&self) -> Link {
        self.link
    }

    fn signup(&self) -> Option<&'static str> {
        self.signup
    }

    async fn connect(&self, secret: Secret) -> Result<Account> {
        let (key, shared) = match secret {
            Secret::Keys { key, secret } => (key.trim().to_owned(), secret.trim().to_owned()),
            Secret::None => (ANONYMOUS.to_owned(), ANONYMOUS.to_owned()),
            _ => bail!("{} needs an api key and a secret", self.id),
        };
        if key.is_empty() || shared.is_empty() {
            bail!("{} needs an api key and a secret", self.id);
        }

        let listener = TcpListener::bind(("127.0.0.1", PORT))
            .await
            .context("cannot listen for the scrobbling callback")?;
        open::that_in_background(authorize_url(self.authorize, &key));

        let token = token(listener).await?;
        let form = vec![
            ("method".to_owned(), "auth.getSession".to_owned()),
            ("api_key".to_owned(), key.clone()),
            ("token".to_owned(), token),
        ];
        let answer = post(self.endpoint, &shared, form).await?;
        let Some(granted) = answer.session else {
            bail!("{} returned no session", self.id);
        };

        Ok(Account {
            key,
            secret: shared,
            session: granted.key,
            name: granted.name,
            enabled: true,
            ..Account::default()
        })
    }

    async fn now_playing(&self, account: &Account, play: &Play) -> Result<()> {
        let mut form = vec![
            ("method".to_owned(), "track.updateNowPlaying".to_owned()),
            ("artist".to_owned(), play.artist.clone()),
            ("track".to_owned(), play.title.clone()),
            ("duration".to_owned(), play.duration.as_secs().to_string()),
        ];
        if let Some(album) = play.release() {
            form.push(("album".to_owned(), album.to_owned()));
        }
        self.call(account, form).await
    }

    async fn scrobble(&self, account: &Account, plays: &[Play]) -> Result<()> {
        for batch in plays.chunks(self.batch) {
            let mut form = vec![("method".to_owned(), "track.scrobble".to_owned())];
            for (index, play) in batch.iter().enumerate() {
                form.push((self.slot("artist", index), play.artist.clone()));
                form.push((self.slot("track", index), play.title.clone()));
                form.push((self.slot("timestamp", index), play.timestamp().to_string()));
                form.push((
                    self.slot("duration", index),
                    play.duration.as_secs().to_string(),
                ));
                if let Some(album) = play.release() {
                    form.push((self.slot("album", index), album.to_owned()));
                }
            }
            self.call(account, form).await?;
        }
        Ok(())
    }

    fn stats_dated(&self) -> bool {
        self.id == "lastfm"
    }

    async fn history(&self, account: &Account) -> Result<ListeningStats> {
        ensure!(self.stats_dated(), "{} does not expose history", self.id);
        let info = self
            .read("user.getInfo", &[("user", account.name.as_str())], account)
            .await?;
        let user = info.get("user").unwrap_or(&info);
        let (milestones, density) = tokio::join!(
            async {
                match number(user, "playcount") {
                    Some(total) => self.milestones(account, total).await,
                    None => Vec::new(),
                }
            },
            async {
                match registered(user) {
                    Some(since) => self.density(account, since).await,
                    None => Vec::new(),
                }
            },
        );
        Ok(ListeningStats {
            milestones,
            density,
            ..Default::default()
        })
    }

    async fn stats(&self, account: &Account, dates: Option<(i64, i64)>) -> Result<ListeningStats> {
        ensure!(self.stats_dated(), "{} does not expose stats", self.id);
        let user = account.name.as_str();
        let profile = [("user", user)];
        let tags = [("user", user), ("limit", "30")];
        // A range is also read for the equal stretch before it, to see how the listening moved.
        let earlier = dates.map(|(from, to)| (2 * from - to - 1, from - 1));
        let (charts, info, tags, activity, before) = tokio::join!(
            self.charts(account, dates),
            self.read("user.getInfo", &profile, account),
            self.read("user.getTopTags", &tags, account),
            self.activity(account, dates),
            async {
                match earlier {
                    Some(range) => Some(tokio::join!(
                        self.charts(account, Some(range)),
                        self.plays_between(account, range)
                    )),
                    None => None,
                }
            },
        );
        let mut charts = charts?;
        let mut overview = Overview {
            artists: charts.distinct[0],
            tracks: charts.distinct[1],
            albums: charts.distinct[2],
            ..Default::default()
        };
        if let Some((plays, times)) = activity {
            overview.plays = Some(plays);
            // Only the latest plays are read, so their days and streaks say nothing of the rest.
            let complete = times.len() >= plays;
            overview = overview.timed(&times);
            if !complete {
                overview = overview.partial();
            }
        }
        if let Some((Ok(before), plays_before @ Some(1..))) = before {
            let key = |name: &str, artist: &str| format!("{name}\u{1}{artist}");
            let names = |rows: &[Ranked<SavedArtist>]| -> Vec<String> {
                rows.iter().map(|row| row.item.name.clone()).collect()
            };
            compare(&mut charts.artists, &names(&before.artists), |artist| {
                artist.name.clone()
            });
            let songs: Vec<_> = before
                .tracks
                .iter()
                .map(|row| key(&row.item.name, &row.item.artists))
                .collect();
            compare(&mut charts.tracks, &songs, |track| {
                key(&track.name, &track.artists)
            });
            let records: Vec<_> = before
                .albums
                .iter()
                .map(|row| key(&row.item.name, &row.item.artists))
                .collect();
            compare(&mut charts.albums, &records, |album| {
                key(&album.name, &album.artists)
            });
            overview.change = overview
                .plays
                .zip(plays_before)
                .map(|(now, was)| ((now as i64 - was as i64) * 100 / was as i64) as i32);
        }
        let mut lifetime = match info {
            Ok(info) => {
                let user = info.get("user").unwrap_or(&info);
                Overview {
                    plays: number(user, "playcount"),
                    artists: number(user, "artist_count"),
                    tracks: number(user, "track_count"),
                    albums: number(user, "album_count"),
                    since: registered(user),
                    ..Default::default()
                }
            }
            Err(error) => {
                log::debug!("stats: cannot load Last.fm lifetime totals: {error:#}");
                Overview::default()
            }
        };
        lifetime.plays_per_day = lifetime
            .plays
            .zip(lifetime.since)
            .and_then(|(plays, since)| {
                let days = (jiff::Timestamp::now().as_second() - since) as f32 / 86_400.;
                (days >= 1.).then(|| plays as f32 / days)
            });
        let genres = tags
            .map(|tags| {
                rows(&tags, "toptags", "tag")
                    .into_iter()
                    .filter(|tag| tag_key(&string(tag, "name")).is_some())
                    .map(|tag| Ranked::new(string(tag, "name"), number(tag, "count")))
                    .collect()
            })
            .unwrap_or_default();
        if dates.is_none() {
            // The window is the whole account, so one set of figures says it all.
            overview = Overview {
                plays: lifetime.plays,
                tracks: lifetime.tracks,
                artists: lifetime.artists,
                albums: lifetime.albums,
                since: lifetime.since,
                plays_per_day: lifetime.plays_per_day,
                ..overview
            };
            lifetime = Overview::default();
        }
        Ok(ListeningStats {
            artists: charts.artists,
            tracks: charts.tracks,
            albums: charts.albums,
            genres,
            overview,
            lifetime,
            ..Default::default()
        })
    }
}

/// The top artists, songs and albums of a stretch of time, and how many distinct ones the
/// stretch holds where that is known.
struct Charts {
    artists: Vec<Ranked<SavedArtist>>,
    tracks: Vec<Ranked<Track>>,
    albums: Vec<Ranked<Album>>,
    distinct: [Option<usize>; 3],
}

impl AudioScrobbler {
    /// Reads one unsigned read method, which is open to the account's public profile.
    async fn read(&self, method: &str, query: &[(&str, &str)], account: &Account) -> Result<Value> {
        let key = match account.key.is_empty() {
            true => ANONYMOUS,
            false => &account.key,
        };
        let answer = super::http()
            .get(self.endpoint)
            .query(&[("method", method), ("api_key", key), ("format", "json")])
            .query(query)
            .send()
            .await
            .with_context(|| format!("cannot reach {} {method}", self.id))?;
        let status = answer.status();
        let value: Value = answer
            .json()
            .await
            .with_context(|| format!("cannot decode {} {method} (HTTP {status})", self.id))?;
        if !status.is_success() || value.get("error").is_some() {
            bail!(
                "{} {method} refused the request (HTTP {status}): {value}",
                self.id
            );
        }
        Ok(value)
    }

    /// Plays in each equal slice of the account's history, one count request for each slice. An
    /// account that cannot be read at all has none.
    async fn density(&self, account: &Account, since: i64) -> Vec<usize> {
        let now = jiff::Timestamp::now().as_second();
        let width = ((now - since) / SLICES as i64).max(1);
        // A few at a time keeps Last.fm from refusing the burst, and a slice it refused is asked
        // for once more.
        let counts: Vec<Option<usize>> = futures::stream::iter(0..SLICES as i64)
            .map(|slice| async move {
                let range = (since + slice * width, since + slice * width + width - 1);
                match self.plays_between(account, range).await {
                    None => self.plays_between(account, range).await,
                    found => found,
                }
            })
            .buffered(DENSITY_AT_ONCE)
            .collect()
            .await;
        match counts.iter().any(Option::is_some) {
            true => counts.into_iter().map(Option::unwrap_or_default).collect(),
            false => Vec::new(),
        }
    }

    /// The plays of the whole history that reached a round number. Reading them newest first one
    /// to a page, the nth play is on page `total - n + 1`.
    async fn milestones(&self, account: &Account, total: usize) -> Vec<Milestone> {
        let reads = milestone_numbers(total)
            .into_iter()
            .map(|number| async move {
                let page = (total - number + 1).to_string();
                let query = [
                    ("user", account.name.as_str()),
                    ("limit", "1"),
                    ("page", page.as_str()),
                ];
                let recent = self
                    .read("user.getRecentTracks", &query, account)
                    .await
                    .ok()?;
                // A song playing right now leads every page and has no date, so it is skipped.
                let track = rows(&recent, "recenttracks", "track")
                    .into_iter()
                    .find(|track| track.get("date").is_some())?;
                let at = number_of(track.get("date")?, "uts")?;
                Some(Milestone {
                    number,
                    title: string(track, "name"),
                    artist: credited(track),
                    at,
                })
            });
        let mut found: Vec<_> = futures::future::join_all(reads)
            .await
            .into_iter()
            .flatten()
            .collect();
        found.sort_by_key(|milestone| milestone.number);
        found
    }

    /// The top artists, songs and albums between two dates, or of all time. The weekly chart
    /// methods take any two dates, and Last.fm says as much of the range as it holds up to 1000.
    async fn charts(&self, account: &Account, dates: Option<(i64, i64)>) -> Result<Charts> {
        let user = account.name.as_str();
        let (from, to) = dates
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .unzip();
        let ranged = [
            ("user", user),
            ("from", from.as_deref().unwrap_or_default()),
            ("to", to.as_deref().unwrap_or_default()),
        ];
        let overall = [("user", user), ("period", "overall"), ("limit", "50")];
        let (query, methods, parents) = match dates {
            Some(_) => (
                &ranged[..],
                [
                    "user.getWeeklyArtistChart",
                    "user.getWeeklyTrackChart",
                    "user.getWeeklyAlbumChart",
                ],
                ["weeklyartistchart", "weeklytrackchart", "weeklyalbumchart"],
            ),
            None => (
                &overall[..],
                [
                    "user.getTopArtists",
                    "user.getTopTracks",
                    "user.getTopAlbums",
                ],
                ["topartists", "toptracks", "topalbums"],
            ),
        };
        let (artists, tracks, albums) = tokio::try_join!(
            self.read(methods[0], query, account),
            self.read(methods[1], query, account),
            self.read(methods[2], query, account),
        )?;
        let ranged = dates.is_some();
        let (artist_rows, artist_total) = kept(&artists, parents[0], "artist", ranged);
        let (track_rows, track_total) = kept(&tracks, parents[1], "track", ranged);
        let (album_rows, album_total) = kept(&albums, parents[2], "album", ranged);
        Ok(Charts {
            artists: artist_rows
                .into_iter()
                .map(|row| {
                    let artist = SavedArtist {
                        name: string(row, "name"),
                        cover: image(row),
                        ..Default::default()
                    };
                    chart_row(row, artist)
                })
                .collect(),
            tracks: track_rows
                .into_iter()
                .map(|row| {
                    let track = Track {
                        name: string(row, "name"),
                        artists: credited(row),
                        album: row
                            .get("album")
                            .map(|album| string(album, "#text"))
                            .unwrap_or_default(),
                        cover: image(row).or_else(|| row.get("album").and_then(image)),
                        ..Default::default()
                    };
                    chart_row(row, track)
                })
                .collect(),
            albums: album_rows
                .into_iter()
                .map(|row| {
                    let album = Album {
                        name: string(row, "name"),
                        artists: credited(row),
                        cover: image(row),
                        ..Default::default()
                    };
                    chart_row(row, album)
                })
                .collect(),
            distinct: [artist_total, track_total, album_total],
        })
    }

    /// How many plays there are between two dates, or ever. The recent tracks of a range say so
    /// in their first page.
    async fn plays_between(&self, account: &Account, range: (i64, i64)) -> Option<usize> {
        let (from, to) = (range.0.to_string(), range.1.to_string());
        let query = [
            ("user", account.name.as_str()),
            ("limit", "1"),
            ("from", from.as_str()),
            ("to", to.as_str()),
        ];
        let recent = self
            .read("user.getRecentTracks", &query, account)
            .await
            .ok()?;
        attr_total(&recent, "recenttracks")
    }

    /// How many plays there are between two dates (or ever), and the start times of the latest
    /// of them, newest first.
    async fn activity(
        &self,
        account: &Account,
        dates: Option<(i64, i64)>,
    ) -> Option<(usize, Vec<i64>)> {
        let (from, to) = dates
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .unzip();
        let page = |number: usize| {
            let (from, to) = (from.as_deref(), to.as_deref());
            async move {
                let number = number.to_string();
                let mut query = vec![
                    ("user", account.name.as_str()),
                    ("limit", ACTIVITY_PAGE),
                    ("page", number.as_str()),
                ];
                query.extend(from.map(|from| ("from", from)));
                query.extend(to.map(|to| ("to", to)));
                self.read("user.getRecentTracks", &query, account)
                    .await
                    .ok()
            }
        };
        // Every page is asked for at once, since the first only says how many there are. A page
        // past the end comes back empty.
        let mut chunks = futures::future::join_all((1..=ACTIVITY_PAGES).map(page))
            .await
            .into_iter();
        let first = chunks.next()??;
        let plays = attr_total(&first, "recenttracks")?;
        let times = std::iter::once(first)
            .chain(chunks.flatten())
            .flat_map(|chunk| {
                rows(&chunk, "recenttracks", "track")
                    .into_iter()
                    .filter_map(|play| number(play.get("date")?, "uts"))
                    .filter_map(|at| i64::try_from(at).ok())
                    .collect::<Vec<_>>()
            })
            .collect();
        Some((plays, times))
    }
}

/// The top rows of a chart and how many distinct rows it holds. A ranged chart is cut off at
/// `CHART_ROWS`, past which its size is unknown.
fn kept<'a>(
    chart: &'a Value,
    parent: &str,
    child: &str,
    ranged: bool,
) -> (Vec<&'a Value>, Option<usize>) {
    let all = rows(chart, parent, child);
    let distinct = match ranged {
        true => (all.len() < CHART_ROWS).then_some(all.len()),
        false => attr_total(chart, parent),
    };
    (all.into_iter().take(RANKED).collect(), distinct)
}

/// The total a Last.fm list reports for itself, under the list's `@attr`.
fn attr_total(value: &Value, parent: &str) -> Option<usize> {
    number(value.get(parent)?.get("@attr")?, "total")
}

/// A chart row's item with its play count and Last.fm page.
fn chart_row<T>(row: &Value, item: T) -> Ranked<T> {
    Ranked::new(item, number(row, "playcount")).linked(lastfm_url(row))
}

/// An allow-listed Last.fm page, used as the identity of a chart entry with no catalogue match.
fn lastfm_url(value: &Value) -> Option<String> {
    let url = string(value, "url");
    (url.starts_with("https://www.last.fm/") || url.starts_with("https://last.fm/")).then_some(url)
}

/// The largest real artwork entry, skipping Last.fm's generic placeholder.
fn image(value: &Value) -> Option<String> {
    value
        .get("image")
        .and_then(Value::as_array)?
        .iter()
        .rev()
        .filter_map(|image| image.get("#text")?.as_str())
        .map(str::trim)
        .find(|url| !url.is_empty() && !url.contains("2a96cbd8b46e442fc41c2b86b821562f"))
        .map(str::to_owned)
}

/// The chart rows under `parent.child`, which Last.fm sends as a lone object for a single row.
fn rows<'a>(value: &'a Value, parent: &str, child: &str) -> Vec<&'a Value> {
    match value.get(parent).and_then(|value| value.get(child)) {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(item @ Value::Object(_)) => vec![item],
        _ => Vec::new(),
    }
}

/// The artist of a track or album row, which is either a name or an object holding one.
fn credited(row: &Value) -> String {
    row.get("artist")
        .map(|artist| match artist.is_string() {
            true => artist.as_str().unwrap_or_default().to_owned(),
            false => Some(string(artist, "name"))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| string(artist, "#text")),
        })
        .unwrap_or_default()
}

fn string(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// When the account registered, in Unix seconds.
fn registered(user: &Value) -> Option<i64> {
    number_of(user.get("registered")?, "unixtime")
}

/// A time in Unix seconds that Last.fm sends as a string.
fn number_of(value: &Value, key: &str) -> Option<i64> {
    number(value, key).and_then(|seconds| i64::try_from(seconds).ok())
}

/// A count Last.fm sends as a string.
fn number(value: &Value, key: &str) -> Option<usize> {
    let number = value.get(key)?;
    number
        .as_str()
        .and_then(|text| text.parse().ok())
        .or_else(|| {
            number
                .as_u64()
                .and_then(|count| usize::try_from(count).ok())
        })
}

impl AudioScrobbler {
    /// Names one listen's parameter. A batching server numbers them, a single-listen one takes
    /// the plain name and would read a numbered parameter as an array.
    fn slot(&self, name: &str, index: usize) -> String {
        match self.batch > 1 {
            true => format!("{name}[{index}]"),
            false => name.to_owned(),
        }
    }

    async fn call(&self, account: &Account, mut form: Vec<(String, String)>) -> Result<()> {
        form.push(("api_key".to_owned(), account.key.clone()));
        form.push(("sk".to_owned(), account.session.clone()));
        post(self.endpoint, &account.secret, form).await.map(|_| ())
    }
}

#[derive(Deserialize)]
struct Answer {
    error: Option<u32>,
    message: Option<String>,
    session: Option<Granted>,
}

#[derive(Deserialize)]
struct Granted {
    key: String,
    name: String,
}

fn authorize_url(authorize: &str, key: &str) -> String {
    format!(
        "{authorize}?api_key={}&cb={}",
        escape::component(key),
        escape::component(&format!("http://127.0.0.1:{PORT}{PATH}"))
    )
}

/// Waits for the browser to come back with the request token. Anything else that reaches the
/// listener is answered and ignored, so a stray favicon fetch does not end the wait.
async fn token(listener: TcpListener) -> Result<String> {
    let deadline = tokio::time::Instant::now() + WAIT;

    loop {
        let accepted = tokio::time::timeout_at(deadline, listener.accept())
            .await
            .context("the browser did not come back in time")?;
        let (mut stream, _) = accepted.context("cannot accept the scrobbling callback")?;

        let mut line = String::new();
        let read = BufReader::new(&mut stream).read_line(&mut line).await;
        if let Err(error) = read {
            log::warn!("scrobble: cannot read the callback: {error}");
            continue;
        }
        let found = parameter(&line, "token");
        stream.write_all(PAGE.as_bytes()).await.ok();
        if let Some(token) = found {
            return Ok(token);
        }
    }
}

/// Signs the form the way the protocol asks, posts it and turns a refusal into an error.
async fn post(endpoint: &str, secret: &str, mut form: Vec<(String, String)>) -> Result<Answer> {
    form.sort_by(|left, right| left.0.cmp(&right.0));
    form.push(("api_sig".to_owned(), signature(&form, secret)));
    form.push(("format".to_owned(), "json".to_owned()));

    let answer: Answer = super::http()
        .post(endpoint)
        .form(&form)
        .send()
        .await
        .context("cannot reach the scrobbling service")?
        .json()
        .await
        .context("cannot read the scrobbling answer")?;

    if let Some(code) = answer.error {
        let message = answer.message.unwrap_or_default();
        bail!("the scrobbling service refused the request ({code}): {message}");
    }
    Ok(answer)
}

fn signature(form: &[(String, String)], secret: &str) -> String {
    let mut base = String::new();
    for (name, value) in form {
        base.push_str(name);
        base.push_str(value);
    }
    base.push_str(secret);
    format!("{:x}", Md5::digest(base.as_bytes()))
}

fn parameter(request: &str, name: &str) -> Option<String> {
    let (_, query) = request.split_whitespace().nth(1)?.split_once('?')?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| percent_decode_str(value).decode_utf8_lossy().into_owned())
    })
}
