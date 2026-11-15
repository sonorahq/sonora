use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};

use crate::youtube::YouTubeProvider;
use crate::{MusicApi, MusicProvider, Playlist, ProviderSession};

const NAME: &str = "Sonora live ownership test - safe to delete";
const QUERY: &str = "Rick Astley Never Gonna Give You Up";
const VERIFY_ATTEMPTS: usize = 30;

/// A playlist the account makes has to come back from the library as its own and take a track.
/// On an account without a channel this is the playlist YouTube lists with no author.
#[tokio::test]
#[ignore = "creates, changes, and deletes a playlist on the connected YouTube Music account"]
async fn youtube_lists_a_playlist_it_made_as_owned_and_adds_to_it() -> Result<()> {
    let session = connected(&YouTubeProvider::new()).await?;
    let api = session.api.as_ref();
    let playlist_id = api.create_playlist(NAME).await?;

    let exercise = async {
        let listed = wait_until_listed(api, &playlist_id).await?;
        if !listed.owned {
            bail!("the library lists the new playlist as someone else's: {listed:?}");
        }
        let track = api
            .search(QUERY)
            .await?
            .into_iter()
            .find_map(|track| track.id)
            .context("the search found no track to add")?;
        api.add_track_to_playlist(&playlist_id, &track).await?;
        wait_until_holds(api, &playlist_id, &track).await
    }
    .await;

    let cleanup = api.delete_playlist(&playlist_id).await;
    match (exercise, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error.context("ownership check failed; playlist deleted")),
        (Ok(()), Err(error)) => Err(error.context("ownership check passed but cleanup failed")),
        (Err(exercise), Err(cleanup)) => Err(anyhow!(
            "ownership check failed: {exercise:#}; deleting the playlist also failed: {cleanup:#}"
        )),
    }
}

/// Checks every playlist of the library against its own page, which names the author of a
/// playlist that belongs to someone else. Reads only, and prints what it found for each. A
/// page that calls itself editable proves nothing, since YouTube says so of its own recaps.
#[tokio::test]
#[ignore = "reads every playlist of the connected YouTube Music account"]
async fn youtube_marks_only_the_accounts_own_library_playlists_as_owned() -> Result<()> {
    let session = connected(&YouTubeProvider::new()).await?;
    let api = session.api.as_ref();
    let account = session.profile.display_name.clone();
    let listed = api.playlists().await?;
    if listed.is_empty() {
        bail!("the connected account has no playlists to check");
    }

    let mut wrong = Vec::new();
    for playlist in &listed {
        let page = api
            .playlist(&playlist.id)
            .await
            .with_context(|| format!("cannot read playlist {}", playlist.id))?
            .playlist;
        eprintln!(
            "{:<40} listed owned={:<5} owner={:<20?} page owned={:<5} owner={:?}",
            playlist.name, playlist.owned, playlist.owner, page.owned, page.owner
        );
        let foreign = !page.owner.is_empty() && page.owner != account;
        if playlist.owned && !page.owned && foreign {
            wrong.push(format!(
                "{} is listed as owned but belongs to {}",
                playlist.name, page.owner
            ));
        }
    }

    match wrong.is_empty() {
        true => Ok(()),
        false => bail!("ownership is wrong for: {}", wrong.join("; ")),
    }
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

async fn wait_until_listed(api: &dyn MusicApi, playlist_id: &str) -> Result<Playlist> {
    for _ in 0..VERIFY_ATTEMPTS {
        let found = api
            .playlists()
            .await?
            .into_iter()
            .find(|playlist| playlist.id == playlist_id);
        if let Some(playlist) = found {
            return Ok(playlist);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    bail!("playlist {playlist_id} never showed up in the library")
}

async fn wait_until_holds(api: &dyn MusicApi, playlist_id: &str, track_id: &str) -> Result<()> {
    for _ in 0..VERIFY_ATTEMPTS {
        let tracks = api.playlist_tracks(playlist_id).await?;
        if tracks
            .iter()
            .any(|track| track.id.as_deref() == Some(track_id))
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    bail!("playlist {playlist_id} never showed track {track_id}")
}
