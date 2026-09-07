use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::spotify::SpotifyProvider;
use crate::youtube::YouTubeProvider;
use crate::{MusicApi, MusicProvider, PlaylistEntry, ProviderSession};

const NAME: &str = "Sonora live privacy test — safe to delete";
const VERIFY_ATTEMPTS: usize = 30;
const LIBRARY_LIMIT: u32 = 10_000;

#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected Spotify account"]
async fn spotify_can_make_a_playlist_public_and_private() -> Result<()> {
    let provider = SpotifyProvider::from_env();
    let session = connected(&provider).await?;

    exercise_playlist_privacy(session.api.as_ref()).await
}

#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected YouTube Music account"]
async fn youtube_can_make_a_playlist_public_and_private() -> Result<()> {
    let provider = YouTubeProvider::new();
    let session = connected(&provider).await?;

    exercise_playlist_privacy(session.api.as_ref()).await
}

async fn connected(provider: &dyn MusicProvider) -> Result<ProviderSession> {
    let session = provider
        .restore()
        .await?
        .with_context(|| format!("{} has no stored Sonora session", provider.name()))?;
    if !session.authenticated {
        bail!(
            "{} restored a guest session, not an account",
            provider.name()
        );
    }
    Ok(session)
}

async fn exercise_playlist_privacy(api: &dyn MusicApi) -> Result<()> {
    let playlist_id = api.create_playlist(NAME).await?;

    let exercise = async {
        for wanted in [true, false, true] {
            api.set_playlist_public(&playlist_id, wanted).await?;
            wait_until_public(api, &playlist_id, wanted).await?;
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let cleanup = api.delete_playlist(&playlist_id).await;
    match (exercise, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.context("privacy cycle failed; playlist deleted")),
        (Ok(()), Err(error)) => Err(error.context("privacy cycle passed but cleanup failed")),
        (Err(exercise), Err(cleanup)) => Err(anyhow!(
            "privacy cycle failed: {exercise:#}; deleting the playlist also failed: {cleanup:#}"
        )),
    }
}

async fn wait_until_public(api: &dyn MusicApi, playlist_id: &str, expected: bool) -> Result<()> {
    let mut observed = None;
    for _ in 0..VERIFY_ATTEMPTS {
        observed = reported(api, playlist_id).await;
        if observed == Some(expected) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    bail!(
        "playlist {playlist_id} did not become {}; last observed public state: {observed:?}",
        if expected { "public" } else { "private" }
    )
}

async fn reported(api: &dyn MusicApi, playlist_id: &str) -> Option<bool> {
    let detail = api
        .playlist(playlist_id)
        .await
        .ok()
        .map(|detail| detail.playlist.public);
    let listed = api.playlists(LIBRARY_LIMIT).await.ok().and_then(|entries| {
        PlaylistEntry::playlists(&entries)
            .into_iter()
            .find(|playlist| playlist.id == playlist_id)
            .map(|playlist| playlist.public)
    });

    match (detail, listed) {
        (Some(true), _) | (_, Some(true)) => Some(true),
        (found @ Some(false), _) | (_, found @ Some(false)) => found,
        _ => None,
    }
}
