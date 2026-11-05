//! Links to a provider's pages read back into what they point at, and the one fetch a short
//! link needs before it can be read.

use std::sync::OnceLock;
use std::time::Duration;

use reqwest::Url;

/// How long a short link may take to answer before it is given up on.
const TIMEOUT: Duration = Duration::from_secs(10);
/// How many redirects a short link may take, the same as reqwest's default policy.
const REDIRECTS: usize = 10;
/// How much of a short link's page is read when its address alone does not say where it goes.
const PAGE_LIMIT: usize = 256 * 1024;
/// What ends an address written into a page.
const ADDRESS_END: [char; 7] = ['"', '\'', '<', '>', '\\', ' ', '`'];

/// What a link points at, in the ids its provider's client takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Linked {
    Track(String),
    Album(String),
    Artist(String),
    Playlist(String),
    User(String),
    /// A group session to join, kept as the whole link since the device network reads it.
    Jam(String),
    /// A short link, which says what it points at only once `follow` has fetched it.
    Short,
}

/// Where a short link led: the address it ended at and the start of the page there, since some
/// short links answer with a page that names the target instead of redirecting to it.
pub struct Followed {
    pub address: String,
    pub page: String,
}

/// Reads `link` as a web address or an address in a provider's own scheme. Whatever does not
/// parse is no link at all.
pub fn address(link: &str) -> Option<Url> {
    Url::parse(link.trim()).ok()
}

/// The host of a web address without its `www.` or `m.`, so a provider matches one name.
pub fn host(url: &Url) -> Option<&str> {
    let host = url.host_str()?;
    Some(
        host.strip_prefix("www.")
            .or_else(|| host.strip_prefix("m."))
            .unwrap_or(host),
    )
}

/// The non-empty parts of an address's path.
pub fn segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|parts| parts.filter(|part| !part.is_empty()).collect())
        .unwrap_or_default()
}

/// The value of `key` in an address's query, when it has a non-empty one.
pub fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// Fetches an https short link, following its redirects, and reads the start of the page it
/// ends at. Nothing but https is ever fetched, so a redirect to any other scheme fails the follow.
pub async fn follow(link: &str) -> Option<Followed> {
    let url = address(link).filter(|url| url.scheme() == "https")?;
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                .redirect(reqwest::redirect::Policy::custom(https_only))
                .build()
                .ok()
        })
        .as_ref()?;
    let mut response = client
        .get(url)
        .send()
        .await
        .inspect_err(|error| log::warn!("links: cannot follow a short link: {error}"))
        .ok()?;
    let address = response.url().to_string();
    let mut page = Vec::new();
    while page.len() < PAGE_LIMIT {
        match response.chunk().await {
            Ok(Some(chunk)) => page.extend_from_slice(&chunk),
            Ok(None) | Err(_) => break,
        }
    }
    Some(Followed {
        address,
        page: String::from_utf8_lossy(&page).into_owned(),
    })
}

/// Every https address written out in `page`, in the order they appear.
pub fn addresses(page: &str) -> impl Iterator<Item = &str> {
    page.match_indices("https://").map(|(at, _)| {
        let rest = &page[at..];
        let end = rest.find(ADDRESS_END).unwrap_or(rest.len());
        &rest[..end]
    })
}

/// Follows a redirect only to another https address and only within `REDIRECTS` hops.
fn https_only(attempt: reqwest::redirect::Attempt) -> reqwest::redirect::Action {
    if attempt.url().scheme() != "https" {
        attempt.error("redirect leaves https")
    } else if attempt.previous().len() > REDIRECTS {
        attempt.error("too many redirects")
    } else {
        attempt.follow()
    }
}
