//! YouTube's Lounge API, through which a phone remote-controls the YouTube app on a TV. Once a
//! screen is known by the id its Cast receiver gives out, Sonora binds to it as a remote: it sets
//! what plays, sends the transport commands, and long-polls for what the screen reports. The
//! screen streams from YouTube itself, so it plays on without Sonora.
//!
//! A YouTube Music receiver, such as the one a speaker runs, plays only for a remote signed in
//! to an account. Such a remote speaks to the lounge through YouTube Music, as its web app
//! does, with the account's cookies, their hash and a token made from them.
//!
//! The protocol is not documented. It follows what pyytlounge and casttube send, and what the
//! YouTube Music web app sends when signed in.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE;
use reqwest::{RequestBuilder, StatusCode};
use serde_json::Value;
use sha1::{Digest as _, Sha1};

/// Where a guest remote and a signed-in one reach the lounge.
const GUEST: &str = "https://www.youtube.com";
const SIGNED_IN: &str = "https://music.youtube.com";
/// The header every bound request carries the lounge token in.
const TOKEN_HEADER: &str = "X-YouTube-LoungeId-Token";
const ANSWER: Duration = Duration::from_secs(15);
/// The screen's `state` values that mean its video moves on: playing, starting, and an advert
/// or a skipped one before it. Paused, ended, cued and unstarted do not.
const PLAYING: i64 = 1;
const STARTING: i64 = 3;
const ADVERT: i64 = 1081;
const ADVERT_SKIPPED: i64 = 1082;
/// The statuses that mean the server forgot the session or its token ran out.
const FORGOTTEN: [StatusCode; 3] = [
    StatusCode::BAD_REQUEST,
    StatusCode::UNAUTHORIZED,
    StatusCode::GONE,
];

/// The account a remote signs in to the lounge with: the cookie store ytmusic keeps current as
/// Google rotates the session, and which of the signed-in accounts it uses.
#[derive(Clone, Debug)]
pub(crate) struct Account {
    pub cookies: PathBuf,
    /// The `Cookie` header the session was signed in with, for as long as ytmusic has not
    /// written the store yet, which after a fresh sign-in can take until the first rotation.
    pub signed_in: String,
    pub authuser: usize,
}

/// What one screen last reported.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Screen {
    /// The video the screen has, paused or ended too. `None` once it stopped altogether.
    pub video: Option<String>,
    /// Where the screen's own queue is, counted from 0, when it said.
    pub index: Option<usize>,
    pub playing: bool,
    pub position: Duration,
    pub duration: Duration,
    /// From 0 to 1.
    pub volume: Option<f32>,
    /// When `position` was measured.
    pub stamp: Option<SystemTime>,
    /// When the screen last said it may not play its video, until it plays one again.
    pub refused: Option<Instant>,
}

/// How a long poll ended.
pub(super) enum Ended {
    /// The poll ran out as polls do. The next one carries on.
    Done,
    /// The server forgot the session, so the remote has to bind again.
    Rebind,
    /// The screen left: the app was closed or another remote took the lounge.
    Gone,
}

/// One bound remote on one screen. Commands and the long poll share it, so what changes as
/// they run sits behind a lock that is never held across a request.
pub(super) struct Lounge {
    http: reqwest::Client,
    screen: String,
    /// This remote's id, kept across rebinds so the screen sees one remote.
    id: String,
    name: String,
    /// Which YouTube app the remote binds as, which has to match the screen's.
    theme: &'static str,
    account: Option<Account>,
    bound: Mutex<Bound>,
}

/// What identifies the bound session, and where its counters stand.
#[derive(Default)]
struct Bound {
    token: String,
    sid: String,
    gsession: String,
    /// The id of the last event read, which the next poll starts after.
    aid: Option<i64>,
    /// The number of the next command map.
    ofs: u64,
    rid: u64,
}

