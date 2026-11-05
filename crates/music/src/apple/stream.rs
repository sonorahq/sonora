//! How an Apple Music track becomes bytes: the web playback lookup, the media playlist behind
//! it, the license round trip, and the response that carries the encrypted fMP4.
//!
//! Nothing here knows about audio output. It hands back a [`Media`] that reads as an ordinary
//! clear MP4, and the engine is what turns that into sound.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::apple::auth::{self, AGENT};
use crate::apple::progressive::Media;
use crate::vault::{Recording, Shelf};
use crate::{Encoding, Offer, Quality};
use widevine::{self, Cdm};

/// Where the web player asks what it may play.
const PLAYBACK: &str = "https://play.music.apple.com/WebObjects/MZPlay.woa/wa/webPlayback";

/// Where it exchanges a challenge for a license.
const LICENSE: &str =
    "https://play.itunes.apple.com/WebObjects/MZPlay.woa/wa/acquireWebPlaybackLicense";

/// The flavours a Widevine CDM can open, AAC at 256 kbps and HE-AAC at 64 kbps. The `cbcp` and
/// `ibhp` flavours are delivered under Apple's own `skd://` key system, which it cannot speak.
const FLAVORS: [&str; 2] = ["28:ctrp256", "32:ctrp64"];

/// The levels Apple Music serves Sonora, best first, one for each of `FLAVORS`.
pub(crate) const QUALITIES: &[Offer] = &[
    Offer {
        quality: Quality::High,
        encoding: Encoding::Aac { kbps: 256 },
        up_to: false,
    },
    Offer {
        quality: Quality::Low,
        encoding: Encoding::Aac { kbps: 64 },
        up_to: false,
    },
];

/// How long a playlist or license body may be. Both are small; this only bounds what a wrong
/// answer can cost.
const TEXT_CEILING: usize = 1 << 20;

/// A track resolved down to what it takes to read it: where the media is, and which key opens
/// it.
#[derive(Clone, Debug)]
pub struct Resolved {
    /// The fMP4 the media playlist maps to.
    pub asset: String,
    /// The `URI` of the Widevine key, passed back to the license endpoint verbatim.
    pub key_uri: String,
    /// The key id, decoded from the tail of `key_uri`.
    pub key_id: Vec<u8>,
    pub duration: Option<Duration>,
    /// The asset is the HE-AAC one, whose priming runs longer.
    pub extended: bool,
}

/// A track ready to decode: decrypting as it is read, with its first seconds already in.
/// Clones share the one download and license, so a preloaded track can be both queued behind
/// the current one and kept for the load that follows it.
#[derive(Clone)]
pub struct Loaded {
    pub media: Media,
    pub duration: Option<Duration>,
    /// The asset is the HE-AAC one, whose priming runs longer.
    pub extended: bool,
}

/// What a kept track needs to be licensed again without the playback lookup. The key id is
/// no secret, since the file's own `pssh` carries it.
#[derive(Serialize, Deserialize)]
struct Facts {
    key_uri: String,
    key_id: Vec<u8>,
    duration_ms: Option<u64>,
    extended: bool,
}

