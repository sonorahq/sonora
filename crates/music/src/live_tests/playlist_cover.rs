use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use image::{DynamicImage, ImageFormat, RgbImage};
use storage::{Cache, Database};

use crate::deezer::DeezerProvider;
use crate::local::LocalProvider;
use crate::spotify::SpotifyProvider;
use crate::subsonic::{SubsonicClient, auth};
use crate::tidal::TidalProvider;
use crate::youtube::YouTubeProvider;
use crate::{MusicApi, MusicProvider, ProviderSession, SignIn};

const NAME: &str = "Sonora live cover test - safe to delete";
const VERIFY_ATTEMPTS: usize = 30;

#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected Spotify account"]
async fn spotify_can_change_a_playlist_cover() -> Result<()> {
    exercise(connected(&SpotifyProvider::from_env()).await?.api.as_ref()).await
}

#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected YouTube Music account"]
async fn youtube_can_change_a_playlist_cover() -> Result<()> {
    exercise(connected(&YouTubeProvider::new()).await?.api.as_ref()).await
}

#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected Deezer account"]
async fn deezer_can_change_a_playlist_cover() -> Result<()> {
    exercise(connected(&DeezerProvider::new()).await?.api.as_ref()).await
}

#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected Tidal account"]
async fn tidal_can_change_a_playlist_cover() -> Result<()> {
    exercise(connected(&TidalProvider::new()).await?.api.as_ref()).await
}

/// Runs against the Navidrome in `SONORA_SUBSONIC_SERVER`, signed in with
/// `SONORA_SUBSONIC_USERNAME` and `SONORA_SUBSONIC_PASSWORD`.
#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on a Navidrome server"]
async fn navidrome_can_change_a_playlist_cover() -> Result<()> {
    let variable = |name: &str| std::env::var(name).with_context(|| format!("{name} is not set"));
    let username = variable("SONORA_SUBSONIC_USERNAME")?;
    let password = variable("SONORA_SUBSONIC_PASSWORD")?;
    let signature = auth::sign(&username, &password);
    let client = SubsonicClient::new(
        variable("SONORA_SUBSONIC_SERVER")?,
        username,
        password,
        &signature,
    )?;
    if !client.takes_playlist_covers().await {
        bail!("the server is not a Navidrome that takes playlist covers");
    }
    exercise(&client).await
}

#[tokio::test]
#[ignore = "writes a cover into Sonora's data folder"]
async fn local_can_change_a_playlist_cover() -> Result<()> {
    let folder = tempfile::tempdir()?;
    let provider = LocalProvider::new(
        folder.path().join("cache"),
        Database::at(folder.path().join("state.sqlite")),
        Cache::at(folder.path().join("cache.sqlite")),
    );
    let (_, input) = tokio::sync::mpsc::unbounded_channel();
    let session = provider
        .sign_in(SignIn::Path(Vec::new()), Arc::new(|_| {}), input)
        .await?;
    exercise(session.api.as_ref()).await
}

async fn connected(provider: &dyn MusicProvider) -> Result<ProviderSession> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
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
    if !session.capabilities.playlist_covers {
        bail!("{} does not offer playlist covers", provider.name());
    }
    Ok(session)
}

/// Creates a playlist, gives it a cover, takes the cover off again and deletes it, checking
/// the cover the provider reports after each change. A read that fails counts as not there
/// yet, since YouTube cannot read a playlist for a moment after creating it.
async fn exercise(api: &dyn MusicApi) -> Result<()> {
    let playlist_id = api.create_playlist(NAME).await?;

    let exercise = async {
        let bare = wait_for(api, &playlist_id, |_| true)
            .await
            .context("the new playlist never became readable")?;
        api.set_playlist_cover(&playlist_id, Some(&picture()))
            .await
            .context("cannot set the cover")?;
        let custom = wait_for(api, &playlist_id, |now| now.is_some() && *now != bare)
            .await
            .context("the new cover never showed")?;
        println!("{playlist_id}: cover {bare:?} became {custom:?}");

        api.set_playlist_cover(&playlist_id, None)
            .await
            .context("cannot remove the cover")?;
        wait_for(api, &playlist_id, |now| *now == bare)
            .await
            .context("the provider's own cover never came back")?;
        println!("{playlist_id}: cover went back to {bare:?}");
        Ok::<(), anyhow::Error>(())
    }
    .await;

    let cleanup = api.delete_playlist(&playlist_id).await;
    match (exercise, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.context("cover cycle failed; playlist deleted")),
        (Ok(()), Err(error)) => Err(error.context("cover cycle passed but cleanup failed")),
        (Err(exercise), Err(cleanup)) => Err(anyhow!(
            "cover cycle failed: {exercise:#}; deleting the playlist also failed: {cleanup:#}"
        )),
    }
}

async fn cover(api: &dyn MusicApi, playlist_id: &str) -> Result<Option<String>> {
    Ok(api.playlist(playlist_id).await?.playlist.cover)
}

async fn wait_for(
    api: &dyn MusicApi,
    playlist_id: &str,
    done: impl Fn(&Option<String>) -> bool,
) -> Result<Option<String>> {
    let mut observed = None;
    for _ in 0..VERIFY_ATTEMPTS {
        match cover(api, playlist_id).await {
            Ok(now) if done(&now) => return Ok(now),
            Ok(now) => observed = Some(format!("{now:?}")),
            Err(error) => observed = Some(format!("{error:#}")),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    bail!("last observed: {}", observed.unwrap_or_default())
}

/// A wide PNG with a gradient, so the provider gets a picture it has to crop and recompress.
fn picture() -> Vec<u8> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as u8)
        .unwrap_or_default();
    let image = RgbImage::from_fn(900, 700, |x, y| {
        image::Rgb([(x / 4) as u8, (y / 3) as u8, stamp.wrapping_mul(37)])
    });
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
        .expect("a png encodes into memory");
    out
}
