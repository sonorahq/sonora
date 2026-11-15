use std::borrow::Cow;
use std::sync::LazyLock;

/// What a hidden query, fragment or user info reads as.
const HIDDEN: &str = "<redacted>";
/// What a hidden user name reads as.
const OWNER: &str = "<user>";
/// The characters that end a url written into running text, besides whitespace.
const URL_END: &[char] = &[')', '"', '\'', '<', '>', '`', ']'];
/// What a user name follows: Spotify uris and the paths of several provider apis.
const OWNERS: &[&str] = &[":user:", "/user/", "/users/"];

/// The home folder as text, which usually holds the account name. A path as short as `/` is
/// left out, since replacing it would mangle every path.
static HOME: LazyLock<Option<String>> = LazyLock::new(|| {
    dirs::home_dir()
        .map(|home| {
            home.to_string_lossy()
                .trim_end_matches(['/', '\\'])
                .to_string()
        })
        .filter(|home| home.len() > 1)
});

/// `text` without what would sign the user in or name them, for a log line or an error shown on
/// screen. Every url loses its user info, query and fragment, the name after `user:` or `/user/`
/// is hidden, and the home folder reads as `~`. Borrows `text` when nothing had to go.
pub fn redact(text: &str) -> Cow<'_, str> {
    let home = HOME.as_deref().filter(|home| text.contains(home));
    let urls = text.contains("://");
    let owners = OWNERS.iter().any(|owner| text.contains(owner));
    if home.is_none() && !urls && !owners {
        return Cow::Borrowed(text);
    }

    let mut clean = text.to_string();
    if let Some(home) = home {
        clean = clean.replace(home, "~");
    }
    if urls {
        clean = hide_queries(&clean);
    }
    if owners {
        clean = hide_owners(&clean);
    }
    match clean == text {
        true => Cow::Borrowed(text),
        false => Cow::Owned(clean),
    }
}

/// Hides the user info, query and fragment of every url in `text` and keeps its scheme, host and
/// path. A token rides in one of those three in every provider Sonora talks to.
fn hide_queries(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, url) = rest.split_at(at + "://".len());
        out.push_str(before);
        let end = url
            .find(|c: char| c.is_whitespace() || URL_END.contains(&c))
            .unwrap_or(url.len());
        let (url, after) = url.split_at(end);
        let path = url.find(['/', '?', '#']).unwrap_or(url.len());
        let (authority, tail) = url.split_at(path);
        match authority.rsplit_once('@') {
            Some((_, host)) => {
                out.push_str(HIDDEN);
                out.push('@');
                out.push_str(host);
            }
            None => out.push_str(authority),
        }
        match tail.find(['?', '#']) {
            Some(cut) if cut + 1 < tail.len() => {
                out.push_str(&tail[..=cut]);
                out.push_str(HIDDEN);
            }
            _ => out.push_str(tail),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Hides the segment after each of [`OWNERS`] in `text`.
fn hide_owners(text: &str) -> String {
    let mut text = text.to_string();
    for owner in OWNERS {
        let mut out = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.find(owner) {
            let (before, name) = rest.split_at(at + owner.len());
            out.push_str(before);
            let end = name
                .find(|c: char| {
                    matches!(c, ':' | '/' | '?' | '#' | ',')
                        || c.is_whitespace()
                        || URL_END.contains(&c)
                })
                .unwrap_or(name.len());
            if end > 0 {
                out.push_str(OWNER);
            }
            rest = &name[end..];
        }
        out.push_str(rest);
        text = out;
    }
    text
}
