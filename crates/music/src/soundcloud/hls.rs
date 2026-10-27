//! The HLS media playlist SoundCloud serves AAC as: one init segment, then media segments of
//! fragmented MP4, each a few seconds of audio. Parsed here into the urls [`Parts`] fetches.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};

/// One media segment of a playlist, with how long it runs in the media timeline.
#[derive(Clone, Debug)]
pub struct Segment {
    pub url: String,
    pub duration: Duration,
}

/// An init segment and the media segments after it, in order.
#[derive(Clone, Debug)]
pub struct Playlist {
    pub init: String,
    pub media: Vec<Segment>,
}

/// Fetches and parses the media playlist `url` points at. The playlist is small, so a wrong
/// answer can cost no more than this many bytes.
const PLAYLIST_CEILING: usize = 1 << 20;

/// Fetches a media playlist and reads its segments.
pub async fn open(http: &reqwest::Client, url: &str) -> Result<Playlist> {
    let response = http
        .get(url)
        .send()
        .await
        .context("cannot fetch the soundcloud playlist")?
        .error_for_status()
        .context("the soundcloud playlist was refused")?;
    let mut text = String::new();
    let mut body = response;
    while let Some(chunk) = body
        .chunk()
        .await
        .context("cannot read the soundcloud playlist")?
    {
        if text.len() + chunk.len() > PLAYLIST_CEILING {
            bail!("the soundcloud playlist is larger than {PLAYLIST_CEILING} bytes");
        }
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    let base = reqwest::Url::parse(url).context("the playlist url does not parse")?;
    parse(&text, &base)
}

/// Parses a media playlist, resolving relative segment urls against `base`.
pub fn parse(text: &str, base: &reqwest::Url) -> Result<Playlist> {
    let mut init: Option<String> = None;
    let mut media = Vec::new();
    let mut pending: Option<Duration> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let uri = rest
                .strip_prefix("URI=\"")
                .and_then(|rest| rest.split('"').next())
                .filter(|uri| !uri.is_empty())
                .context("the playlist's init segment names no URI")?;
            init = Some(join(base, uri)?);
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let seconds: f64 = rest
                .split(',')
                .next()
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0.0);
            pending = Some(Duration::from_secs_f64(seconds));
        } else if !line.starts_with('#')
            && let Some(duration) = pending.take()
        {
            media.push(Segment {
                url: join(base, line)?,
                duration,
            });
        }
    }
    let init = init.context("the playlist names no init segment")?;
    if media.is_empty() {
        bail!("the playlist lists no media segments");
    }
    Ok(Playlist { init, media })
}

/// Resolves a segment url against the playlist's own address, so an absolute url passes through
/// and a relative one joins it.
fn join(base: &reqwest::Url, uri: &str) -> Result<String> {
    let resolved = base
        .join(uri)
        .with_context(|| format!("cannot resolve the segment url {uri}"))?;
    Ok(resolved.to_string())
}