impl Lounge {
    /// Binds to the screen with the given screen id as a remote named `name` of the YouTube app
    /// `theme` names, signed in to `account` when there is one, and returns the lounge with what
    /// the screen reported on the way.
    pub async fn bind(
        http: reqwest::Client,
        screen: String,
        id: String,
        name: String,
        theme: &'static str,
        account: Option<Account>,
    ) -> Result<(Self, Screen)> {
        let lounge = Self {
            http,
            screen,
            id,
            name,
            theme,
            account,
            bound: Mutex::new(Bound::default()),
        };
        lounge.bound().token = lounge.token().await?;
        let mut reported = Screen::default();
        lounge.connect(&mut reported).await?;
        Ok((lounge, reported))
    }

    /// Binds again after the server forgot the session, with a fresh token when the old one
    /// expired.
    pub async fn rebind(&self, reported: &mut Screen) -> Result<()> {
        if self.connect(reported).await.is_ok() {
            return Ok(());
        }
        let token = self.token().await?;
        self.bound().token = token;
        self.connect(reported).await
    }

    fn bound(&self) -> MutexGuard<'_, Bound> {
        self.bound.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn connect(&self, reported: &mut Screen) -> Result<()> {
        let token = self.bound().token.clone();
        let body = [
            ("app", "web"),
            ("mdx-version", "3"),
            ("name", self.name.as_str()),
            ("id", self.id.as_str()),
            ("device", "REMOTE_CONTROL"),
            ("capabilities", "que,dsdtr,atp"),
            ("magnaKey", "cloudPairedDevice"),
            ("ui", "false"),
            ("theme", self.theme),
            ("loungeIdToken", token.as_str()),
        ];
        let response = self
            .signed(
                self.http
                    .post(format!("{}/api/lounge/bc/bind", self.origin())),
            )
            .query(&[
                ("RID", "1"),
                ("VER", "8"),
                ("CVER", "1"),
                ("auth_failure_option", "send_error"),
            ])
            .header(TOKEN_HEADER, &token)
            .form(&body)
            .timeout(ANSWER)
            .send()
            .await
            .context("cannot bind to the youtube screen")?
            .error_for_status()
            .context("youtube refused the bind to the screen")?;
        let text = response
            .text()
            .await
            .context("cannot read the youtube bind")?;
        {
            let mut bound = self.bound();
            bound.sid.clear();
            bound.gsession.clear();
            bound.aid = None;
            bound.ofs = 1;
            bound.rid = 1;
        }
        let mut chunks = Chunks::default();
        chunks.feed(text.as_bytes());
        let mut gone = false;
        while let Some(events) = chunks.next() {
            gone |= self.apply(events, reported);
        }
        anyhow::ensure!(!gone, "the youtube screen turned the remote away");
        let bound = self.bound();
        anyhow::ensure!(
            !bound.sid.is_empty() && !bound.gsession.is_empty(),
            "the youtube bind named no session"
        );
        Ok(())
    }

    /// Plays `video` from `at`, replacing whatever the screen played and its queue.
    pub async fn play(&self, video: &str, at: Duration) -> Result<()> {
        self.command(&[(
            "setPlaylist",
            vec![
                ("videoId", video.to_owned()),
                ("currentTime", format!("{:.3}", at.as_secs_f64())),
                ("currentIndex", "-1".to_owned()),
                ("listId", String::new()),
                ("audioOnly", "false".to_owned()),
            ],
        )])
        .await
    }

    /// Adds a video to the end of the screen's queue.
    pub async fn enqueue(&self, video: &str) -> Result<()> {
        self.command(&[("addVideo", vec![("videoId", video.to_owned())])])
            .await
    }

    /// Sends one of the commands that take no more than a value, such as `play`, `pause`,
    /// `next`, `seekTo` or `setVolume`.
    pub async fn send(&self, name: &str, arguments: Vec<(&str, String)>) -> Result<()> {
        self.command(&[(name, arguments)]).await
    }

