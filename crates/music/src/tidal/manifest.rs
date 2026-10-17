//! What a Tidal playback answer says about where a track's bytes are.
//!
//! The answer carries a base64 manifest in one of two shapes. Lossless and below come as a small
//! JSON document naming one file. Hi-res comes as a DASH MPD: an init segment, then media
//! segments numbered from a template and timed by a `SegmentTimeline`, which is what lets a seek
//! go straight to the segment that covers it.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

/// The manifest type of a track that is one file.
const BTS: &str = "application/vnd.tidal.bts";
/// The manifest type of a track served as DASH segments.
const DASH: &str = "application/dash+xml";

/// The most segments one track may expand to. A long track is a few hundred, so this only
/// bounds what a wrong `r` attribute can cost.
const SEGMENT_CEILING: usize = 20_000;

/// Where a track's bytes are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manifest {
    /// One file, read as it arrives.
    File { url: String, codec: String },
    /// An init segment and the media segments behind it, in order, with where each one starts
    /// in the music.
    Segments {
        init: String,
        media: Vec<String>,
        starts: Vec<Duration>,
        codec: String,
    },
}

/// The JSON body of a `BTS` manifest.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bts {
    #[serde(default)]
    codecs: String,
    #[serde(default)]
    encryption_type: String,
    #[serde(default)]
    urls: Vec<String>,
}

impl Manifest {
    /// Reads the manifest a playback answer carries, given its declared type and its base64
    /// body. Refuses an encrypted track rather than handing ciphertext to a decoder.
    pub fn read(mime: &str, encoded: &str) -> Result<Self> {
        let bytes = STANDARD
            .decode(encoded.trim())
            .context("cannot decode the tidal manifest")?;
        match mime {
            BTS => {
                let bts: Bts =
                    serde_json::from_slice(&bytes).context("cannot read the tidal manifest")?;
                if !bts.encryption_type.is_empty() && bts.encryption_type != "NONE" {
                    bail!("the track is encrypted as {}", bts.encryption_type);
                }
                let url = bts
                    .urls
                    .into_iter()
                    .next()
                    .context("the tidal manifest names no file")?;
                Ok(Self::File {
                    url,
                    codec: bts.codecs,
                })
            }
            DASH => {
                let text = std::str::from_utf8(&bytes).context("the tidal manifest is not text")?;
                dash(text)
            }
            other => bail!("tidal answered with a manifest of type {other}"),
        }
    }

    pub fn codec(&self) -> &str {
        match self {
            Self::File { codec, .. } | Self::Segments { codec, .. } => codec,
        }
    }
}

/// Expands an MPD's one audio representation into segment urls and start times.
fn dash(text: &str) -> Result<Manifest> {
    let document = roxmltree::Document::parse(text).context("cannot parse the tidal mpd")?;
    if document
        .descendants()
        .any(|node| node.has_tag_name("ContentProtection"))
    {
        bail!("the track is protected");
    }
    let representation = document
        .descendants()
        .find(|node| node.has_tag_name("Representation"))
        .context("the tidal mpd has no representation")?;
    let codec = representation.attribute("codecs").unwrap_or_default();
    let template = representation
        .descendants()
        .find(|node| node.has_tag_name("SegmentTemplate"))
        .or_else(|| {
            document
                .descendants()
                .find(|node| node.has_tag_name("SegmentTemplate"))
        })
        .context("the tidal mpd has no segment template")?;
    let init = template
        .attribute("initialization")
        .context("the tidal mpd names no init segment")?;
    let media = template
        .attribute("media")
        .context("the tidal mpd names no media segments")?;
    let first: u64 = template
        .attribute("startNumber")
        .and_then(|number| number.parse().ok())
        .unwrap_or(1);
    let timescale: u64 = template
        .attribute("timescale")
        .and_then(|ticks| ticks.parse().ok())
        .filter(|ticks| *ticks > 0)
        .unwrap_or(1);

    let mut starts = Vec::new();
    let mut ticks = 0u64;
    for step in template.descendants().filter(|node| node.has_tag_name("S")) {
        if let Some(at) = step.attribute("t").and_then(|at| at.parse().ok()) {
            ticks = at;
        }
        let length: u64 = step
            .attribute("d")
            .and_then(|length| length.parse().ok())
            .context("a tidal segment has no duration")?;
        let repeats: usize = step
            .attribute("r")
            .and_then(|repeats| repeats.parse().ok())
            .unwrap_or(0);
        for _ in 0..=repeats {
            if starts.len() >= SEGMENT_CEILING {
                bail!("the tidal mpd lists more than {SEGMENT_CEILING} segments");
            }
            starts.push(Duration::from_secs_f64(ticks as f64 / timescale as f64));
            ticks += length;
        }
    }
    if starts.is_empty() {
        bail!("the tidal mpd lists no segments");
    }
    let media = (first..)
        .take(starts.len())
        .map(|number| media.replace("$Number$", &number.to_string()))
        .collect();
    Ok(Manifest::Segments {
        init: init.to_owned(),
        media,
        starts,
        codec: codec.to_owned(),
    })
}
