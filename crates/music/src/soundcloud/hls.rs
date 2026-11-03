//! The HLS media playlist SoundCloud serves AAC as: one init segment, then media segments of
//! fragmented MP4, each a few seconds of audio. Parsed here into the urls [`Parts`] fetches.
//! A protected track's playlist also names its Widevine key, as a `pssh` box inline in the
//! `EXT-X-KEY` line, which is what the CDM's license challenge is built from.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

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
    /// The Widevine key of a protected playlist: its `pssh` box, and the key id inside it.
    pub key: Option<Key>,
}

/// What licensing a protected playlist takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// The whole `pssh` box, which the CDM takes as its init data.
    pub pssh: Vec<u8>,
    /// The content key the box names, which every sample is decrypted under.
    pub id: Vec<u8>,
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
    let mut key: Option<Key> = None;
    let mut encrypted = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let attributes = attributes(rest);
            let value = |name: &str| {
                attributes
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.as_str())
            };
            if value("METHOD").is_some_and(|method| method != "NONE") {
                encrypted = true;
            }
            let widevine = value("KEYFORMAT")
                .is_some_and(|format| format.eq_ignore_ascii_case(widevine::KEY_FORMAT));
            if widevine && key.is_none() {
                let uri = value("URI").context("the playlist's widevine key names no URI")?;
                key = Some(inline_key(uri)?);
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
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
    if encrypted && key.is_none() {
        bail!("the playlist is encrypted under a key system other than widevine");
    }
    Ok(Playlist { init, media, key })
}

/// The `pssh` box a `data:` key URI carries in base64, and the key id inside it.
fn inline_key(uri: &str) -> Result<Key> {
    let (_, encoded) = uri
        .strip_prefix("data:")
        .and_then(|rest| rest.split_once(";base64,"))
        .context("the playlist's widevine key is not inline")?;
    let pssh = STANDARD
        .decode(encoded.trim())
        .context("the playlist's widevine key is not base64")?;
    let id = key_id(&pssh).context("the playlist's widevine key names no key id")?;
    Ok(Key { pssh, id })
}

/// The first key id a Widevine `pssh` box names. A version 1 box lists key ids in its header;
/// a version 0 box only has them in the `WidevinePsshData` protobuf, as field 2.
pub fn key_id(pssh: &[u8]) -> Option<Vec<u8>> {
    if pssh.get(4..8)? != b"pssh" || pssh.get(12..28)? != widevine::SYSTEM_ID {
        return None;
    }
    let version = *pssh.get(8)?;
    let mut at = 28;
    if version >= 1 {
        let count = u32::from_be_bytes(pssh.get(at..at + 4)?.try_into().ok()?) as usize;
        at += 4;
        if count > 0 {
            return pssh.get(at..at + 16).map(<[u8]>::to_vec);
        }
    }
    let size = u32::from_be_bytes(pssh.get(at..at + 4)?.try_into().ok()?) as usize;
    at += 4;
    let data = pssh.get(at..at + size)?;
    protobuf_key_id(data)
}

/// Field 2 of a `WidevinePsshData` protobuf, walking past every field before it. Only the wire
/// types that message uses are read: varints and length-delimited bytes.
fn protobuf_key_id(mut data: &[u8]) -> Option<Vec<u8>> {
    fn varint(data: &mut &[u8]) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let (&byte, rest) = data.split_first()?;
            *data = rest;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    }
    while !data.is_empty() {
        let tag = varint(&mut data)?;
        match tag & 7 {
            0 => {
                varint(&mut data)?;
            }
            2 => {
                let len = varint(&mut data)? as usize;
                let field = data.get(..len)?;
                data = &data[len..];
                if tag >> 3 == 2 && !field.is_empty() {
                    return Some(field.to_vec());
                }
            }
            _ => return None,
        }
    }
    None
}

/// The `NAME=value` pairs of an HLS attribute list. A quoted value keeps its commas and loses
/// its quotes.
fn attributes(list: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = list.trim();
    while !rest.is_empty() {
        let Some((name, after)) = rest.split_once('=') else {
            break;
        };
        let (value, next) = match after.strip_prefix('"') {
            Some(quoted) => match quoted.split_once('"') {
                Some((value, tail)) => (value, tail.trim_start_matches(',')),
                None => (quoted, ""),
            },
            None => match after.split_once(',') {
                Some((value, tail)) => (value, tail),
                None => (after, ""),
            },
        };
        found.push((name.trim().to_owned(), value.to_owned()));
        rest = next.trim_start();
    }
    found
}

