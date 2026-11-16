use anyhow::{Context as _, Result, bail};

use crate::apple::AppleProvider;
use crate::deezer::DeezerProvider;
use crate::spotify::SpotifyProvider;
use crate::subsonic::{SubsonicClient, auth};
use crate::youtube::YouTubeProvider;
use crate::{MusicApi, MusicProvider, ReleaseType};

/// A release every catalogue files as an EP, and the query that finds it.
const ARTIST: &str = "Sylosis";
const TITLE: &str = "The Path";

#[tokio::test]
#[ignore = "reads the catalogue through the stored Spotify session"]
async fn spotify_labels_an_ep_as_one() -> Result<()> {
    album_is_an_ep(&SpotifyProvider::from_env()).await
}

#[tokio::test]
#[ignore = "reads the catalogue through the stored YouTube Music session"]
async fn youtube_labels_an_ep_as_one() -> Result<()> {
    album_is_an_ep(&YouTubeProvider::new()).await
}

#[tokio::test]
#[ignore = "reads the catalogue through the stored Deezer session"]
async fn deezer_labels_an_ep_as_one() -> Result<()> {
    album_is_an_ep(&DeezerProvider::new()).await
}

#[tokio::test]
#[ignore = "reads the catalogue through the stored Apple Music session"]
async fn apple_labels_an_ep_as_one() -> Result<()> {
    album_is_an_ep(&AppleProvider::new()).await
}

/// Runs against the server in `SONORA_SUBSONIC_SERVER`, signed in with `SONORA_SUBSONIC_USERNAME`
/// and `SONORA_SUBSONIC_PASSWORD`, which has to hold the EP tagged with `RELEASETYPE=ep`.
#[tokio::test]
#[ignore = "needs a Subsonic server holding the EP"]
async fn subsonic_labels_an_ep_as_one() -> Result<()> {
    let variable = |name: &str| std::env::var(name).with_context(|| format!("{name} is not set"));
    let username = variable("SONORA_SUBSONIC_USERNAME")?;
    let password = variable("SONORA_SUBSONIC_PASSWORD")?;
    let signature = auth::sign(&username, &password);
    let client = SubsonicClient::new(
        variable("SONORA_SUBSONIC_SERVER")?,
        username,
        password,
        &signature,
        None,
    )?;
    ep_is_labelled("Subsonic", &client).await
}

async fn album_is_an_ep(provider: &dyn MusicProvider) -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let session = provider
        .restore()
        .await?
        .with_context(|| format!("{} has no stored Sonora session", provider.name()))?;
    ep_is_labelled(provider.name(), session.api.as_ref()).await
}

/// Finds the EP by search and checks both the search hit and the album page call it an EP.
async fn ep_is_labelled(provider: &str, api: &dyn MusicApi) -> Result<()> {
    let hits = api
        .search_albums(&format!("{ARTIST} {TITLE}"))
        .await
        .with_context(|| format!("{provider} could not search albums"))?;
    let Some(hit) = hits
        .iter()
        .find(|album| album.name.starts_with(TITLE) && album.artists.contains(ARTIST))
    else {
        bail!(
            "{provider} found no {ARTIST} - {TITLE} among {} hits",
            hits.len()
        );
    };
    let detail = api
        .album(&hit.id)
        .await
        .with_context(|| format!("{provider} could not load album {}", hit.id))?;

    let found = (hit.release_type, detail.album.release_type);
    if found != (ReleaseType::Ep, ReleaseType::Ep) {
        bail!("{provider} labels {TITLE} as {found:?} (search, page), not an EP");
    }
    Ok(())
}