/// Resolves a catalog track, licenses it, and hands back a reader over its cleartext.
///
/// The body and the license are fetched at the same time: the bytes do not depend on the
/// license, and the CDM is only needed to read what has already arrived, so the round trip
/// costs nothing that the download was not spending anyway.
///
/// With a shelf, the encrypted file is kept as it arrives. A kept track skips the lookup and
/// the download but is licensed afresh on every play, so it plays only while Apple still
/// hands this account its key.
pub async fn prepare(
    http: &reqwest::Client,
    adam_id: &str,
    user_token: &str,
    quality: Quality,
    shelf: Option<&Shelf>,
) -> Result<Loaded> {
    widevine::require()?;
    let bearer = auth::bearer(http).await?;
    let variant = quality.within(QUALITIES).id();
    let recalled = match shelf {
        Some(shelf) => recall(shelf, adam_id, variant).await,
        None => None,
    };
    let (mut media, resolved) = match recalled {
        Some(recalled) => recalled,
        None => {
            log::info!("apple: resolving track {adam_id}");
            let resolved = resolve(http, adam_id, &bearer, user_token, quality).await?;
            let response = open_asset(http, &resolved.asset, user_token).await?;
            let total = response.content_length();
            let source = match shelf {
                Some(shelf) => {
                    let facts = Facts {
                        key_uri: resolved.key_uri.clone(),
                        key_id: resolved.key_id.clone(),
                        duration_ms: resolved
                            .duration
                            .map(|duration| duration.as_millis() as u64),
                        extended: resolved.extended,
                    };
                    let facts = move || serde_json::to_vec(&facts).ok();
                    shelf.keep(adam_id, variant, facts, response)
                }
                None => Recording::through(response),
            };
            (Media::pulling(source, total), resolved)
        }
    };

    log::info!("apple: opening widevine");
    let init = widevine::pssh(&resolved.key_id);
    // One exchange at a time, held until the license is in. Preloading the next track while
    // this one licenses is the ordinary case, and the CDM cannot have two challenges open.
    let licensing = widevine::licensing().await;
    // The lease is held across the license request, so the challenge and its license reach
    // the same CDM host.
    let (cdm, challenge) = tokio::task::spawn_blocking(move || -> Result<(Cdm, Vec<u8>)> {
        let cdm = Cdm::open()?;
        let challenge = cdm.challenge(&init)?;
        Ok((cdm, challenge))
    })
    .await
    .context("the widevine challenge task failed")??;

    log::info!("apple: requesting license");
    let license = license(http, &challenge, &resolved, adam_id, &bearer, user_token).await?;
    let cdm = tokio::task::spawn_blocking(move || -> Result<Cdm> {
        cdm.accept(&license)?;
        Ok(cdm)
    })
    .await
    .context("the widevine license task failed")??;
    drop(licensing);
    log::info!("apple: license accepted");

    media.license(cdm, resolved.key_id.clone())?;
    media.release_when_downloaded();
    media.prime().await?;
    Ok(Loaded {
        media,
        duration: resolved.duration,
        extended: resolved.extended,
    })
}

/// The kept encrypted file of a track and what licensing it takes, when the vault has it.
async fn recall(shelf: &Shelf, adam_id: &str, variant: &str) -> Option<(Media, Resolved)> {
    let kept = shelf.find(adam_id, variant).await?;
    let facts = serde_json::from_slice::<Facts>(&kept.facts)
        .inspect_err(|error| log::warn!("apple: the kept facts of {adam_id} do not read: {error}"))
        .ok()?;
    log::info!("apple: {adam_id} comes from the vault and is licensed again");
    let resolved = Resolved {
        asset: String::new(),
        key_uri: facts.key_uri,
        key_id: facts.key_id,
        duration: facts.duration_ms.map(Duration::from_millis),
        extended: facts.extended,
    };
    Some((Media::pulling(kept.source, Some(kept.length)), resolved))
}

/// Asks web playback for the track and reads the media playlist of its asset at `quality`.
pub async fn resolve(
    http: &reqwest::Client,
    adam_id: &str,
    bearer: &str,
    user_token: &str,
    quality: Quality,
) -> Result<Resolved> {
    let body = serde_json::json!({ "salableAdamId": adam_id });
    let response = http
        .post(PLAYBACK)
        .header(reqwest::header::USER_AGENT, AGENT)
        .header(reqwest::header::ORIGIN, "https://music.apple.com")
        .header(reqwest::header::REFERER, "https://music.apple.com/")
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header("x-apple-music-user-token", user_token)
        .json(&body)
        .send()
        .await
        .context("cannot reach apple web playback")?;
    let status = response.status();
    let answered: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("cannot read the web playback answer (http {status})"))?;
    if !status.is_success() {
        bail!("apple web playback refused the track: http {status}");
    }
    if let Some(reason) = answered
        .get("failureType")
        .and_then(serde_json::Value::as_str)
        .filter(|reason| !reason.is_empty())
    {
        bail!("apple web playback refused the track: {reason}");
    }

    let song = answered
        .get("songList")
        .and_then(serde_json::Value::as_array)
        .and_then(|songs| songs.first())
        .context("apple web playback listed no song")?;
    let (flavor, asset) = select(song, quality)?;
    log::info!("apple: selected {flavor}");

    let playlist = read_playlist(http, &asset).await?;
    let (file, key_uri) =
        parse_playlist(&playlist, &asset).context("the media playlist carries no widevine key")?;
    let encoded = key_uri
        .rsplit_once(',')
        .map(|(_, kid)| kid)
        .context("the widevine key uri names no key id")?;
    let key_id = STANDARD
        .decode(encoded)
        .context("the widevine key id is not base64")?;
    if key_id.len() != 16 {
        log::warn!("apple: the key id is {} bytes rather than 16", key_id.len());
    }

    Ok(Resolved {
        asset: file,
        key_uri,
        key_id,
        duration: duration(song),
        extended: flavor == FLAVORS[1],
    })
}

