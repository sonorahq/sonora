//! Read-only check of Spotify's sidebar pins using the cached session.
use anyhow::{Context as _, Result};
use music::MusicApi;
use music::spotify::{AuthConfig, SpotifyClient, auth};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let session = auth::restore(&AuthConfig::from_env())
        .await?
        .context("no cached Spotify session")?;
    let client = SpotifyClient::new(session);
    let items = client
        .pin_targets()
        .await?
        .context("provider keeps no pins")?;
    println!(
        "pin targets={}, pinned={}",
        items.len(),
        items.iter().filter(|item| item.pinned).count()
    );
    for item in items.iter().take(10) {
        println!("{:?} pinned={} {}", item.kind, item.pinned, item.name);
    }
    Ok(())
}
