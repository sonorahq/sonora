//! Cookie sessions use the web player's CENC audio, decoded by Sonora's ordinary audio engine.
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use async_trait::async_trait;
use serde_json::Value;
use widevine::Cdm;

use super::web::Web;
use crate::aac::Aac;
use crate::audio::Trimmed;
use crate::cenc::{Cenc, Media};
use crate::engine::{self, Fetch};
use crate::stream::Reader;
use crate::trim;
use crate::vault::Recording;
use crate::{Encoding, Offer, PlaybackConfig, PlaybackEvents, PlaybackFactory, Player, Quality};

// Free accounts license HIGH AAC; VERY_HIGH manifests can be listed but refuse a license.
pub(super) const QUALITIES: &[Offer] = &[Offer {
    quality: Quality::High,
    encoding: Encoding::Aac { kbps: 128 },
    up_to: false,
}];

pub(super) struct Factory(pub Web);
impl PlaybackFactory for Factory {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>) {
        engine::start(Spotify(self.0.clone()), config)
    }
}
struct Spotify(Web);
#[derive(Clone)]
struct Loaded {
    media: Media,
    duration: Option<Duration>,
}

#[async_trait]
impl Fetch for Spotify {
    type Loaded = Loaded;
    type Source = Trimmed<rodio::Decoder<Reader<Cenc>>>;
    fn name(&self) -> &'static str {
        "spotify"
    }
    fn plays_offline(&self) -> bool {
        false
    }
    async fn load(&self, id: &str) -> Result<Loaded> {
        widevine::require()?;
        ensure!(
            id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()),
            "invalid Spotify track id"
        );
        let answer = self.0.get(&format!("/track-playback/v1/media/spotify:track:{id}?manifestFileFormat=file_ids_mp4&manifestFileFormat=file_ids_mp4_dual")).await?;
        let (file, duration) = select(&answer, id)?;
        let storage = self.0.get(&format!("/storage-resolve/v2/files/audio/interactive/10/{file}?version=10000000&product=9&platform=39&alt=json")).await?;
        let url = storage["cdnurl"]
            .as_array()
            .and_then(|urls| urls.first())
            .and_then(Value::as_str)
            .context("Spotify returned no audio URL")?;
        let response = self.0.audio(url).await?;
        let total = response.content_length();
        let mut media = Media::pulling(Recording::through(response), total);
        media.prime().await?;
        let (pssh, key_id) = media
            .protection()
            .context("Spotify audio carries no Widevine init data")?;
        let licensing = widevine::licensing().await;
        let (cdm, challenge) = tokio::task::spawn_blocking(move || -> Result<_> {
            let cdm = Cdm::open()?;
            let challenge = cdm.challenge(&pssh)?;
            Ok((cdm, challenge))
        })
        .await
        .context("Spotify challenge task failed")??;
        let license = self.0.license(challenge).await?;
        let cdm = tokio::task::spawn_blocking(move || -> Result<_> {
            cdm.accept(&license)?;
            Ok(cdm)
        })
        .await
        .context("Spotify license task failed")??;
        drop(licensing);
        media.license(cdm, key_id)?;
        media.release_when_downloaded();
        Ok(Loaded { media, duration })
    }
    fn length(&self, loaded: &Loaded) -> Option<Duration> {
        loaded.duration
    }
    fn encoding(&self, _: &Loaded) -> Option<Encoding> {
        Some(Encoding::Aac { kbps: 128 })
    }
    fn progress(&self, loaded: &Loaded) -> Option<f32> {
        loaded.media.progress()
    }
    async fn downloaded(&self, loaded: &Loaded) {
        loaded.media.finished().await;
    }
    fn reopen_to_seek(&self) -> bool {
        true
    }
    fn open(&self, id: &str, loaded: &Loaded, at: Duration) -> Option<Self::Source> {
        let entry = (!at.is_zero()).then(|| loaded.media.entry(at)).flatten();
        let reader = match entry {
            Some(entry) => loaded.media.reader_at(entry),
            None => loaded.media.reader(),
        };
        let decoder = rodio::Decoder::builder()
            .with_symphonia_decoder::<Aac>()
            .with_data(reader)
            .with_seekable(false)
            .build()
            .inspect_err(|error| log::warn!("spotify: cannot decode {id}: {error}"))
            .ok()?;
        let edit = loaded.media.header().as_deref().and_then(trim::from_mp4);
        let (skip, take) = match entry {
            Some(entry) => (entry.into, None),
            None => (
                edit.map(|e| e.skip).unwrap_or_default(),
                edit.and_then(|e| e.take).or(loaded.duration),
            ),
        };
        Some(Trimmed::new(decoder, skip, take))
    }
}

fn select(answer: &Value, id: &str) -> Result<(String, Option<Duration>)> {
    let media = answer["media"]
        .as_object()
        .context("Spotify returned no media")?;
    // Region relinking may return another URI for the requested recording. Prefer the exact
    // URI, otherwise the first audio item the endpoint explicitly resolved for this request.
    let uri = format!("spotify:track:{id}");
    let item = media
        .get(&uri)
        .or_else(|| {
            media
                .values()
                .find(|entry| entry["item"]["manifest"]["file_ids_mp4"].is_array())
        })
        .context("Spotify returned no audio item")?;
    let item = &item["item"];
    let formats = item["manifest"]["file_ids_mp4"]
        .as_array()
        .context("Spotify listed no AAC files")?;
    let format = formats
        .iter()
        .find(|f| f["audio_quality"] == "HIGH")
        .context("Spotify listed no HIGH audio file")?;
    let file = format["file_id"]
        .as_str()
        .context("Spotify audio has no file id")?;
    ensure!(
        file.len() == 40 && file.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid Spotify audio file id"
    );
    let duration = item["metadata"]["duration"]
        .as_u64()
        .map(Duration::from_millis);
    Ok((file.to_owned(), duration))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_high_quality_from_relinked_recording() {
        let answer = serde_json::json!({"media": {"spotify:track:relinked": {"item": {
            "metadata": {"duration": 12000}, "manifest": {"file_ids_mp4": [
                {"audio_quality": "VERY_HIGH", "file_id": "1111111111111111111111111111111111111111"},
                {"audio_quality": "HIGH", "file_id": "2222222222222222222222222222222222222222"}
            ]}
        }}}});
        assert_eq!(
            select(&answer, "requested").unwrap(),
            (
                "2222222222222222222222222222222222222222".to_owned(),
                Some(Duration::from_secs(12))
            )
        );
        assert!(select(&serde_json::json!({}), "requested").is_err());
    }
}
