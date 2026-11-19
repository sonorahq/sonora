use std::collections::HashMap;

use anyhow::{Context as _, Result};
use librespot_core::Session;
use librespot_protocol::extension_kind::ExtensionKind;
use serde::Deserialize;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::Track;
use crate::spotify::{collection, pb};

/// Spotify's audio-features request limit.
const FEATURES_AT_ONCE: usize = 100;

/// Fetches descriptors and audio features independently; a failed source leaves its fields empty.
pub(crate) async fn load(session: &Session, ids: &[String]) -> Vec<Track> {
    let uris: Vec<_> = ids.iter().map(|id| format!("spotify:track:{id}")).collect();
    let (words, sound) = tokio::join!(
        collection::extended(session, &uris, ExtensionKind::TRACK_DESCRIPTOR),
        features(session, ids),
    );
    let mut words: HashMap<_, _> = words
        .unwrap_or_else(|error| {
            log::warn!("descriptors: cannot read Spotify descriptor words: {error:#}");
            Vec::new()
        })
        .into_iter()
        .filter_map(|entity| match concepts(&entity.extension_data.value) {
            Ok(found) => Some((entity.entity_uri, found)),
            Err(error) => {
                log::warn!("descriptors: skipped {}: {error:#}", entity.entity_uri);
                None
            }
        })
        .collect();
    let mut sound = sound.unwrap_or_else(|error| {
        log::warn!("descriptors: cannot read Spotify audio features: {error:#}");
        HashMap::new()
    });
    ids.iter()
        .map(|id| {
            let (genres, tags) = words
                .remove(&format!("spotify:track:{id}"))
                .unwrap_or_default();
            let features = sound.remove(id);
            Track {
                id: Some(id.clone()),
                genres,
                tags,
                moods: features.as_ref().map(Features::moods).unwrap_or_default(),
                bpm: features
                    .and_then(|features| features.tempo)
                    .filter(|bpm| bpm.is_finite() && *bpm > 0.),
                ..Track::default()
            }
        })
        .collect()
}

/// Decodes descriptor labels, separating genres from other tags.
fn concepts(bytes: &[u8]) -> Result<(Vec<String>, Vec<String>)> {
    let mut reader = pb::Reader::new(bytes);
    let mut genres = Vec::new();
    let mut tags = Vec::new();
    while let Some((field, value)) = reader.field()? {
        let (1, pb::Value::Bytes(descriptor)) = (field, value) else {
            continue;
        };
        let mut descriptor = pb::Reader::new(descriptor);
        let mut text = String::new();
        let mut genre = false;
        while let Some((field, value)) = descriptor.field()? {
            match (field, value) {
                (1, pb::Value::Bytes(value)) => text = pb::text(value)?,
                (3, pb::Value::Varint(value)) => genre |= value == 1,
                (3, pb::Value::Bytes(values)) => genre |= pb::contains_varint(values, 1)?,
                _ => {}
            }
        }
        let text = text.trim().to_lowercase();
        let text = text
            .strip_prefix("spotify:concept:")
            .unwrap_or(&text)
            .replace('_', " ");
        if text.is_empty() || text == "track" {
            continue;
        }
        let text: String = text
            .split_word_bounds()
            .flat_map(|word| {
                let mut letters = word.chars();
                letters
                    .next()
                    .into_iter()
                    .flat_map(char::to_uppercase)
                    .chain(letters)
            })
            .collect();
        let list = match genre {
            true => &mut genres,
            false => &mut tags,
        };
        if !list.contains(&text) {
            list.push(text);
        }
    }
    Ok((genres, tags))
}

/// Spotify audio features: scores from zero to one, and tempo in BPM.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Features {
    id: String,
    danceability: Option<f32>,
    energy: Option<f32>,
    valence: Option<f32>,
    acousticness: Option<f32>,
    instrumentalness: Option<f32>,
    tempo: Option<f32>,
}

impl Features {
    /// Maps audio-feature thresholds to localized mood keys. Missing scores produce no labels.
    fn moods(&self) -> Vec<String> {
        [
            (self.energy.is_some_and(|v| v >= 0.7), "mood-energetic"),
            (
                self.danceability.is_some_and(|v| v >= 0.7),
                "mood-danceable",
            ),
            (self.valence.is_some_and(|v| v >= 0.6), "mood-positive"),
            (
                self.valence.is_some_and(|v| v <= 0.3) && self.energy.is_some_and(|v| v >= 0.5),
                "mood-negative",
            ),
            (
                self.valence.is_some_and(|v| v <= 0.3) && self.energy.is_some_and(|v| v < 0.5),
                "mood-sad",
            ),
            (self.energy.is_some_and(|v| v <= 0.35), "mood-calming"),
            (
                self.instrumentalness.is_some_and(|v| v >= 0.5),
                "mood-instrumental",
            ),
            (self.acousticness.is_some_and(|v| v >= 0.6), "mood-acoustic"),
        ]
        .into_iter()
        .filter(|(matches, _)| *matches)
        .map(|(_, mood)| mood.to_owned())
        .collect()
    }
}

/// Fetches audio features within Spotify's request limit, omitting unknown tracks.
async fn features(session: &Session, ids: &[String]) -> Result<HashMap<String, Features>> {
    #[derive(Deserialize)]
    struct Answer {
        audio_features: Vec<Option<Features>>,
    }
    let mut found = HashMap::with_capacity(ids.len());
    for ids in ids.chunks(FEATURES_AT_ONCE) {
        let path = format!("/audio-attributes/v1/audio-features?ids={}", ids.join(","));
        let body = session
            .spclient()
            .request_as_json(&http::Method::GET, &path, None, None)
            .await
            .context("cannot request audio features")?;
        let answer: Answer =
            serde_json::from_slice(&body).context("cannot decode audio features")?;
        for mut features in answer.audio_features.into_iter().flatten() {
            found.insert(std::mem::take(&mut features.id), features);
        }
    }
    Ok(found)
}
