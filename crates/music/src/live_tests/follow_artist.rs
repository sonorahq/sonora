use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::spotify::SpotifyProvider;
use crate::youtube::YouTubeProvider;
use crate::{MusicApi, MusicProvider, ProviderSession};

const SPOTIFY_ARTISTS: &[&str] = &["1WAB4gjjNfQpAgT5SoAbRE"];
const YOUTUBE_ARTISTS: &[&str] = &["UCilQecy8UKHUSgE6l-uxP1Q"];
const LIBRARY_LIMIT: u32 = 10_000;
const VERIFY_ATTEMPTS: usize = 30;

#[tokio::test]
#[ignore = "changes the followed artists of the connected Spotify account"]
async fn spotify_can_follow_and_unfollow_an_artist_for_the_connected_account() -> Result<()> {
    let provider = SpotifyProvider::from_env(ytmusic::YtMusic::anonymous().into());
    let session = connected(&provider).await?;

    exercise_artist_cycles(session.api.as_ref(), SPOTIFY_ARTISTS).await
}

#[tokio::test]
#[ignore = "changes the followed artists of the connected YouTube Music account"]
async fn youtube_can_follow_and_unfollow_an_artist_for_the_connected_account() -> Result<()> {
    let provider = YouTubeProvider::new();
    let session = connected(&provider).await?;

    exercise_artist_cycles(session.api.as_ref(), YOUTUBE_ARTISTS).await
}

async fn exercise_artist_cycles(api: &dyn MusicApi, artist_ids: &[&str]) -> Result<()> {
    for artist_id in artist_ids {
        exercise_artist_cycle(api, artist_id).await?;
    }
    Ok(())
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

async fn exercise_artist_cycle(api: &dyn MusicApi, artist_id: &str) -> Result<()> {
    let originally_saved = artist_is_saved(api, artist_id).await?;
    let changed = !originally_saved;

    let exercise = async {
        api.set_artist_saved(artist_id, changed).await?;
        wait_until_saved(api, artist_id, changed).await
    }
    .await;

    let restore = async {
        api.set_artist_saved(artist_id, originally_saved).await?;
        wait_until_saved(api, artist_id, originally_saved).await
    }
    .await;

    match (exercise, restore) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.context("artist cycle failed; original state restored")),
        (Ok(()), Err(error)) => Err(error.context("artist cycle passed but restoring it failed")),
        (Err(exercise), Err(restore)) => Err(anyhow!(
            "artist cycle failed: {exercise:#}; restoring it also failed: {restore:#}"
        )),
    }
}

async fn wait_until_saved(api: &dyn MusicApi, artist_id: &str, expected: bool) -> Result<()> {
    for _ in 0..VERIFY_ATTEMPTS {
        if artist_is_saved(api, artist_id).await? == expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    bail!(
        "artist {artist_id} did not become {}",
        if expected { "followed" } else { "unfollowed" }
    )
}

async fn artist_is_saved(api: &dyn MusicApi, artist_id: &str) -> Result<bool> {
    Ok(api
        .saved_artists(LIBRARY_LIMIT)
        .await?
        .iter()
        .any(|artist| artist.id == artist_id))
}
