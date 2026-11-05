//! Minting YouTube's proof-of-origin token in a hidden browser window.
//!
//! YouTube stops serving some streams unless the request carries a token that proves it came from
//! a browser. Google's BotGuard virtual machine produces one only after attesting the engine it is
//! running in, and no hand-written javascript environment passes that, so the app opens the one
//! real engine it ships: a `webview` page on youtube.com running `potoken.js`, off the screen,
//! which leaves each token in a cookie.
//!
//! Nothing here drives it on a schedule. `music::potoken` records the identifier the YouTube
//! client is waiting on, this entity notices, mints, and files the answer. The client uses a cold
//! start token until then, which covers the start of a track and no more. A host where webkit2gtk
//! does not load never opens a window and never mints.
//!
//! The YouTube client only asks for a track that neither tokenless client serves, and playback
//! preloads the next track, so the ask usually comes in while the previous one is still playing.
//! The page is the embed player, the lightest YouTube page that carries the minting client, at
//! about 270 MiB. A token outlives the page that minted it, so the page closes [`LINGER`] after
//! its last answer and only a burst of asks shares one.

use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Context, Task};

/// Where the minting page is loaded from: the embed player of the first video on YouTube, which
/// is never taken down. The first binding rides in the query string because the page has nothing
/// else to read it from before it loads.
const PAGE: &str = "https://www.youtube.com/embed/jNQXAC9IVRw";
const LANDING: &str = "www.youtube.com";
const DOMAIN: &str = "youtube.com";
/// The cookie the page leaves its answers in.
const COOKIE: &str = "SONORA_POT";
/// What the page prefixes a failure with. A token is base64url and never starts with one.
const FAILED: char = '!';
/// What ends the escaped binding at the start of an answer.
const SEPARATOR: char = ':';
/// How often the entity looks for an ask while no page is open.
const TICK: Duration = Duration::from_millis(500);
/// How often it ticks while a page is open. Most of what a warm mint costs a track is this.
const BUSY_TICK: Duration = Duration::from_millis(100);
/// How long a fresh page gets to attest and mint before it is given up on. Google's round trip
/// takes a second or two; a page still quiet after this is not going to answer.
const PATIENCE: Duration = Duration::from_secs(45);
/// How long an attested page gets to answer one more ask.
const ANSWER: Duration = Duration::from_secs(10);
/// How long a page stays open after its last answer, for a skip or two that ask again at once.
const LINGER: Duration = Duration::from_secs(20);

/// The browser window that mints proof-of-origin tokens, and the ask it is serving.
pub struct PoToken {
    open: Option<Open>,
    task: Option<Task<()>>,
}

/// A minting page that is open, and where it is with its asks.
struct Open {
    page: webview::Page,
    /// Whether the page has minted once, which means its minter is attested.
    warm: bool,
    /// The binding the page is minting for, and when it was asked.
    asked: Option<(String, Instant)>,
    /// When the page last answered.
    used: Instant,
}

/// What one look at an ask in flight found.
enum Outcome {
    Waiting,
    Minted(String),
    Failed,
}