/// The flavour and url of the asset a song entry is played from at `quality`. A song without
/// that flavour plays the other one Widevine can open.
fn select(song: &serde_json::Value, quality: Quality) -> Result<(&'static str, String)> {
    if song
        .get("hls-playlist-url")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|url| !url.is_empty())
    {
        bail!("the track is served as an hls playlist, which this path cannot read");
    }
    let assets = song
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .context("the song carries no assets")?;
    for asset in assets {
        log::debug!(
            "apple: asset flavor {}",
            asset
                .get("flavor")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("none")
        );
    }
    let url = |flavor: &str| {
        assets
            .iter()
            .find(|asset| asset.get("flavor").and_then(serde_json::Value::as_str) == Some(flavor))
            .and_then(|asset| asset.get("URL").and_then(serde_json::Value::as_str))
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
    };
    let wanted = match quality.within(QUALITIES) {
        Quality::Low | Quality::Normal => FLAVORS[1],
        Quality::High | Quality::Lossless | Quality::HiRes => FLAVORS[0],
    };
    std::iter::once(wanted)
        .chain(FLAVORS.into_iter().filter(|flavor| *flavor != wanted))
        .find_map(|flavor| url(flavor).map(|url| (flavor, url)))
        .with_context(|| format!("the track has none of the {FLAVORS:?} assets"))
}

/// The track length, wherever this answer happens to carry it. Apple has moved it, and a
/// length is a nicety here: the catalog metadata already has one.
fn duration(song: &serde_json::Value) -> Option<Duration> {
    let millis = [
        song.pointer("/assets/0/metadata/durationInMillis"),
        song.pointer("/metadata/durationInMillis"),
        song.get("durationInMillis"),
    ]
    .into_iter()
    .flatten()
    .find_map(serde_json::Value::as_u64)?;
    Some(Duration::from_millis(millis))
}

async fn read_playlist(http: &reqwest::Client, url: &str) -> Result<String> {
    let response = http
        .get(url)
        .header(reqwest::header::USER_AGENT, AGENT)
        .send()
        .await
        .context("cannot fetch the media playlist")?
        .error_for_status()
        .context("apple refused the media playlist")?;
    if response
        .content_length()
        .is_some_and(|len| len as usize > TEXT_CEILING)
    {
        bail!("the media playlist is implausibly long");
    }
    let text = response
        .text()
        .await
        .context("cannot read the media playlist")?;
    match text.len() > TEXT_CEILING {
        true => bail!("the media playlist is implausibly long"),
        false => Ok(text),
    }
}

/// The `METHOD` Apple gives the CENC key on a `ctrp` playlist. It carries no `KEYFORMAT` at
/// all; the key system is decided by what the license endpoint is asked for.
const CENC_METHOD: &str = "ISO-23001-7";

