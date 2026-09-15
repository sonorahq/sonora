//! Read-only check of the Deezer protocol with a real account: sign-in, search, stream
//! resolution, and the first decrypted bytes read through the same `Stream` playback uses —
//! they must open with a known audio magic (`ID3`, an MP3 frame sync, or `fLaC`).
//!
//! Run with `DEEZER_ARL=<cookie> cargo run --example deezer-probe --package music`.
use std::io::Read as _;

use anyhow::{Context as _, Result, bail};
use music::MusicApi as _;
use music::deezer::{DeezerClient, Stream, Striped, arl};

#[tokio::main]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();
    let input = std::env::var("DEEZER_ARL").context("DEEZER_ARL is not set")?;
    let client = DeezerClient::connect(&arl(&input)?).await?;

    let profile = client.profile().await?;
    println!("profile: {} ({})", profile.display_name, profile.id);

    let found = client.search("daft punk").await?;
    let track = found.first().context("the search came back empty")?;
    let id = track.id.clone().context("the track has no id")?;
    println!("search: {} — {} ({id})", track.name, track.artists);

    let (response, key, duration) = client.open_stream(&id).await?;
    println!("stream: length {duration:?}");
    let stream = Stream::open(response, Striped::new(&key)).await?;

    let mut head = [0u8; 8192];
    stream
        .reader()
        .read_exact(&mut head)
        .context("cannot read the stream head")?;
    let magic = match &head[..4] {
        [b'I', b'D', b'3', ..] => "id3 (mp3)",
        [0xFF, second, ..] if second & 0xE0 == 0xE0 => "mp3 frame sync",
        [b'f', b'L', b'a', b'C'] => "flac",
        other => bail!("unexpected stream head: {other:02x?}"),
    };
    println!("decrypted head: {magic}");
    Ok(())
}
