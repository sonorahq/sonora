//! NetEase data access, over the endpoints the site's own web player calls.
//!
//! None of this is an official api — NetEase publishes none — so every path here is one the
//! site's pages or its desktop client use, called with the `MUSIC_U` cookie a browser holds.
//! The catalog answers without an account; everything scoped to the listener, the likes and
//! the playlists and the stream urls, needs one.
//!
//! The site answers HTTP 200 to almost every call, whatever went wrong, and puts the reason in
//! the json `code`, so `check` is what turns one into an error.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::netease::{auth, wire::Record};
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, GenreItem, GenreSection, HomeFeed, MediaKind,
    MusicApi, Page, Pages, Playlist, PlaylistDetail, SavedArtist, Track, UserDetail, UserProfile,
    distinct_covers,
};

/// Every call goes here. The site's pages and its desktop client both use this prefix, and it
/// answers json in the shape those pages were written against.
const API: &str = "https://music.163.com/api";
/// The site a call pretends to come from. NetEase refuses one without it.
const SITE: &str = "https://music.163.com";
/// What the web player says about itself. The site's risk control is quicker to drop a call
/// from an agent it does not recognise.
const AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) \
                    Chrome/120.0.0.0 Safari/537.36";
/// How long the encoded id list one `song/detail` call may carry. The site answers code 400 to
/// a request line past about eight kilobytes — nginx's own header limit — and the ids are
/// seven to ten digits each, so a batch is cut by the length of the encoded list rather than by
/// a count of ids, which would drift with the ids themselves.
const BATCH: usize = 7000;
/// How many rows one listing asks for at a time.
const PAGE: usize = 100;
/// How many portraits one lookup fills in, so a search that ranks a wall of names does not fan
/// out without end.
const PORTRAIT_LIMIT: usize = 24;
/// How many tracks a station seeds with.
const RADIO_COUNT: usize = 25;
/// How many cards a home shelf holds.
const SHELF: usize = 12;
/// The audio level a stream asks for. The site answers the best audio this account's rights
/// reach at or below it — a free account gets 320 kbps mp3 where a subscription gets the
/// lossless file — and names what it granted in `level`. So one ask is enough, and asking the
/// best is the only honest choice: what an account may play is the site's to decide, not a
/// profile field read once at sign-in.
///
/// Hi-Res is deliberately not asked for. Its rates are 48 kHz and up, and the mixer resamples
/// anything that is not the output's own rate by discarding samples (rodio's own
/// `SampleRateConverter` says so), so on an output running at 44.1 kHz it would sound worse than
/// the lossless file it is meant to beat.
const LEVEL: &str = "lossless";
/// The container the stream endpoint is asked about. It refuses a call without one — answering
/// `{"msg":"参数错误","code":400}` whatever the level is — so it goes out on every call. `flac`
/// is what the site's own clients send, and the levels below the lossless ones ignore it.
const ENCODE: &str = "flac";
/// What the home shelves are called. The provider layer carries no language of its own, so
/// these are English, the way every other provider's shelves are.
const RECOMMENDED: &str = "Recommended playlists";
const NEW_SONGS: &str = "New songs";
const NEW_ALBUMS: &str = "New albums";

/// What the account is, as far as the site has said: its id and its name. What it may play is
/// not kept here, because the site answers that per track.
#[derive(Clone, Default)]
struct Account {
    id: String,
    name: String,
}

#[derive(Clone)]
pub struct NeteaseClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    /// The whole `Cookie` header every call carries, built once at connect.
    cookies: String,
    /// The `__csrf` token a write sends back beside the cookies.
    csrf: String,
    account: RwLock<Account>,
}

