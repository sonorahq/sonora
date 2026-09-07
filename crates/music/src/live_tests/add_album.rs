use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::spotify::SpotifyProvider;
use crate::youtube::YouTubeProvider;
use crate::{Album, MusicApi, MusicProvider, ProviderSession};

const SPOTIFY_ALBUMS: &[&str] = &["1vHPNtDfd0V29ol70EMqP8", "2Ef2E0yk88zQfjvOJunK8A"];
const YOUTUBE_ALBUMS: &[&str] = &["MPREb_vupB1BNh7XE", "MPREb_3SWMG6RbCTQ"];
const LIBRARY_LIMIT: u32 = 10_000;
const VERIFY_ATTEMPTS: usize = 30;

#[tokio::test]
#[ignore = "changes the saved albums of the connected Spotify account"]
async fn spotify_can_add_and_remove_an_album_for_the_connected_account() -> Result<()> {
    let provider = SpotifyProvider::from_env(ytmusic::YtMusic::anonymous().into());
    let session = connected(&provider).await?;

    exercise_album_cycles("Spotify", session.api.as_ref(), SPOTIFY_ALBUMS).await
}

#[tokio::test]
#[ignore = "changes the saved albums of the connected YouTube Music account"]
async fn youtube_can_add_and_remove_an_album_for_the_connected_account() -> Result<()> {
    let provider = YouTubeProvider::new();
    let session = connected(&provider).await?;

    exercise_album_cycles("YouTube Music", session.api.as_ref(), YOUTUBE_ALBUMS).await
}

async fn exercise_album_cycles(
    provider: &str,
    api: &dyn MusicApi,
    album_ids: &[&str],
) -> Result<()> {
    for album_id in album_ids {
        exercise_album_cycle(provider, api, album_id).await?;
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

async fn exercise_album_cycle(provider: &str, api: &dyn MusicApi, album_id: &str) -> Result<()> {
    let album = api
        .album(album_id)
        .await
        .with_context(|| format!("{provider} could not load album {album_id}"))?
        .album;
    let originally_saved = album_is_saved(api, &album.id).await?;
    let changed = !originally_saved;

    let exercise = async {
        api.set_album_saved(&album.id, changed).await?;
        wait_until_saved(api, &album, changed).await
    }
    .await;

    let restore = async {
        api.set_album_saved(&album.id, originally_saved).await?;
        wait_until_saved(api, &album, originally_saved).await
    }
    .await;

    match (exercise, restore) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.context("album cycle failed; original state restored")),
        (Ok(()), Err(error)) => Err(error.context("album cycle passed but restoring it failed")),
        (Err(exercise), Err(restore)) => Err(anyhow!(
            "album cycle failed: {exercise:#}; restoring it also failed: {restore:#}"
        )),
    }
}

async fn wait_until_saved(api: &dyn MusicApi, album: &Album, expected: bool) -> Result<()> {
    for _ in 0..VERIFY_ATTEMPTS {
        if album_is_saved(api, &album.id).await? == expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    bail!(
        "album {:?} ({}) did not become {}",
        album.name,
        album.id,
        if expected { "saved" } else { "unsaved" }
    )
}

async fn album_is_saved(api: &dyn MusicApi, album_id: &str) -> Result<bool> {
    Ok(api
        .saved_albums(LIBRARY_LIMIT)
        .await?
        .iter()
        .any(|album| album.id == album_id))
}
