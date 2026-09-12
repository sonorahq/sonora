use std::env;

use anyhow::{Context as _, Result, bail};

use crate::youtube::YouTubeProvider;
use crate::{MusicProvider, ProviderSession};

const LIBRARY_LIMIT: u32 = 10_000;

/// Read-only coverage for the cookie-backed session that the OAuth probe must not replace.
#[tokio::test]
#[ignore = "reads a connected YouTube Music account and needs baseline playlist/video ids"]
async fn youtube_cookie_session_covers_baseline_reads() -> Result<()> {
    let provider = YouTubeProvider::new();
    let session = connected(&provider).await?;
    let api = session.api.as_ref();

    api.profile()
        .await
        .context("YouTube profile baseline failed")?;
    api.home().await.context("YouTube home baseline failed")?;
    api.search("Sonora")
        .await
        .context("YouTube search baseline failed")?;
    api.saved_tracks(LIBRARY_LIMIT)
        .await
        .context("YouTube liked-songs baseline failed")?;
    api.playlists(LIBRARY_LIMIT)
        .await
        .context("YouTube library-playlists baseline failed")?;

    let playlist_id = env::var("SONORA_YOUTUBE_BASELINE_PLAYLIST_ID")
        .context("set SONORA_YOUTUBE_BASELINE_PLAYLIST_ID to a playlist id")?;
    api.playlist(&playlist_id)
        .await
        .with_context(|| format!("YouTube playlist baseline failed for {playlist_id}"))?;

    let video_id = env::var("SONORA_YOUTUBE_BASELINE_VIDEO_ID")
        .context("set SONORA_YOUTUBE_BASELINE_VIDEO_ID to a playable video id")?;
    api.track(&video_id)
        .await
        .with_context(|| format!("YouTube player baseline failed for {video_id}"))?;
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
