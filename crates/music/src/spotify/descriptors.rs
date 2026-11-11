use std::collections::HashMap;

use anyhow::{Context as _, Result};
use librespot_core::Session;
use librespot_protocol::extension_kind::ExtensionKind;
use serde::Deserialize;
use unicode_segmentation::UnicodeSegmentation as _;

use crate::Descriptors;
use crate::spotify::{collection, pb};

const TRACK_PREFIX: &str = "spotify:track:";
/// How many tracks one audio features request names.
const FEATURES_AT_ONCE: usize = 100;

/// Reads the genres, moods, descriptive words and tempo of tracks by id, over librespot's own
/// metadata transport and spclient, never the Web API. Either source can fail alone and the other
/// still describes the tracks; only both failing is an error.
pub(crate) async fn read(
    session: &Session,
    ids: &[String],
) -> Result<HashMap<String, Descriptors>> {
    let uris: Vec<_> = ids.iter().map(|id| format!("{TRACK_PREFIX}{id}")).collect();
    let (words, sound) = tokio::join!(words(session, &uris), features(session, ids));
    if let (Err(error), Err(_)) = (&words, &sound) {
        anyhow::bail!("cannot read track descriptors: {error:#}");
    }
    let mut words = words.unwrap_or_else(|error| {
        log::warn!("descriptors: cannot read Spotify descriptor words: {error:#}");
        HashMap::new()
    });
    let sound = sound.unwrap_or_else(|error| {
        log::warn!("descriptors: cannot read Spotify audio features: {error:#}");
        HashMap::new()
    });
    Ok(ids
        .iter()
        .zip(&uris)
        .map(|(id, uri)| {
            let mut found = words.remove(uri).unwrap_or_default();
            if let Some(features) = sound.get(id) {
                found.moods = features.moods();
                found.bpm = Some(features.tempo).filter(|bpm| *bpm > 0.);
            }
            (id.clone(), found)
        })
        .collect())
}

/// Reads the typed descriptor words of each track, by uri. A track with none is left untagged.
async fn words(session: &Session, uris: &[String]) -> Result<HashMap<String, Descriptors>> {
    let entities = collection::extended(session, uris, ExtensionKind::TRACK_DESCRIPTOR).await?;
    Ok(entities
        .into_iter()
        .filter_map(|entity| match concepts(&entity.extension_data.value) {
            Ok(found) => Some((entity.entity_uri, found)),
            Err(error) => {
                log::warn!("descriptors: skipped {}: {error:#}", entity.entity_uri);
                None
            }
        })
        .collect())
}

/// Reads typed descriptor text, keeping genres apart from other descriptive tags.
fn concepts(bytes: &[u8]) -> Result<Descriptors> {
    let mut reader = pb::Reader::new(bytes);
    let mut found = Descriptors::default();
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
                (3, pb::Value::Bytes(values)) => genre |= pb::varints(values)?.contains(&1),
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
        let text = titlecase(&text);
        let list = match genre {
            true => &mut found.genres,
            false => &mut found.tags,
        };
        if !list.contains(&text) {
            list.push(text);
        }
    }
    Ok(found)
}

/// Capitalises each word, as a genre or tag is written for display.
fn titlecase(text: &str) -> String {
    text.split_word_bounds()
        .flat_map(|word| {
            let mut letters = word.chars();
            letters
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .chain(letters)
        })
        .collect()
}

/// How a song sounds, as Spotify's audio analysis measures it. Each measure runs from none to all,
/// and the tempo is in beats per minute.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Features {
    id: String,
    danceability: f32,
    energy: f32,
    valence: f32,
    acousticness: f32,
    instrumentalness: f32,
    tempo: f32,
}

impl Features {
    /// The feelings the song carries, by the Fluent key that names each. A song can carry several,
    /// and a middling one none.
    fn moods(&self) -> Vec<String> {
        [
            (self.energy >= 0.7, "stats-mood-energetic"),
            (self.danceability >= 0.7, "stats-mood-danceable"),
            (self.valence >= 0.6, "stats-mood-positive"),
            (
                self.valence <= 0.3 && self.energy >= 0.5,
                "stats-mood-negative",
            ),
            (self.valence <= 0.3 && self.energy < 0.5, "stats-mood-sad"),
            (self.energy <= 0.35, "stats-mood-calming"),
            (self.instrumentalness >= 0.5, "stats-mood-instrumental"),
            (self.acousticness >= 0.6, "stats-mood-acoustic"),
        ]
        .into_iter()
        .filter(|(matches, _)| *matches)
        .map(|(_, mood)| mood.to_owned())
        .collect()
    }
}

/// Reads how each of the tracks sounds by id, a hundred to a request, which is what Spotify's own
/// audio analysis answers for. A track it does not know is left out.
async fn features(session: &Session, ids: &[String]) -> Result<HashMap<String, Features>> {
    #[derive(Deserialize)]
    struct Answer {
        audio_features: Vec<Option<Features>>,
    }
    let answers =
        futures::future::try_join_all(ids.chunks(FEATURES_AT_ONCE).map(|ids| async move {
            let path = format!("/audio-attributes/v1/audio-features?ids={}", ids.join(","));
            let body = session
                .spclient()
                .request_as_json(&http::Method::GET, &path, None, None)
                .await
                .context("cannot request audio features")?;
            serde_json::from_slice::<Answer>(&body).context("cannot decode audio features")
        }))
        .await?;
    let found = answers
        .into_iter()
        .flat_map(|answer| answer.audio_features)
        .flatten()
        .map(|features| (features.id.clone(), features))
        .collect();
    Ok(found)
}