/// Pulls the fMP4 url and the CENC key uri out of a media playlist.
///
/// A playlist can carry more than one `EXT-X-KEY`, so the right one is picked by what it is
/// rather than by being first: either the Widevine `KEYFORMAT`, or Apple's own CENC method. A
/// FairPlay key is passed over, since `skd://` delivery is not something a Widevine CDM can
/// answer. The `URI` holds a comma, which is why attributes are split with the quoting rules
/// and not on commas alone.
fn parse_playlist(playlist: &str, base: &str) -> Option<(String, String)> {
    let mut key = None;
    let mut map = None;
    for line in playlist.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            // The key lines name a key id and a key system, never a credential, and which ones
            // an asset carries is the first thing to look at when a track will not open.
            log::debug!("apple: playlist key {rest}");
            let attributes = attributes(rest);
            let value = |wanted: &str| {
                attributes
                    .iter()
                    .find(|(name, _)| name == wanted)
                    .map(|(_, value)| value.as_str())
            };
            let cenc = value("KEYFORMAT") == Some(widevine::KEY_FORMAT)
                || value("METHOD") == Some(CENC_METHOD);
            if let Some(uri) = value("URI").filter(|uri| cenc && !uri.starts_with("skd://")) {
                key = Some(uri.to_owned());
            }
        }
        if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            map = attributes(rest)
                .into_iter()
                .find(|(name, _)| name == "URI")
                .map(|(_, value)| value);
        }
    }
    Some((absolute(&map?, base), key?))
}

/// The `NAME=VALUE` pairs of an HLS tag, with quoted values unwrapped. A comma inside quotes
/// belongs to the value.
fn attributes(line: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let Some((name, tail)) = rest.split_once('=') else {
            break;
        };
        let (value, tail) = match tail.strip_prefix('"') {
            Some(quoted) => match quoted.split_once('"') {
                Some((value, tail)) => (value, tail.strip_prefix(',').unwrap_or(tail)),
                None => (quoted, ""),
            },
            None => match tail.split_once(',') {
                Some((value, tail)) => (value, tail),
                None => (tail, ""),
            },
        };
        found.push((name.trim().to_owned(), value.to_owned()));
        rest = tail;
    }
    found
}

/// Resolves a playlist-relative url against the playlist's own.
fn absolute(url: &str, base: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_owned();
    }
    match base.rsplit_once('/') {
        Some((root, _)) => format!("{root}/{url}"),
        None => url.to_owned(),
    }
}

/// Exchanges the CDM's challenge for a license. Nothing about the license is logged: it is the
/// wrapped content key.
async fn license(
    http: &reqwest::Client,
    challenge: &[u8],
    resolved: &Resolved,
    adam_id: &str,
    bearer: &str,
    user_token: &str,
) -> Result<Vec<u8>> {
    let envelope = serde_json::json!({
        "challenge": STANDARD.encode(challenge),
        "key-system": "com.widevine.alpha",
        "uri": resolved.key_uri,
        "adamId": adam_id,
        "isLibrary": false,
        "user-initiated": true,
    });
    let response = http
        .post(LICENSE)
        .header(reqwest::header::USER_AGENT, AGENT)
        .header(reqwest::header::ORIGIN, "https://music.apple.com")
        .header(reqwest::header::REFERER, "https://music.apple.com/")
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {bearer}"))
        .header("x-apple-music-user-token", user_token)
        .json(&envelope)
        .send()
        .await
        .context("cannot reach the apple license endpoint")?;
    let status = response.status();
    let answered: serde_json::Value = response
        .json()
        .await
        .with_context(|| format!("cannot read the license answer (http {status})"))?;
    if !status.is_success() {
        bail!("apple refused the license: http {status}");
    }
    // Apple answers 200 with the refusal in the body.
    if let Some(code) = answered
        .get("errorCode")
        .and_then(serde_json::Value::as_i64)
        .filter(|code| *code != 0)
    {
        bail!("apple refused the license: error code {code}");
    }
    let encoded = answered
        .get("license")
        .and_then(serde_json::Value::as_str)
        .context("the license answer carries no license")?;
    STANDARD
        .decode(encoded)
        .context("the license is not base64")
}

