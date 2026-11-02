//! Spotify tracks downloaded for offline play, kept the way Spotify's own clients keep theirs:
//! the file as the CDN sent it, still encrypted, with its key and metadata beside it. The vault
//! seals all of that again under the vault's master key, so a download plays outside Sonora
//! only for someone who holds that key, and for them it is as easy to play as any file.

use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use librespot_core::audio_key::{AudioKey, AudioKeyError};
use librespot_core::error::ErrorKind;
use librespot_core::{FileId, Session, SpotifyId, SpotifyUri};
use librespot_playback::config::Bitrate;
use librespot_playback::offline::{self, Download, Offline, OfflineTrack};
use librespot_protocol::metadata::Track as TrackMessage;
use protobuf::Message as _;
use serde::{Deserialize, Serialize};

use crate::Throttled;
use crate::spotify::playback::track_uri;
use crate::vault::Shelf;

/// What is kept beside a downloaded file, which is everything librespot asks the network for
/// when it plays one.
#[derive(Serialize, Deserialize)]
struct Facts {
    /// The track's `TRACK_V4` metadata, in base64.
    metadata: String,
    /// The file's format, as Spotify's protocol numbers it.
    format: i32,
    file_id: String,
    key: Option<[u8; 16]>,
}

/// The downloads librespot plays before it reaches for the network.
pub(crate) struct Kept(pub(crate) Shelf);

impl Offline for Kept {
    fn track(&self, track: SpotifyId) -> Option<OfflineTrack> {
        let (facts, file) = self.0.open_pinned(&track.to_base62())?;
        let facts = serde_json::from_slice::<Facts>(&facts)
            .inspect_err(|error| log::warn!("spotify: the kept facts do not read: {error}"))
            .ok()?;
        let metadata = STANDARD.decode(&facts.metadata).ok()?;
        let format = offline::AudioFileFormat::try_from(facts.format).ok()?;
        let file_id = FileId::from(hex(&facts.file_id)?.as_slice());
        let len = file.length();
        log::debug!("spotify: {} plays from its download", track.to_base62());
        Some(OfflineTrack {
            metadata,
            format,
            file_id,
            key: facts.key.map(AudioKey),
            file: Box::new(file),
            len,
        })
    }
}

/// Fetches all of track `id` at `bitrate` on a connected session and keeps it as a download.
pub(crate) async fn download(
    session: &Session,
    shelf: &Shelf,
    id: &str,
    bitrate: Bitrate,
) -> Result<()> {
    shelf.settle().await;
    if shelf.holds(id) {
        return Ok(());
    }
    let uri = track_uri(id)?;
    if let Err(error) = key_ready(session, &uri).await
        && refused_for_now(&error)
    {
        return Err(failure(error, id));
    }
    let fetched = match offline::download(session, uri.clone(), bitrate).await {
        Ok(fetched) => fetched,
        Err(error) if error.kind == ErrorKind::Unavailable && !refused_for_now(&error) => {
            match relinked(session, &uri, bitrate).await {
                Some(Ok(fetched)) => fetched,
                Some(Err(alternative)) => return Err(failure(alternative, id)),
                None => return Err(failure(error, id)),
            }
        }
        Err(error) => return Err(failure(error, id)),
    };
    let facts = serde_json::to_vec(&Facts {
        metadata: STANDARD.encode(&fetched.metadata),
        format: fetched.format.into(),
        file_id: fetched.file_id.to_base16(),
        key: fetched.key.map(|key| key.0),
    })?;
    let shelf = shelf.clone();
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || shelf.pin(&id, facts, &fetched.file))
        .await
        .context("the download stopped")?
}

/// Asks for one of the track's audio keys while the session is held to a pause, so a track that
/// would be refused costs a packet and not a whole file fetched first. Free while the session is
/// not throttled, and any failure but a refusal is left for the download itself to hit.
async fn key_ready(session: &Session, uri: &SpotifyUri) -> Result<(), librespot_core::Error> {
    if !session.audio_key().is_throttled() {
        return Ok(());
    }
    let Ok(track) = SpotifyId::try_from(uri) else {
        return Ok(());
    };
    let metadata = session.spclient().get_track_metadata(uri).await?;
    let Ok(message) = TrackMessage::parse_from_bytes(&metadata) else {
        return Ok(());
    };
    let Some(file) = message.file.iter().find(|file| !file.file_id().is_empty()) else {
        return Ok(());
    };
    session
        .audio_key()
        .request(track, FileId::from(file.file_id()))
        .await
        .map(|_| ())
}

/// Fetches a track Spotify relinked for this market through the first of its alternatives that
/// downloads, the way playback falls back to them. `None` when the track has files of its own or
/// no alternatives, so the failure of the track itself stands. The download is still kept under
/// the id the listener asked for, which is the one the player looks up.
async fn relinked(
    session: &Session,
    uri: &SpotifyUri,
    bitrate: Bitrate,
) -> Option<Result<Download, librespot_core::Error>> {
    let metadata = session.spclient().get_track_metadata(uri).await.ok()?;
    let track = TrackMessage::parse_from_bytes(&metadata).ok()?;
    if !track.file.is_empty() {
        return None;
    }
    let mut last = None;
    for alternative in &track.alternative {
        let Ok(id) = SpotifyId::from_raw(alternative.gid()) else {
            continue;
        };
        match offline::download(session, SpotifyUri::Track { id }, bitrate).await {
            Ok(fetched) => {
                log::debug!("spotify: {uri} downloads as its alternative {id}");
                return Some(Ok(fetched));
            }
            Err(error) => last = Some(Err(error)),
        }
    }
    last
}

/// Whether the audio key was refused for now in this very attempt, which is the one failure
/// worth waiting out. The session's sticky throttled flag is not asked, since it stays set until
/// a later key is served and would make any other failure read as throttling.
fn refused_for_now(error: &librespot_core::Error) -> bool {
    matches!(
        error.error.downcast_ref::<AudioKeyError>(),
        Some(AudioKeyError::Refused(_))
    )
}

/// A failed download as the queue sees it, marked [`Throttled`] when Spotify only asked it to
/// wait.
fn failure(error: librespot_core::Error, id: &str) -> anyhow::Error {
    match refused_for_now(&error) {
        true => anyhow::Error::new(error).context(Throttled),
        false => anyhow::Error::new(error).context(format!("cannot download {id}")),
    }
}

/// The bytes a run of hex digits spells, or `None` for anything else.
pub(crate) fn hex(digits: &str) -> Option<Vec<u8>> {
    (0..digits.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok())
        .collect()
}