impl NeteaseClient {
    /// Signs in with the cookies a browser holds.
    /// Cookies the site refuses answer an account of null, which is what makes this the check:
    /// a session that was signed in and has since expired fails here rather than halfway
    /// through a page.
    pub(crate) async fn connect(credentials: &auth::Credentials) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(AGENT)
            .build()
            .context("cannot build the netease http client")?;
        let client = Self {
            inner: Arc::new(Inner {
                http,
                cookies: credentials.header(),
                csrf: credentials.csrf.clone(),
                account: RwLock::new(Account::default()),
            }),
        };
        let account = client.account().await?;
        if account.id.is_empty() {
            bail!("the cookies were refused; sign in to music.163.com again");
        }
        *client.inner.account.write().await = account;
        Ok(client)
    }

    async fn user_id(&self) -> String {
        self.inner.account.read().await.id.clone()
    }

    /// The account the cookies stand for. `account` is null when the site did not accept them,
    /// which is also how a session that has since expired shows up.
    async fn account(&self) -> Result<Account> {
        let answer = self.get("nuser/account/get", &[]).await?;
        let account = answer.get("account").filter(|account| !account.is_null());
        let profile = answer.get("profile").filter(|profile| !profile.is_null());
        Ok(Account {
            id: account
                .and_then(|account| account.get("id"))
                .and_then(|value| Record::new(value).id())
                .unwrap_or_default(),
            name: profile
                .and_then(|profile| Record::new(profile).text(&["nickname"]))
                .unwrap_or_default()
                .to_owned(),
        })
    }

    /// One GET on the site's api. The endpoint answers HTTP 200 to almost everything, so the
    /// json `code` is what decides whether it worked.
    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        let response = self
            .inner
            .http
            .get(format!("{API}/{path}"))
            .query(query)
            .header("Referer", SITE)
            .header("Cookie", &self.inner.cookies)
            .send()
            .await
            .with_context(|| format!("cannot reach netease for {path}"))?
            .error_for_status()
            .with_context(|| format!("netease refused {path}"))?;
        self.read(path, response).await
    }

    /// One POST on the site's api, carrying the CSRF token the site asks for beside the
    /// cookies. Every write in this file goes through it.
    async fn post(&self, path: &str, form: &[(&str, String)]) -> Result<Value> {
        let mut form: Vec<(&str, &str)> = form
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        form.push(("csrf_token", self.inner.csrf.as_str()));
        let response = self
            .inner
            .http
            .post(format!("{API}/{path}"))
            .form(&form)
            .header("Referer", SITE)
            .header("Cookie", &self.inner.cookies)
            .send()
            .await
            .with_context(|| format!("cannot reach netease for {path}"))?
            .error_for_status()
            .with_context(|| format!("netease refused {path}"))?;
        self.read(path, response).await
    }

    /// Reads the json one answer carries, and only its first value.
    ///
    /// The site's edge answers a refused call with the same error object twice over, in one
    /// body — `{"msg":"参数错误","code":400}` repeats before the next byte — which no whole-body
    /// parse survives. Reading the first value instead makes such an answer arrive at `check`
    /// with the site's own reason, rather than as a decoder error about trailing characters.
    async fn read(&self, path: &str, response: reqwest::Response) -> Result<Value> {
        let body = response
            .text()
            .await
            .with_context(|| format!("cannot read what netease answered for {path}"))?;
        let mut values = serde_json::Deserializer::from_str(&body).into_iter::<Value>();
        match values.next() {
            Some(Ok(answer)) => check(path, answer),
            Some(Err(error)) => bail!("netease answered {path} with no json: {error}"),
            None => bail!("netease answered {path} with an empty body"),
        }
    }

    /// Resolves ids to tracks, keeping the order the ids came in and dropping the ones the
    /// site no longer knows. Every listing of tracks goes through here, so a page never
    /// re-parses track fields of its own.
    async fn songs(&self, ids: &[String]) -> Result<Vec<Track>> {
        let mut tracks = Vec::with_capacity(ids.len());
        for batch in batches(ids) {
            let wanted: Vec<Value> = batch.iter().map(|id| json!({ "id": id })).collect();
            let answer = self
                .get("v3/song/detail", &[("c", serde_json::to_string(&wanted)?)])
                .await?;
            let mut found: HashMap<String, Track> = Record::new(&answer)
                .tracks(&["songs"])
                .into_iter()
                .filter_map(|track| track.id.clone().map(|id| (id, track)))
                .collect();
            for id in batch {
                if let Some(track) = found.remove(id) {
                    tracks.push(track);
                }
            }
        }
        Ok(tracks)
    }

    /// The ids the account has liked, which is the whole songs library on this provider.
    async fn liked_ids(&self) -> Result<Vec<String>> {
        let uid = self.user_id().await;
        let answer = self.get("song/like/get", &[("uid", uid)]).await?;
        Ok(answer
            .get("ids")
            .and_then(Value::as_array)
            .map(|ids| {
                ids.iter()
                    .filter_map(|value| Record::new(value).id())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The account's playlists, one page at a time. NetEase keeps the collected ones behind a
    /// second endpoint that has no plaintext form, so this is the only list the app has: what
    /// the site reports for a user here is what the playlists page shows.
    async fn playlists_of(&self, uid: &str) -> Result<Vec<Playlist>> {
        let mut playlists = Vec::new();
        let mut offset = 0usize;
        loop {
            let answer = self
                .get(
                    "user/playlist",
                    &[
                        ("uid", uid.to_owned()),
                        ("limit", PAGE.to_string()),
                        ("offset", offset.to_string()),
                    ],
                )
                .await?;
            let page = answer
                .get("playlist")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // the site says outright whether another page follows; where it does not, a page
            // that came back full is what says so
            let more = answer
                .get("more")
                .and_then(Value::as_bool)
                .unwrap_or(page.len() == PAGE);
            playlists.extend(
                page.iter()
                    .filter_map(|value| Record::new(value).playlist(uid)),
            );
            if page.is_empty() || !more {
                return Ok(playlists);
            }
            offset += page.len();
        }
    }

    /// The ids of a playlist's tracks, in order. `n=0` asks for the ids alone, which is what a
    /// list of a thousand tracks wants: the records come from `songs` afterwards, in batches.
    async fn playlist_ids(&self, playlist_id: &str) -> Result<Vec<String>> {
        let answer = self
            .get(
                "v6/playlist/detail",
                &[("id", playlist_id.to_owned()), ("n", "0".to_owned())],
            )
            .await?;
        Ok(Record::new(&answer["playlist"]).ids(&["trackIds"]))
    }

    /// The url one track's audio is served from: one call, at the best level, and the site's
    /// own answer decides what that track actually sounds like.
    async fn stream_url(&self, track_id: &str) -> Result<String> {
        let answer = self
            .get(
                "song/enhance/player/url/v1",
                &[
                    ("ids", format!("[{track_id}]")),
                    ("level", LEVEL.to_owned()),
                    ("encodeType", ENCODE.to_owned()),
                ],
            )
            .await?;
        let entry = answer
            .get("data")
            .and_then(Value::as_array)
            .and_then(|list| list.first());
        let detail = entry.unwrap_or(&Value::Null);
        let granted = Record::new(detail).text(&["level"]).unwrap_or(LEVEL);
        match Record::new(detail)
            .text(&["url"])
            .filter(|url| !url.is_empty())
        {
            // a track the plan only offers a taste of answers a url with `freeTrialInfo` beside
            // it, and the file behind it is half a minute long. A queue of full tracks is worth
            // more than a clip, so it is passed over like an unavailable one.
            Some(url) if !trial(entry) => {
                // the level asked for and the level granted are two different things, and the
                // second is the one worth knowing: it is what this account actually hears
                log::debug!(
                    "netease: {track_id} streams at {granted} ({} bps, {})",
                    Record::new(detail).number(&["br"]).unwrap_or(0),
                    Record::new(detail).text(&["type"]).unwrap_or("unknown"),
                );
                Ok(url.to_owned())
            }
            Some(_) => bail!("netease offers only a trial clip of {track_id}"),
            None => bail!("netease will not stream {track_id}; the site granted {granted}"),
        }
    }

    /// Opens the audio of a track: the url, then the GET itself.
    pub(crate) async fn open_stream(&self, track_id: &str) -> Result<reqwest::Response> {
        let url = self.stream_url(track_id).await?;
        self.inner
            .http
            .get(&url)
            .header("Referer", SITE)
            .send()
            .await
            .context("cannot stream the netease track")?
            .error_for_status()
            .context("the netease cdn refused the stream")
    }

    /// The albums of an artist, newest first.
    async fn artist_albums(&self, artist_id: &str, wanted: usize) -> Result<Vec<Album>> {
        let answer = self
            .get(
                &format!("artist/albums/{artist_id}"),
                &[("limit", wanted.to_string()), ("offset", "0".to_owned())],
            )
            .await?;
        Ok(answer
            .get("hotAlbums")
            .and_then(Value::as_array)
            .map(|albums| {
                albums
                    .iter()
                    .filter_map(|value| Record::new(value).album())
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The account's liked songs, which on this provider is the whole songs library.
    async fn library_tracks(&self) -> Result<Vec<Track>> {
        let ids = self.liked_ids().await?;
        self.songs(&ids).await
    }

    /// What the listener played lately, as cards. The endpoint refuses an account that has not
    /// asked for its own history, so a failure here leaves the shelf out rather than failing
    /// the page.
    async fn listen_again(&self) -> Vec<GenreItem> {
        let uid = self.user_id().await;
        let answer = self
            .get("v1/play/record", &[("uid", uid), ("type", "1".to_owned())])
            .await
            .map_err(|error| log::debug!("netease: no play history: {error:#}"));
        let Ok(answer) = answer else {
            return Vec::new();
        };
        answer
            .get("allData")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| Record::new(&row["song"]).item())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What the site recommends today, which is what the quick picks are.
    async fn daily_picks(&self) -> Vec<Track> {
        let answer = self
            .get("discovery/recommend/songs", &[("limit", SHELF.to_string())])
            .await
            .map_err(|error| log::debug!("netease: no daily picks: {error:#}"));
        let Ok(answer) = answer else {
            return Vec::new();
        };
        Record::new(&answer["data"]).tracks(&["dailySongs"])
    }

    /// One home shelf from a listing, if the listing answered anything at all. A shelf whose
    /// endpoint failed is left out rather than failing the page: the three come from three
    /// different endpoints and any one of them can be having a bad day.
    async fn shelf(
        &self,
        title: &str,
        holds: Holds,
        path: &str,
        query: &[(&str, String)],
        key: &str,
    ) -> Option<GenreSection> {
        let answer = self
            .get(path, query)
            .await
            .map_err(|error| log::debug!("netease: no {path} shelf: {error:#}"))
            .ok()?;
        let items: Vec<GenreItem> = answer
            .get(key)
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(|row| holds.read(row)).collect())
            .unwrap_or_default();
        (!items.is_empty()).then(|| GenreSection {
            title: title.to_owned(),
            items,
        })
    }

    /// One search, in the shape the site's own search page uses. The three kinds differ only
    /// in the type number, and each answers under its own plural.
    async fn search_for(&self, query: &str, kind: u32) -> Result<Value> {
        self.get(
            "cloudsearch/pc",
            &[
                ("s", query.to_owned()),
                ("type", kind.to_string()),
                ("limit", "50".to_owned()),
                ("offset", "0".to_owned()),
            ],
        )
        .await
    }

    /// Adds or removes one track on a playlist. The ids travel as a json list, which is what
    /// the endpoint parses, and the site spells a removal `del`.
    async fn manipulate(&self, playlist_id: &str, track_id: &str, op: &str) -> Result<()> {
        let ids = json!([track_id]).to_string();
        self.post(
            "playlist/manipulate/tracks",
            &[
                ("op", op.to_owned()),
                ("pid", playlist_id.to_owned()),
                ("trackIds", ids),
            ],
        )
        .await
        .map(|_| ())
        .with_context(|| format!("cannot change the netease playlist {playlist_id}"))
    }
}

/// Cuts a list of ids into batches whose encoded json list stays inside what the site accepts.
///
/// The length is worked out rather than measured: one entry is `{"id":"<id>"}`, and every
/// character of it that is not a letter or a digit takes three bytes once encoded, which for
/// that shape is exactly `id.len() + 23` — plus the comma joining it to the last, which takes
/// three more. A batch that does not fit is the one thing here that is not worth finding out
/// from the site, since its answer to an oversized request line is a bare code 400.
fn batches(ids: &[String]) -> Vec<&[String]> {
    let mut batches: Vec<&[String]> = Vec::new();
    let mut start = 0;
    let mut length = 0;
    for (index, id) in ids.iter().enumerate() {
        let entry = id.len() + 26;
        if index > start && length + entry > BATCH {
            batches.push(&ids[start..index]);
            start = index;
            length = 0;
        }
        length += entry;
    }
    if start < ids.len() {
        batches.push(&ids[start..]);
    }
    debug_assert!(batches.iter().all(|batch| encoded(batch) <= BATCH));
    batches
}

/// How long a batch's id list is once it is percent-encoded into the query.
fn encoded(ids: &[String]) -> usize {
    let list = Value::Array(ids.iter().map(|id| json!({ "id": id })).collect());
    utf8_percent_encode(&list.to_string(), NON_ALPHANUMERIC)
        .to_string()
        .len()
}

/// What a home shelf holds, so each listing is read with the conversion its own shape needs.
/// The three shelves answer in three different dialects and none of them says which it is.
#[derive(Clone, Copy)]
enum Holds {
    Playlists,
    Songs,
    Albums,
}

impl Holds {
    fn read(self, value: &Value) -> Option<GenreItem> {
        match self {
            Self::Playlists => Record::new(value).playlist("").map(GenreItem::Playlist),
            Self::Songs => Record::new(value).item(),
            Self::Albums => Record::new(value).album().map(GenreItem::Album),
        }
    }
}

/// Reads the `code` every answer carries. An endpoint that omits it means success; anything
/// else is mapped to something a caller can act on.
fn check(path: &str, answer: Value) -> Result<Value> {
    match answer.get("code").and_then(Value::as_i64) {
        None | Some(200) => Ok(answer),
        Some(301) => bail!("netease {path} needs a signed-in account"),
        Some(-2) => bail!("the account has no access to netease {path}"),
        Some(-462) => bail!("netease {path} wants a verification this app cannot answer"),
        Some(400) => bail!("netease {path} refused the parameters"),
        Some(404) => bail!("netease has no {path} endpoint"),
        Some(code) => bail!("netease {path} answered code {code}"),
    }
}

/// Whether the stream a level answered with is a trial clip rather than the whole track.
fn trial(entry: Option<&Value>) -> bool {
    entry
        .and_then(|entry| entry.get("freeTrialInfo"))
        .is_some_and(|trial| !trial.is_null())
}

#[async_trait]
impl MusicApi for NeteaseClient {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let page = match kind {
            MediaKind::Track => "song",
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
        };
        Some(format!("{SITE}/#/{page}?id={id}"))
    }

    async fn profile(&self) -> Result<UserProfile> {
        let account = self.inner.account.read().await.clone();
        Ok(UserProfile {
            id: account.id,
            display_name: account.name,
        })
    }

    async fn user(&self, user_id: &str) -> Result<UserDetail> {
        let answer = self.get(&format!("v1/user/detail/{user_id}"), &[]).await?;
        let profile = answer.get("profile").unwrap_or(&Value::Null);
        let playlists = self.playlists_of(user_id).await.unwrap_or_default();
        Ok(UserDetail {
            id: user_id.to_owned(),
            name: Record::new(profile)
                .text(&["nickname"])
                .unwrap_or_default()
                .to_owned(),
            avatar: Record::new(profile).picture(&["avatarUrl"], 300),
            followers: Record::new(profile).number(&["followeds"]),
            following: Record::new(profile).number(&["follows"]),
            playlists,
        })
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let answer = self
            .get(&format!("v1/artist/{artist_id}"), &[])
            .await
            .context("cannot load the netease artist")?;
        let artist = answer.get("artist").unwrap_or(&Value::Null);
        let albums = self.artist_albums(artist_id, 30).await.unwrap_or_default();
        Ok(Artist {
            name: Record::new(artist)
                .text(&["name"])
                .unwrap_or_default()
                .to_owned(),
            cover_large: Record::new(artist).picture(&["picUrl"], 1000),
            biography: Record::new(artist)
                .text(&["briefDesc"])
                .filter(|bio| !bio.trim().is_empty())
                .map(str::to_owned),
            monthly_listeners: None,
            top_tracks: Record::new(&answer).tracks(&["hotSongs"]),
            albums,
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let answer = self
            .get(&format!("v1/artist/{artist_id}"), &[])
            .await
            .context("cannot load the netease artist")?;
        let artist = answer.get("artist").unwrap_or(&Value::Null);
        Ok(ArtistProfile {
            name: Record::new(artist)
                .text(&["name"])
                .unwrap_or_default()
                .to_owned(),
            cover_large: Record::new(artist).picture(&["picUrl"], 1000),
            biography: Record::new(artist)
                .text(&["briefDesc"])
                .filter(|bio| !bio.trim().is_empty())
                .map(str::to_owned),
        })
    }

    /// Looks up the portraits a listing left out, one artist at a time. The site's risk
    /// control is quicker to drop a burst of calls than a slow trickle, so this is deliberately
    /// not fanned out, and an artist it will not answer for is simply left out.
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let mut images = HashMap::new();
        for id in ids.into_iter().take(PORTRAIT_LIMIT) {
            let Ok(answer) = self.get(&format!("v1/artist/{id}"), &[]).await else {
                continue;
            };
            if let Some(cover) = Record::new(&answer["artist"]).picture(&["picUrl"], 300) {
                images.insert(id, cover);
            }
        }
        Ok(images)
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        self.library_tracks().await
    }

    /// The liked songs a page at a time, so a library of a few thousand draws its first rows
    /// while the rest are still being resolved. The albums and artists the listener collected
    /// are listed whole: they run to dozens, not thousands, so a page at a time buys nothing
    /// there.
    async fn saved_tracks_paged(&self) -> Result<Pages<Track>> {
        let ids = self.liked_ids().await?;
        let total = Some(ids.len());
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        let client = self.clone();
        tokio::spawn(async move {
            for batch in batches(&ids) {
                let page = match client.songs(batch).await {
                    Ok(items) => Page { total, items },
                    Err(error) => {
                        sender.send(Err(error)).await.ok();
                        return;
                    }
                };
                if sender.send(Ok(page)).await.is_err() {
                    return;
                }
            }
        });
        Ok(receiver)
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.post(
            "song/like",
            &[
                ("trackId", track_id.to_owned()),
                ("like", saved.to_string()),
            ],
        )
        .await
        .map(|_| ())
        .context("cannot change whether netease has this track liked")
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let wanted = vec![track_id.to_owned()];
        self.songs(&wanted)
            .await?
            .into_iter()
            .next()
            .with_context(|| format!("netease does not know the track {track_id}"))
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let uid = self.user_id().await;
        self.playlists_of(&uid).await
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let answer = self
            .post("playlist/create", &[("name", name.to_owned())])
            .await
            .context("cannot create the netease playlist")?;
        answer
            .get("id")
            .and_then(|value| Record::new(value).id())
            .context("netease made a playlist without an id")
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.post(
            "playlist/update/name",
            &[("id", playlist_id.to_owned()), ("name", name.to_owned())],
        )
        .await
        .map(|_| ())
        .context("cannot rename the netease playlist")
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.post("playlist/delete", &[("id", playlist_id.to_owned())])
            .await
            .map(|_| ())
            .context("cannot delete the netease playlist")
    }

    /// A playlist the listener collected is taken back out of their library. The site keeps
    /// that apart from owning one, which is what `delete_playlist` is for.
    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        self.post("playlist/unsubscribe", &[("id", playlist_id.to_owned())])
            .await
            .map(|_| ())
            .context("cannot take the netease playlist out of the library")
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        self.post("playlist/subscribe", &[("id", playlist_id.to_owned())])
            .await
            .map(|_| ())
            .context("cannot collect the netease playlist")
    }

    /// NetEase keeps a playlist's visibility at 0 for a public one and 10 for a private one.
    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        let privacy = match public {
            true => "0",
            false => "10",
        };
        self.post(
            "playlist/update/privacy",
            &[
                ("id", playlist_id.to_owned()),
                ("privacy", privacy.to_owned()),
            ],
        )
        .await
        .map(|_| ())
        .context("cannot change the netease playlist's visibility")
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.manipulate(playlist_id, track_id, "add").await
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.manipulate(playlist_id, track_id, "del").await
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let mut albums = Vec::new();
        let mut offset = 0usize;
        loop {
            let answer = self
                .get(
                    "album/sublist",
                    &[("limit", PAGE.to_string()), ("offset", offset.to_string())],
                )
                .await?;
            let page = answer
                .get("data")
                .or_else(|| answer.get("albums"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let more = answer
                .get("hasMore")
                .and_then(Value::as_bool)
                .unwrap_or(page.len() == PAGE);
            albums.extend(page.iter().filter_map(|value| Record::new(value).album()));
            if page.is_empty() || !more {
                return Ok(albums);
            }
            offset += page.len();
        }
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        let path = match saved {
            true => "album/sub",
            false => "album/unsub",
        };
        self.post(path, &[("id", album_id.to_owned())])
            .await
            .map(|_| ())
            .context("cannot change whether netease has this album collected")
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let mut artists = Vec::new();
        let mut offset = 0usize;
        loop {
            let answer = self
                .get(
                    "artist/sublist",
                    &[("limit", PAGE.to_string()), ("offset", offset.to_string())],
                )
                .await?;
            let page = answer
                .get("data")
                .or_else(|| answer.get("artists"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let more = answer
                .get("hasMore")
                .and_then(Value::as_bool)
                .unwrap_or(page.len() == PAGE);
            artists.extend(
                page.iter()
                    .filter_map(|value| Record::new(value).saved_artist()),
            );
            if page.is_empty() || !more {
                return Ok(artists);
            }
            offset += page.len();
        }
    }

    /// NetEase names the artist here rather than the track, and spells the two directions
    /// differently: following takes a lone `artistId` and unfollowing a plural `artistIds`.
    /// Each is refused with a parameter error for the other.
    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        let (path, key) = match saved {
            true => ("artist/sub", "artistId"),
            false => ("artist/unsub", "artistIds"),
        };
        self.post(path, &[(key, artist_id.to_owned())])
            .await
            .map(|_| ())
            .context("cannot change whether netease has this artist followed")
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let answer = self
            .get(&format!("v1/album/{album_id}"), &[])
            .await
            .context("cannot load the netease album")?;
        let source = answer.get("album").unwrap_or(&Value::Null);
        let tracks = Record::new(&answer).tracks(&["songs"]);
        let mut album = Record::new(source)
            .album()
            .with_context(|| format!("netease does not know the album {album_id}"))?;
        if album.track_count == 0 {
            album.track_count = tracks.len() as u32;
        }
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let answer = self
            .get(
                "v6/playlist/detail",
                &[("id", playlist_id.to_owned()), ("n", "0".to_owned())],
            )
            .await
            .context("cannot load the netease playlist")?;
        let source = answer.get("playlist").unwrap_or(&Value::Null);
        let uid = self.user_id().await;
        let mut playlist = Record::new(source)
            .playlist(&uid)
            .with_context(|| format!("netease does not know the playlist {playlist_id}"))?;
        let ids = Record::new(source).ids(&["trackIds"]);
        let tracks = self.songs(&ids).await?;
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
        let ids = self.playlist_ids(playlist_id).await?;
        self.songs(&ids).await
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let ids: Vec<String> = self
            .playlist_ids(playlist_id)
            .await?
            .into_iter()
            .take(50)
            .collect();
        let tracks = self.songs(&ids).await?;
        Ok(distinct_covers(&tracks, wanted))
    }

    /// The site's similar tracks come as one batch, without a continuation.
    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let answer = self
            .get(
                "discovery/simiSong",
                &[
                    ("songid", track_id.to_owned()),
                    ("limit", RADIO_COUNT.to_string()),
                ],
            )
            .await;
        match answer {
            Ok(answer) => Ok((Record::new(&answer).tracks(&["songs"]), None)),
            Err(error) => {
                log::warn!("netease: no station for {track_id}: {error:#}");
                Ok((Vec::new(), None))
            }
        }
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let answer = self
            .search_for(query, 1)
            .await
            .context("cannot search netease")?;
        Ok(Record::new(&answer["result"]).tracks(&["songs"]))
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let answer = self
            .search_for(query, 10)
            .await
            .context("cannot search netease albums")?;
        Ok(answer["result"]
            .get("albums")
            .and_then(Value::as_array)
            .map(|albums| {
                albums
                    .iter()
                    .filter_map(|value| Record::new(value).album())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let answer = self
            .search_for(query, 1000)
            .await
            .context("cannot search netease playlists")?;
        Ok(answer["result"]
            .get("playlists")
            .and_then(Value::as_array)
            .map(|playlists| {
                playlists
                    .iter()
                    .filter_map(|value| Record::new(value).playlist(""))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// The site's front page, as shelves. Every one of them is optional: a shelf whose listing
    /// failed is left out rather than failing the page, since the four come from four different
    /// endpoints and any one of them can be having a bad day.
    async fn home(&self) -> Result<HomeFeed> {
        let limit = [("limit", SHELF.to_string())];
        let newest = [("area", "ALL".to_owned()), ("limit", SHELF.to_string())];
        let (listen_again, quick_picks, recommended, songs, albums) = tokio::join!(
            self.listen_again(),
            self.daily_picks(),
            self.shelf(
                RECOMMENDED,
                Holds::Playlists,
                "personalized/playlist",
                &limit,
                "result"
            ),
            self.shelf(
                NEW_SONGS,
                Holds::Songs,
                "personalized/newsong",
                &limit,
                "result"
            ),
            self.shelf(NEW_ALBUMS, Holds::Albums, "album/new", &newest, "albums"),
        );
        Ok(HomeFeed {
            listen_again,
            quick_picks: (!quick_picks.is_empty()).then_some(quick_picks),
            sections: [recommended, songs, albums].into_iter().flatten().collect(),
        })
    }
}