/// Resolves a segment url against the playlist's own address, so an absolute url passes through
/// and a relative one joins it.
fn join(base: &reqwest::Url, uri: &str) -> Result<String> {
    let resolved = base
        .join(uri)
        .with_context(|| format!("cannot resolve the segment url {uri}"))?;
    Ok(resolved.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_ID: [u8; 16] = [
        0x10, 0x32, 0x54, 0x76, 0x98, 0xba, 0xdc, 0xfe, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd,
        0xef,
    ];

    fn base() -> reqwest::Url {
        reqwest::Url::parse("https://cf-hls-media.sndcdn.com/playlist/abc/aac_160k.m3u8?Policy=p")
            .unwrap()
    }

    /// A protected playlist as SoundCloud lays one out: the FairPlay key beside the Widevine
    /// one, an init segment and two media segments.
    fn protected(pssh: &[u8]) -> String {
        format!(
            "#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:10\n\
             #EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://abc\",KEYFORMAT=\"com.apple.streamingkeydelivery\",KEYFORMATVERSIONS=\"1\"\n\
             #EXT-X-KEY:METHOD=SAMPLE-AES-CTR,URI=\"data:text/plain;base64,{}\",KEYFORMAT=\"{}\",KEYFORMATVERSIONS=\"1\"\n\
             #EXT-X-MAP:URI=\"init.mp4\"\n\
             #EXTINF:9.98,\nseg-0.m4s\n#EXTINF:4.5,\nseg-1.m4s\n#EXT-X-ENDLIST\n",
            STANDARD.encode(pssh),
            widevine::KEY_FORMAT,
        )
    }

    #[test]
    fn a_protected_playlist_names_its_widevine_key() {
        let pssh = widevine::pssh(&KEY_ID);
        let playlist = parse(&protected(&pssh), &base()).unwrap();

        let key = playlist.key.unwrap();
        assert_eq!(key.pssh, pssh);
        assert_eq!(key.id, KEY_ID);
        assert_eq!(
            playlist.init,
            "https://cf-hls-media.sndcdn.com/playlist/abc/init.mp4"
        );
        assert_eq!(playlist.media.len(), 2);
        assert_eq!(playlist.media[1].duration, Duration::from_secs_f64(4.5));
    }

    #[test]
    fn a_clear_playlist_has_no_key() {
        let text = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10.0,\nseg-0.m4s\n";
        assert!(parse(text, &base()).unwrap().key.is_none());
    }

    #[test]
    fn a_playlist_only_fairplay_opens_is_refused() {
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://abc\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n\
                    #EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:10.0,\nseg-0.m4s\n";
        let error = parse(text, &base()).unwrap_err();
        assert!(error.to_string().contains("other than widevine"));
    }

    #[test]
    fn reads_the_key_id_of_a_version_one_box() {
        let mut pssh = Vec::new();
        pssh.extend_from_slice(&[0, 0, 0, 0]);
        pssh.extend_from_slice(b"pssh");
        pssh.extend_from_slice(&[1, 0, 0, 0]);
        pssh.extend_from_slice(&widevine::SYSTEM_ID);
        pssh.extend_from_slice(&1u32.to_be_bytes());
        pssh.extend_from_slice(&KEY_ID);
        pssh.extend_from_slice(&0u32.to_be_bytes());
        let total = pssh.len() as u32;
        pssh[..4].copy_from_slice(&total.to_be_bytes());
        assert_eq!(key_id(&pssh).unwrap(), KEY_ID);
    }

    #[test]
    fn a_box_of_another_system_names_no_key() {
        let mut pssh = widevine::pssh(&KEY_ID);
        pssh[12] ^= 0xff;
        assert!(key_id(&pssh).is_none());
    }

    #[test]
    fn a_quoted_attribute_keeps_its_commas() {
        let found = attributes(r#"METHOD=SAMPLE-AES-CTR,URI="data:a,b",KEYFORMATVERSIONS="1""#);
        assert_eq!(
            found,
            [
                ("METHOD".to_owned(), "SAMPLE-AES-CTR".to_owned()),
                ("URI".to_owned(), "data:a,b".to_owned()),
                ("KEYFORMATVERSIONS".to_owned(), "1".to_owned()),
            ]
        );
    }
}
