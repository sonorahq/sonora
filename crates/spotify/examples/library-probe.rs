//! Read-only check of Spotify's mixed library using the cached session.
use anyhow::{Context as _, Result};
use music::MusicApi;
use spotify::{AuthConfig, LibrespotClient, auth};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let session = auth::restore(&AuthConfig::from_env())
        .await?
        .context("no cached Spotify session")?;
    let client = LibrespotClient::new(session);
    let items = client
        .pin_targets()
        .await?
        .context("provider keeps no pins")?;
    println!(
        "library items={}, pinned={}",
        items.len(),
        items.iter().filter(|item| item.pinned).count()
    );
    for item in items.iter().take(10) {
        println!("{:?} pinned={} {}", item.kind, item.pinned, item.name);
    }
    Ok(())
}