    /// Sends `commands` in one request. A session the server forgot, as it does while the
    /// computer sleeps, is bound again and the request sent once more.
    async fn command(&self, commands: &[(&str, Vec<(&str, String)>)]) -> Result<()> {
        let status = self.request(commands).await?;
        if !FORGOTTEN.contains(&status) {
            return check(status);
        }
        log::info!("youtube: the screen session ran out, binding again");
        self.rebind(&mut Screen::default()).await?;
        check(self.request(commands).await?)
    }

    async fn request(&self, commands: &[(&str, Vec<(&str, String)>)]) -> Result<StatusCode> {
        let (query, token, ofs) = {
            let mut bound = self.bound();
            let ofs = bound.ofs;
            bound.ofs += commands.len() as u64;
            bound.rid += 1;
            let rid = bound.rid.to_string();
            (self.session(&bound, &rid), bound.token.clone(), ofs)
        };
        let mut body = vec![
            ("count".to_owned(), commands.len().to_string()),
            ("ofs".to_owned(), ofs.to_string()),
        ];
        for (at, (name, arguments)) in commands.iter().enumerate() {
            body.push((format!("req{at}__sc"), (*name).to_owned()));
            for (key, value) in arguments {
                body.push((format!("req{at}_{key}"), value.clone()));
            }
        }
        let response = self
            .signed(
                self.http
                    .post(format!("{}/api/lounge/bc/bind", self.origin())),
            )
            .query(&query)
            .header(TOKEN_HEADER, &token)
            .form(&body)
            .timeout(ANSWER)
            .send()
            .await
            .context("cannot reach the youtube screen")?;
        Ok(response.status())
    }

    /// Waits for what the screen reports until the poll ends, folding each report into
    /// `reported` and calling `changed` after each batch of events.
    pub async fn poll(
        &self,
        reported: &mut Screen,
        mut changed: impl FnMut(&Screen),
    ) -> Result<Ended> {
        let (mut query, token) = {
            let bound = self.bound();
            (self.session(&bound, "rpc"), bound.token.clone())
        };
        query.extend([("CI", "0".to_owned()), ("TYPE", "xmlhttp".to_owned())]);
        let mut response = self
            .signed(
                self.http
                    .get(format!("{}/api/lounge/bc/bind", self.origin())),
            )
            .query(&query)
            .header(TOKEN_HEADER, &token)
            .send()
            .await
            .context("cannot poll the youtube screen")?;
        let status = response.status();
        if FORGOTTEN.contains(&status) {
            return Ok(Ended::Rebind);
        }
        check(status)?;
        let mut chunks = Chunks::default();
        while let Some(bytes) = response
            .chunk()
            .await
            .context("the youtube screen poll broke")?
        {
            chunks.feed(&bytes);
            while let Some(events) = chunks.next() {
                if self.apply(events, reported) {
                    return Ok(Ended::Gone);
                }
                changed(reported);
            }
        }
        Ok(Ended::Done)
    }

    /// Asks YouTube for the token that lets a remote bind to the screen.
    async fn token(&self) -> Result<String> {
        let answer: Value = self
            .signed(self.http.post(format!(
                "{}/api/lounge/pairing/get_lounge_token_batch",
                self.origin()
            )))
            .form(&[("screen_ids", self.screen.as_str())])
            .timeout(ANSWER)
            .send()
            .await
            .context("cannot ask youtube for a lounge token")?
            .error_for_status()
            .context("youtube refused a lounge token")?
            .json()
            .await
            .context("cannot read the lounge token")?;
        answer["screens"][0]["loungeToken"]
            .as_str()
            .map(str::to_owned)
            .context("youtube named no lounge token for the screen")
    }

