//! A native browser window with a throwaway session, for the two jobs that need a real engine.
//!
//! The window loads a page in a data store that lives only as long as the window. Once the proof
//! cookies appear, their header is handed back and the window closes. No browser profile ever
//! holds that session, so nothing rotates the cookies behind the app's back the way a shared
//! browser session does.
//!
//! The first job is signing in: the user works through the provider's pages and the proof cookies
//! are the session. The second is running a script on a page the app cannot reach any other way,
//! which is how YouTube's proof-of-origin token is minted; that window is hidden and its script
//! leaves the answer in a cookie the same poll reads.
//!
//! An account provider can finish the sign-in on an interstitial of its own — Google's security
//! check-up, say — that jumps straight to the return url and skips the hop that hands the account
//! to the provider's domain. The page then comes up signed out. When that happens, the sign-in url
//! is loaded once more: with the account already in, it only runs the hop that was skipped, which
//! is exactly what the page's own Sign in button would do.
//!
//! macOS, Windows and Linux have native backends. Every other platform reports
//! `supported() == false` and `Page::open` fails, so a caller falls back to pasting a header or
//! goes without a token.

use anyhow::Result;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
mod native;

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
use unsupported as platform;

/// What a window is asked to do. `url` opens first. `landing` scopes cookie reads on platforms
/// whose cookie store asks for a URL. The window is done as soon as the cookies for `domain`
/// carry at least one of the `proof` names.
///
/// Two kinds of window fit this. A sign-in window is shown and carries no script: the user works
/// through the provider's pages and the proof cookies are the session. A scripted window is
/// hidden and runs `script` on every page it loads, which does its work and leaves the answer in
/// a cookie named in `proof`; nothing about it is a sign-in, and the retry that recovers a
/// skipped hand-off is left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub url: String,
    pub landing: String,
    pub domain: String,
    pub proof: Vec<String>,
    pub title: String,
    /// A user agent the window presents instead of the backend's default, when the provider
    /// needs one that matches the engine the backend drives. A scripted window without one
    /// presents the engine's own agent.
    pub agent: Option<String>,
    /// Javascript to run in every page the window loads, before the page's own scripts. A window
    /// with one is a scripted window and is never shown.
    pub script: Option<String>,
}

impl Target {
    /// Whether this target is a background page rather than a window the user works in.
    pub(crate) fn scripted(&self) -> bool {
        self.script.is_some()
    }
}

/// One cookie as the page holds it. `domain` keeps the leading dot when the browser stored one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
}

/// Where a sign-in window is between two polls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    /// Still open, the user is not through yet.
    Pending,
    /// The user closed the window before signing in.
    Closed,
    /// The `Cookie` header for the target's domain. The window has closed itself.
    Cookies(String),
}

/// A browser window driving one [`Target`]. Open it on the main thread and poll it from there;
/// dropping it closes it.
pub struct Page {
    target: Target,
    window: platform::Window,
    /// Whether the sign-in url has already been loaded a second time. Once is the limit: a page
    /// that comes up signed out again is left to the user.
    retried: bool,
}

/// The argument that starts this executable as a throwaway check for webkit2gtk on Linux.
pub const PROBE: &str = "--webview-probe";

/// Whether this platform can open a browser window at all. On Linux the answer is only known once
/// a child process has looked for webkit2gtk, so ask it where a pause would not be felt.
pub fn supported() -> bool {
    platform::supported()
}

/// The exit code for a [`PROBE`] launch, zero when webkit2gtk loads, or none when this process was
/// started for anything else. Check it first thing in `main`.
pub fn probed() -> Option<i32> {
    #[cfg(target_os = "linux")]
    return platform::probed().map(|loads| i32::from(!loads));
    #[cfg(not(target_os = "linux"))]
    None
}

impl Page {
    /// Opens the window and starts loading the target url. Must run on the main thread.
    pub fn open(target: Target) -> Result<Self> {
        let window = platform::Window::open(&target)?;
        Ok(Self {
            target,
            window,
            retried: false,
        })
    }

    /// Reads the cookies of a scripted page and leaves it open, so one page can answer many times.
    /// Every read that finishes comes back as `Cookies` whatever it holds, and `Pending` means a
    /// read is still out. Call it every few hundred milliseconds or faster.
    pub fn peek(&mut self) -> Poll {
        if self.window.closed() {
            return Poll::Closed;
        }
        match self.window.fetch() {
            Some(cookies) => Poll::Cookies(header(&cookies, &self.target.domain)),
            None => Poll::Pending,
        }
    }