impl PoToken {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            open: None,
            task: None,
        };
        this.watch(cx);
        this
    }

    /// Ticks for the life of the app: one tick starts a mint, carries one along, or does nothing.
    fn watch(&mut self, cx: &mut Context<Self>) {
        self.task = Some(cx.spawn(async move |this, cx| {
            let mut delay = TICK;
            loop {
                cx.background_executor().timer(delay).await;
                let Ok(next) = this.update(cx, |this, _| this.tick()) else {
                    return;
                };
                delay = next;
            }
        }));
    }

    /// Carries the entity one step and says how long to wait before the next.
    fn tick(&mut self) -> Duration {
        match &self.open {
            Some(open) if open.asked.is_some() => self.carry(),
            Some(_) => self.ask(),
            None => self.start(),
        }
        match self.open.is_some() {
            true => BUSY_TICK,
            false => TICK,
        }
    }

    /// Opens a page for whatever the client is waiting on, if anything.
    fn start(&mut self) {
        let Some(binding) = music::potoken::wanted() else {
            return;
        };
        if !webview::supported() {
            log::debug!(
                "potoken: no browser engine here, the stream goes out on a cold start token"
            );
            music::potoken::failed();
            return;
        }
        let target = webview::Target {
            url: format!("{PAGE}?binding={}", music::escape::component(&binding)),
            landing: LANDING.to_string(),
            domain: DOMAIN.to_string(),
            proof: vec![COOKIE.to_string()],
            title: String::new(),
            agent: None,
            script: Some(include_str!("potoken.js").to_string()),
        };
        match webview::Page::open(target) {
            Ok(page) => {
                log::debug!("potoken: minting for {binding}");
                let now = Instant::now();
                self.open = Some(Open {
                    page,
                    warm: false,
                    asked: Some((binding, now)),
                    used: now,
                });
            }
            Err(error) => {
                log::warn!("potoken: cannot open the minting page: {error:#}");
                music::potoken::failed();
            }
        }
    }

    /// Hands an attested page the next ask, or closes it once [`LINGER`] has passed without one.
    /// An ask that comes in after that waits for the next page.
    fn ask(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if open.used.elapsed() > LINGER {
            log::debug!("potoken: nothing more to mint, closing the minting page");
            self.close();
            return;
        }
        let Some(binding) = music::potoken::wanted() else {
            return;
        };
        let quoted = serde_json::to_string(&binding).unwrap_or_default();
        open.page.run(&format!(
            "window.sonoraPoToken && window.sonoraPoToken({quoted})"
        ));
        open.asked = Some((binding, Instant::now()));
    }

    /// Carries the ask in flight one poll further, and files whatever the page produced. A page
    /// that mints stays open for the next ask, and one that fails is closed.
    fn carry(&mut self) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let Some((binding, asked)) = open.asked.clone() else {
            return;
        };
        let patience = match open.warm {
            true => ANSWER,
            false => PATIENCE,
        };
        let outcome = match open.page.peek() {
            webview::Poll::Pending => Outcome::Waiting,
            webview::Poll::Closed => {
                log::warn!("potoken: the minting page closed before it answered");
                Outcome::Failed
            }
            webview::Poll::Cookies(header) => match reply(&header, &binding) {
                None => Outcome::Waiting,
                Some(answer) => match answer.strip_prefix(FAILED) {
                    Some(why) => {
                        log::warn!("potoken: the page could not mint a token: {why}");
                        Outcome::Failed
                    }
                    None => Outcome::Minted(answer.to_string()),
                },
            },
        };
        let outcome = match outcome {
            Outcome::Waiting if asked.elapsed() > patience => {
                log::warn!("potoken: the minting page did not answer in {patience:?}");
                Outcome::Failed
            }
            outcome => outcome,
        };
        match outcome {
            Outcome::Waiting => {}
            Outcome::Minted(token) => {
                log::debug!(
                    "potoken: minted a token for {binding} in {:?}",
                    asked.elapsed()
                );
                open.warm = true;
                open.asked = None;
                open.used = Instant::now();
                music::potoken::give(&binding, token);
                music::potoken::set_warm(true);
            }
            Outcome::Failed => {
                self.close();
                music::potoken::failed();
            }
        }
    }

    /// Closes the page, which gives back everything the engine held for it.
    fn close(&mut self) {
        self.open = None;
        music::potoken::set_warm(false);
    }
}

/// The page's answer for `binding` out of a `Cookie` header value, or none while the cookie still
/// holds the answer to an earlier ask.
fn reply<'a>(header: &'a str, binding: &str) -> Option<&'a str> {
    let prefix = format!("{}{SEPARATOR}", music::escape::component(binding));
    header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == COOKIE)
        .and_then(|(_, value)| value.strip_prefix(prefix.as_str()))
}

/// Starts the minting window. Nothing holds it but the global, and nothing reads it: the client
/// and the window meet in `music::potoken`.
pub fn attach(cx: &mut App) -> gpui::Entity<PoToken> {
    cx.new(PoToken::new)
}
