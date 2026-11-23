use anyhow::{Context as _, Result};

use crate::spotify::{auth::sign_in_error, collection, session::Session};
use spotifyapis::proto::connect::ContextTrack;

const RADIO_TRACKS: usize = 50;

/// Ordered context rows used to locate occurrences in Sonora's queue.
pub(in crate::spotify) struct Resolved {
    pub(in crate::spotify) tracks: Vec<ContextTrack>,
    pub(in crate::spotify) complete: bool,
}

pub(in crate::spotify) async fn resolve(session: &Session, uri: &str) -> Result<Resolved> {
    let resolved = session
        .api()
        .resolve_sequence(uri, session.device_id())
        .await
        .map_err(sign_in_error)?;
    Ok(Resolved {
        tracks: resolved.tracks,
        complete: resolved.complete,
    })
}

/// Resolves and hydrates a track station in Spotify's playback order.
pub(in crate::spotify) async fn track_radio(
    session: &Session,
    id: &str,
) -> Result<Vec<crate::Track>> {
    radio(session, "track", id).await
}

pub(in crate::spotify) fn radio_kind(seed: crate::RadioSeed) -> &'static str {
    match seed {
        crate::RadioSeed::Album => "album",
        crate::RadioSeed::Playlist => "playlist",
        crate::RadioSeed::Artist => "artist",
    }
}

/// Resolves and hydrates a station while preserving Spotify's order.
pub(in crate::spotify) async fn radio(
    session: &Session,
    kind: &str,
    id: &str,
) -> Result<Vec<crate::Track>> {
    let station = format!("spotify:station:{kind}:{id}");
    let uris: Vec<_> = resolve(session, &station)
        .await
        .context("cannot resolve Spotify radio")?
        .tracks
        .into_iter()
        .filter_map(|track| track.uri)
        .filter(|uri| uri.starts_with("spotify:track:"))
        .take(RADIO_TRACKS)
        .collect();
    collection::ordered_tracks(session, &uris, collection::TRACK_DETAILS).await
}