/// Starts the asset request and reads its headers.
async fn open_asset(
    http: &reqwest::Client,
    url: &str,
    user_token: &str,
) -> Result<reqwest::Response> {
    http.get(url)
        .header(reqwest::header::USER_AGENT, AGENT)
        .header("x-apple-music-user-token", user_token)
        .send()
        .await
        .context("cannot fetch the encrypted track")?
        .error_for_status()
        .context("apple refused the encrypted track")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a `28:ctrp256` playlist actually looks like: one CENC key, named by its method and
    /// carrying the key id in a `data:` uri, with no `KEYFORMAT` at all.
    const PLAYLIST: &str = r#"#EXTM3U
#EXT-X-VERSION:6
#EXT-X-TARGETDURATION:6
#EXT-X-MAP:URI="audio.mp4"
#EXT-X-KEY:METHOD=ISO-23001-7,URI="data:;base64,AAAAABozqhkAHa+aTGXMNg=="
#EXTINF:6.00000,
#EXT-X-BYTERANGE:12345@0
audio.mp4
#EXT-X-ENDLIST
"#;

    /// And what a `cbcp` one looks like: FairPlay, which a Widevine CDM cannot answer.
    const FAIRPLAY: &str = r#"#EXTM3U
#EXT-X-MAP:URI="audio.mp4"
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://itunes.apple.com/P000000000/s1/e1",KEYFORMAT="com.apple.streamingkeydelivery",KEYFORMATVERSIONS="1"
#EXT-X-ENDLIST
"#;

    #[test]
    fn takes_the_cenc_key_and_the_file_it_maps_to() {
        let (file, key) =
            parse_playlist(PLAYLIST, "https://aod.itunes.apple.com/x/y/playlist.m3u8").unwrap();
        assert_eq!(file, "https://aod.itunes.apple.com/x/y/audio.mp4");
        assert_eq!(key, "data:;base64,AAAAABozqhkAHa+aTGXMNg==");
    }

    #[test]
    fn takes_a_key_the_widevine_keyformat_names() {
        let named = PLAYLIST.replace(
            "METHOD=ISO-23001-7,",
            r#"METHOD=SAMPLE-AES-CTR,KEYFORMAT="urn:uuid:edef8ba9-79d6-4ace-a3c8-27dcd51d21ed","#,
        );
        let (_, key) = parse_playlist(&named, "https://x/y/playlist.m3u8").unwrap();
        assert_eq!(key, "data:;base64,AAAAABozqhkAHa+aTGXMNg==");
    }

    #[test]
    fn a_fairplay_playlist_is_refused() {
        assert_eq!(parse_playlist(FAIRPLAY, "https://x/y/playlist.m3u8"), None);
    }

    #[test]
    fn a_quoted_value_keeps_its_commas() {
        let read = attributes(r#"METHOD=SAMPLE-AES-CTR,URI="data:text/plain;base64,AA==",N=1"#);
        assert_eq!(
            read,
            vec![
                ("METHOD".to_owned(), "SAMPLE-AES-CTR".to_owned()),
                ("URI".to_owned(), "data:text/plain;base64,AA==".to_owned()),
                ("N".to_owned(), "1".to_owned()),
            ]
        );
    }

    #[test]
    fn an_absolute_map_uri_is_left_alone() {
        assert_eq!(
            absolute("https://other/audio.mp4", "https://x/y/playlist.m3u8"),
            "https://other/audio.mp4"
        );
        assert_eq!(
            absolute("audio.mp4", "https://x/y/playlist.m3u8"),
            "https://x/y/audio.mp4"
        );
    }

    #[test]
    fn picks_the_ctrp256_asset() {
        let song = serde_json::json!({
            "assets": [
                { "flavor": "32:cbcp64", "URL": "https://x/cbcp.m3u8" },
                { "flavor": FLAVORS[0], "URL": "https://x/ctr.m3u8" },
            ]
        });
        assert_eq!(
            select(&song, Quality::High).unwrap(),
            (FLAVORS[0], "https://x/ctr.m3u8".to_owned())
        );
    }

    #[test]
    fn a_song_with_no_ctr_asset_is_refused() {
        let song = serde_json::json!({
            "assets": [{ "flavor": "32:cbcp64", "URL": "https://x/cbcp.m3u8" }]
        });
        assert!(select(&song, Quality::High).is_err());
        assert!(select(&serde_json::json!({}), Quality::High).is_err());
    }

    #[test]
    fn reads_the_length_wherever_the_answer_carries_it() {
        let song =
            serde_json::json!({ "assets": [{ "metadata": { "durationInMillis": 12_000 } }] });
        assert_eq!(duration(&song), Some(Duration::from_secs(12)));
        assert_eq!(duration(&serde_json::json!({})), None);
    }
}
