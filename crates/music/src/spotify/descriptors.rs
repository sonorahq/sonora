use std::collections::HashMap;

use anyhow::{Context as _, Result};
use librespot_core::Session;
use librespot_protocol::extension_kind::ExtensionKind;
use serde::Deserialize;

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
    let mut words = words.unwrap_or_default();
    let sound = sound.unwrap_or_default();
    Ok(ids
        .iter()
        .map(|id| {
            let mut found = words
                .remove(&format!("{TRACK_PREFIX}{id}"))
                .unwrap_or_default();
            if let Some(features) = sound.get(id) {
                found.moods = features.moods().into_iter().map(str::to_owned).collect();
                found.bpm = Some(features.tempo).filter(|bpm| *bpm > 0.);
            }
            (id.clone(), found)
        })
        .collect())
}

/// Reads the typed descriptor words of each track, by uri. A track with none is left untagged.
async fn words(session: &Session, uris: &[String]) -> Result<HashMap<String, Descriptors>> {
    let entities = collection::extended(session, uris, ExtensionKind::TRACK_DESCRIPTOR).await?;
    entities
        .into_iter()
        .map(|entity| Ok((entity.entity_uri, concepts(&entity.extension_data.value)?)))
        .collect()
}

/// Reads descriptor text and its repeated type enums: type 1 is a genre, and any other word is a
/// tag.
fn concepts(bytes: &[u8]) -> Result<Descriptors> {
    let mut reader = pb::Reader::new(bytes);
    let mut found = Descriptors::default();
    while let Some((field, value)) = reader.field()? {
        let (1, pb::Value::Bytes(descriptor)) = (field, value) else {
            continue;
        };
        let mut descriptor = pb::Reader::new(descriptor);
        let mut text = None;
        let mut kinds = Vec::new();
        while let Some((field, value)) = descriptor.field()? {
            match (field, value) {
                (1, pb::Value::Bytes(value)) => text = Some(pb::text(value)?),
                (3, pb::Value::Varint(value)) => kinds.push(value),
                (3, pb::Value::Bytes(values)) => kinds.extend(pb::varints(values)?),
                _ => {}
            }
        }
        let Some(text) = text else { continue };
        let text = text.trim().to_lowercase();
        let text = text
            .strip_prefix("spotify:concept:")
            .unwrap_or(&text)
            .replace('_', " ");
        if text.is_empty() || text == "track" {
            continue;
        }
        let list = match kinds.contains(&1) {
            true => &mut found.genres,
            false => &mut found.tags,
        };
        if !list.contains(&text) {
            list.push(text);
        }
    }
    Ok(found)
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
    fn moods(&self) -> Vec<&'static str> {
        let mut moods = Vec::new();
        let mut when = |condition: bool, mood| {
            if condition {
                moods.push(mood);
            }
        };
        when(self.energy >= 0.7, "stats-mood-energetic");
        when(self.danceability >= 0.7, "stats-mood-danceable");
        when(self.valence >= 0.6, "stats-mood-positive");
        when(
            self.valence <= 0.3 && self.energy >= 0.5,
            "stats-mood-negative",
        );
        when(self.valence <= 0.3 && self.energy < 0.5, "stats-mood-sad");
        when(self.energy <= 0.35, "stats-mood-calming");
        when(self.instrumentalness >= 0.5, "stats-mood-instrumental");
        when(self.acousticness >= 0.6, "stats-mood-acoustic");
        moods
    }
}

/// Reads how each of the tracks sounds by id, a hundred to a request, which is what Spotify's own
/// audio analysis answers for. A track it does not know is left out.
async fn features(session: &Session, ids: &[String]) -> Result<HashMap<String, Features>> {
    #[derive(Deserialize)]
    struct Answer {
        audio_features: Vec<Option<Features>>,
    }
    let mut found = HashMap::new();
    for ids in ids.chunks(FEATURES_AT_ONCE) {
        let path = format!("/audio-attributes/v1/audio-features?ids={}", ids.join(","));
        let body = session
            .spclient()
            .request_as_json(&http::Method::GET, &path, None, None)
            .await
            .context("cannot request audio features")?;
        let answer: Answer =
            serde_json::from_slice(&body).context("cannot decode audio features")?;
        found.extend(
            answer
                .audio_features
                .into_iter()
                .flatten()
                .map(|features| (features.id.clone(), features)),
        );
    }
    Ok(found)
}
