//! Read-only check of the Apple Music account behind `SONORA_APPLE_MEDIA_USER_TOKEN`, without
//! building the app: the profile the account card draws, and the favorited songs the library
//! shows as loved.
//!
//! Set `SONORA_APPLE_REPORT_ID` to a catalog song id (the numeric `adam-id`, e.g. `1440913722`)
//! to also fire one play-activity beacon and check whether the play reaches the account's
//! recently played on another device. Leave it unset for a purely read-only run.
use anyhow::{Context as _, Result};
use music::MusicApi as _;
use music::apple::{self, AppleClient};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let token = apple::account()
        .context("no apple account: set SONORA_APPLE_MEDIA_USER_TOKEN to your media-user-token")?;
    let client = AppleClient::connect(&token).await?;

    let profile = client.profile().await?;
    println!("profile name: {}", profile.display_name);
    println!("storefront:   {}", profile.id);
    match &profile.avatar {
        Some(url) => println!("avatar:       {url}"),
        None => println!("avatar:       none (card falls back to initials)"),
    }

    let saved = client.saved_tracks().await?;
    println!("\nfavorited songs: {}", saved.len());
    for track in saved.iter().take(10) {
        println!("  {} — {}", track.name, track.artists);
    }

    let recent = client.recently_played().await?;
    println!(
        "\nrecently played (cross-device, newest first): {}",
        recent.len()
    );
    for track in recent.iter().take(15) {
        println!("  {} — {}", track.name, track.artists);
    }

    if let Ok(id) = std::env::var("SONORA_APPLE_REPORT_ID")
        && !id.is_empty()
    {
        println!("\nreporting a play of {id} …");
        match client.report_play(&id).await {
            Ok(()) => println!("  accepted by the beacon"),
            Err(error) => println!("  failed: {error:#}"),
        }
        // Give Apple a moment to fold the play into the account, then read the list back and see
        // whether the reported id surfaced. This is the whole write→read round trip in one run.
        println!("  waiting 8s, then re-reading recently played …");
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        let after = client.recently_played().await?;
        let landed = after
            .iter()
            .take(5)
            .any(|track| track.id.as_deref() == Some(id.as_str()));
        match landed {
            true => println!("  landed: the reported play is now near the top ✓"),
            false => println!(
                "  not visible yet in the top of recently played (Apple can lag, or the id/shape is off)"
            ),
        }
    }
    Ok(())
}