    fn origin(&self) -> &'static str {
        match self.account {
            Some(_) => SIGNED_IN,
            None => GUEST,
        }
    }

    /// Signs `request` in to the account, with its cookies as they stand now. A guest's
    /// request, or one whose cookies cannot be read, goes out as it is.
    fn signed(&self, request: RequestBuilder) -> RequestBuilder {
        let Some(account) = &self.account else {
            return request;
        };
        let Some(cookies) = cookies(account) else {
            log::warn!("youtube: cannot read the cookies to sign in to the screen with");
            return request;
        };
        let value = |name: &str| {
            cookies
                .split("; ")
                .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
                .unwrap_or("")
        };
        let (sapisid, third) = (value("SAPISID"), value("__Secure-3PAPISID"));
        let first = match value("__Secure-1PAPISID") {
            "" => sapisid,
            first => first,
        };
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or(0);
        let hash = |id: &str| {
            let digest = Sha1::digest(format!("{stamp} {id} {SIGNED_IN}"));
            let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            format!("{stamp}_{hex}")
        };
        let authorization = format!(
            "SAPISIDHASH {} SAPISID1PHASH {} SAPISID3PHASH {}",
            hash(sapisid),
            hash(first),
            hash(third)
        );
        // the web app's own token, made from the same two cookies the hash is
        let xsrf = URL_SAFE.encode(format!(
            ",{},{}",
            URL_SAFE.encode(sapisid),
            URL_SAFE.encode(third)
        ));
        request
            .header("Cookie", &cookies)
            .header("Origin", SIGNED_IN)
            .header("Referer", format!("{SIGNED_IN}/"))
            .header("X-Goog-AuthUser", account.authuser.to_string())
            .header("Authorization", authorization)
            .header("x-youtube-lounge-xsrf-token", xsrf)
    }

    /// The query every request inside the bound session carries.
    fn session(&self, bound: &Bound, rid: &str) -> Vec<(&'static str, String)> {
        let mut query = vec![
            ("name", self.name.clone()),
            ("loungeIdToken", bound.token.clone()),
            ("SID", bound.sid.clone()),
            ("gsessionid", bound.gsession.clone()),
            ("device", "REMOTE_CONTROL".to_owned()),
            ("app", "youtube-desktop".to_owned()),
            ("VER", "8".to_owned()),
            ("v", "2".to_owned()),
            ("RID", rid.to_owned()),
        ];
        if let Some(aid) = bound.aid {
            query.push(("AID", aid.to_string()));
        }
        query
    }

    /// Folds a batch of events into `reported`. Answers whether the screen left.
    fn apply(&self, events: Vec<Value>, reported: &mut Screen) -> bool {
        let mut gone = false;
        for event in events {
            let Some(event) = event.as_array() else {
                continue;
            };
            if let Some(id) = event.first().and_then(Value::as_i64) {
                self.bound().aid = Some(id);
            }
            let Some(body) = event.get(1).and_then(Value::as_array) else {
                continue;
            };
            let kind = body.first().and_then(Value::as_str).unwrap_or("");
            let argument = body.get(1).unwrap_or(&Value::Null);
            match kind {
                "c" => self.bound().sid = argument.as_str().unwrap_or("").to_owned(),
                "S" => self.bound().gsession = argument.as_str().unwrap_or("").to_owned(),
                "nowPlaying" => {
                    let video = argument["videoId"]
                        .as_str()
                        .filter(|video| !video.is_empty())
                        .map(str::to_owned);
                    // the clock belongs to the video the screen had before
                    if video != reported.video {
                        reported.position = Duration::ZERO;
                        reported.duration = Duration::ZERO;
                        reported.stamp = None;
                    }
                    reported.video = video;
                    reported.index = number(&argument["currentIndex"])
                        .filter(|index| *index >= 0.)
                        .map(|index| index as usize);
                    clock(reported, argument);
                    if reported.video.is_none() {
                        reported.playing = false;
                    }
                }
                "onStateChange" => clock(reported, argument),
                "onVolumeChanged" => {
                    let muted = argument["muted"] == "true";
                    reported.volume = number(&argument["volume"])
                        .map(|volume| if muted { 0. } else { volume as f32 / 100. });
                }
                "loungeScreenDisconnected" => gone = true,
                "loungeStatus" => {
                    let devices: Vec<Value> = argument["devices"]
                        .as_str()
                        .and_then(|devices| serde_json::from_str(devices).ok())
                        .unwrap_or_default();
                    for device in devices {
                        log::debug!(
                            "youtube: the screen has {} {:?}, signed in: {}",
                            device["type"].as_str().unwrap_or("?"),
                            device["name"].as_str().unwrap_or("?"),
                            device["user"].as_str().is_some_and(|user| !user.is_empty()),
                        );
                    }
                }
                _ => {}
            }
        }
        gone
    }
}