    /// Runs `script` in the page that is loaded now, in the same world as the target's own script.
    /// Nothing comes back, so a script that has an answer leaves it in a cookie for [`peek`]. A
    /// page that has not finished setting up yet drops the call.
    ///
    /// [`peek`]: Self::peek
    pub fn run(&self, script: &str) {
        self.window.run(script);
    }

    /// Checks where the window is. Call it every few hundred milliseconds until it stops answering
    /// `Pending`; the cookies come back once, and the window closes on that poll.
    pub fn poll(&mut self) -> Poll {
        if self.window.closed() {
            return Poll::Closed;
        }
        let Some(cookies) = self.window.fetch() else {
            return Poll::Pending;
        };
        let header = header(&cookies, &self.target.domain);
        match proven(&header, &self.target.proof) {
            true => {
                self.window.close();
                Poll::Cookies(header)
            }
            false => {
                self.retry();
                Poll::Pending
            }
        }
    }

    /// Loads the sign-in url again when the page is on the provider's domain without a session,
    /// once. Any page of the domain counts, not only the landing: a skipped hand-off can end on
    /// an error page of the provider's just as well. A scripted window has no hand-off to
    /// recover, and reloading would only throw away the work its script is in the middle of.
    fn retry(&mut self) {
        if self.retried || self.target.scripted() {
            return;
        }
        let Some(host) = self.window.host() else {
            return;
        };
        if !matches(&host, &self.target.domain) {
            return;
        }
        log::debug!("webview: {host} came up signed out, loading the sign-in url again");
        self.retried = true;
        self.window.load(&self.target.url);
    }
}

/// Joins the cookies a browser would send to `domain` into a `Cookie` header value.
fn header(cookies: &[Cookie], domain: &str) -> String {
    cookies
        .iter()
        .filter(|cookie| matches(&cookie.domain, domain))
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Whether a cookie stored for `stored` is sent to `domain` and its subdomains.
fn matches(stored: &str, domain: &str) -> bool {
    let stored = stored.trim_start_matches('.');
    stored == domain || stored.ends_with(&format!(".{domain}"))
}

/// Whether the header carries at least one of the proof cookies with a value. The name alone is
/// not enough: a signed-out Apple Music page already sets an empty `media-user-token`, and
/// taking that for a session closes the window before the user has typed anything.
fn proven(header: &str, proof: &[String]) -> bool {
    header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .any(|(name, value)| !value.trim().is_empty() && proof.iter().any(|wanted| wanted == name))
}

#[cfg(test)]
mod tests {
    use super::{Cookie, header, matches, proven};

    fn cookie(name: &str, domain: &str) -> Cookie {
        Cookie {
            name: name.to_string(),
            value: format!("{name}-value"),
            domain: domain.to_string(),
        }
    }

    #[test]
    fn keeps_the_domain_and_its_subdomains() {
        assert!(matches(".youtube.com", "youtube.com"));
        assert!(matches("youtube.com", "youtube.com"));
        assert!(matches("music.youtube.com", "youtube.com"));
        assert!(!matches(".google.com", "youtube.com"));
        assert!(!matches("notyoutube.com", "youtube.com"));
    }

    #[test]
    fn joins_only_the_matching_cookies() {
        let cookies = [
            cookie("SAPISID", ".youtube.com"),
            cookie("NID", ".google.com"),
            cookie("PREF", "music.youtube.com"),
        ];
        assert_eq!(
            header(&cookies, "youtube.com"),
            "SAPISID=SAPISID-value; PREF=PREF-value"
        );
    }

    #[test]
    fn proof_needs_one_of_the_names() {
        let proof = vec!["SAPISID".to_string(), "__Secure-3PAPISID".to_string()];
        assert!(proven("VISITOR=1; __Secure-3PAPISID=x", &proof));
        assert!(!proven("VISITOR=1; PREF=x", &proof));
        assert!(!proven("", &proof));
    }

    /// Apple Music sets the cookie it delivers the account in before anyone has signed in, with
    /// nothing in it.
    #[test]
    fn an_empty_proof_cookie_is_not_a_session() {
        let proof = vec!["media-user-token".to_string()];
        assert!(!proven("geo=PL; media-user-token=", &proof));
        assert!(!proven("media-user-token=   ", &proof));
        assert!(proven("media-user-token=AbCd", &proof));
    }
}