/// The `Cookie` header of the account's cookie store. Every cookie in it is YouTube's.
fn cookies(account: &Account) -> Option<String> {
    let Ok(stored) = std::fs::read(&account.cookies) else {
        return Some(account.signed_in.clone()).filter(|cookies| !cookies.is_empty());
    };
    let stored: Vec<Value> = serde_json::from_slice(&stored).ok()?;
    let pairs: Vec<&str> = stored
        .iter()
        .filter_map(|cookie| cookie["raw_cookie"].as_str()?.split(';').next())
        .collect();
    Some(pairs.join("; "))
}

fn check(status: StatusCode) -> Result<()> {
    anyhow::ensure!(status.is_success(), "the youtube screen answered {status}");
    Ok(())
}

/// Takes the position, length and play state from a report that carries them.
fn clock(reported: &mut Screen, argument: &Value) {
    // a time no duration can hold is left out rather than trusted
    if let Some(at) =
        number(&argument["currentTime"]).and_then(|at| Duration::try_from_secs_f64(at.max(0.)).ok())
    {
        reported.position = at;
        reported.stamp = Some(SystemTime::now());
    }
    if let Some(length) = number(&argument["duration"])
        .filter(|length| *length > 0.)
        .and_then(|length| Duration::try_from_secs_f64(length).ok())
    {
        reported.duration = length;
    }
    if let Some(state) = number(&argument["state"]) {
        let state = state as i64;
        reported.playing = matches!(state, PLAYING | STARTING | ADVERT | ADVERT_SKIPPED);
    }
    // a video the screen may not play still starts, and stays there
    match argument["playabilityStatus"].as_str() {
        Some("OK") => reported.refused = None,
        Some(status) => {
            log::info!("youtube: the screen cannot play its video: {status}");
            reported.playing = false;
            reported.refused = Some(Instant::now());
        }
        None => {}
    }
}

/// A number the lounge sends as a string, as it sends them all, or as a number.
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::String(text) => text.parse().ok(),
        value => value.as_f64(),
    }
}

/// The bind channel's framing: a line with a length, then a JSON array of events. The length
/// is not trusted, since it counts characters rather than bytes, so each array is read to its
/// own end instead.
#[derive(Default)]
struct Chunks {
    buffer: Vec<u8>,
}

impl Chunks {
    fn feed(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next whole batch of events, or `None` until more has arrived.
    fn next(&mut self) -> Option<Vec<Value>> {
        let start = self
            .buffer
            .iter()
            .position(|byte| !byte.is_ascii_whitespace())?;
        self.buffer.drain(..start);
        let newline = self.buffer.iter().position(|byte| *byte == b'\n')?;
        let mut values =
            serde_json::Deserializer::from_slice(&self.buffer[newline + 1..]).into_iter::<Value>();
        match values.next() {
            Some(Ok(value)) => {
                let used = newline + 1 + values.byte_offset();
                self.buffer.drain(..used);
                Some(match value {
                    Value::Array(events) => events,
                    _ => Vec::new(),
                })
            }
            Some(Err(error)) if error.is_eof() => None,
            None => None,
            Some(Err(error)) => {
                log::warn!("youtube: cannot read what the screen sent: {error}");
                self.buffer.clear();
                None
            }
        }
    }
}
