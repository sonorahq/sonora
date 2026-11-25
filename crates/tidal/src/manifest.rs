//! What a Tidal playback answer says about where a track's bytes are.
//!
//! The answer carries a base64 manifest in one of two shapes. Lossless and below come as a small
//! JSON document naming one file. Hi-res comes as a DASH MPD: an init segment, then media
//! segments named by a template and timed by a `SegmentTimeline` or one fixed duration, which is
//! what lets a seek go straight to the segment that covers it.

use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use reqwest::Url;
use roxmltree::Node;
use serde::Deserialize;

/// The manifest type of a track that is one file.
const BTS: &str = "application/vnd.tidal.bts";
/// The manifest type of a track served as DASH segments.
const DASH: &str = "application/dash+xml";

/// The codec names Dolby Atmos and Sony 360 Reality Audio arrive under, which no decoder here
/// plays.
const SPATIAL: &[&str] = &["ec-3", "eac3", "ac-3", "ac3", "ac-4", "ac4", "mha1", "mhm1"];

/// The most segments one track may expand to. A long track is a few hundred, so this only
/// bounds what a wrong `r` or `duration` attribute can cost.
const SEGMENT_CEILING: usize = 20_000;

/// The widest `%0Nd` a segment template may pad a number to. A `u64` has at most 20 digits, so
/// anything wider is padding alone, and an absurd width would make the url too big to allocate.
const WIDEST: usize = 20;

/// What the identifiers of a segment template stand for, for one segment. The init segment has
/// no number or time.
#[derive(Clone, Copy)]
struct Fields<'a> {
    id: Option<&'a str>,
    bandwidth: Option<u64>,
    number: Option<u64>,
    time: Option<u64>,
}

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
    /// body. Refuses an encrypted track rather than handing ciphertext to a decoder, and a
    /// spatial mix no decoder here can play.
    pub fn read(mime: &str, encoded: &str) -> Result<Self> {
        let manifest = Self::parse(mime, encoded)?;
        let codec = manifest.codec().to_ascii_lowercase();
        if SPATIAL.iter().any(|spatial| codec.starts_with(spatial)) {
            bail!("the track is coded as {codec}, which sonora cannot decode");
        }
        Ok(manifest)
    }

    fn parse(mime: &str, encoded: &str) -> Result<Self> {
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
    let adaptation = representation
        .parent_element()
        .filter(|node| node.has_tag_name("AdaptationSet"));
    let codec = representation
        .attribute("codecs")
        .or_else(|| adaptation.and_then(|set| set.attribute("codecs")))
        .unwrap_or_default();
    let template = representation
        .descendants()
        .find(|node| node.has_tag_name("SegmentTemplate"))
        .or_else(|| adaptation.and_then(|set| child(set, "SegmentTemplate")))
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
    let first = number(template, "startNumber").unwrap_or(1);
    let timescale = number(template, "timescale")
        .filter(|ticks| *ticks > 0)
        .unwrap_or(1);
    let offset = number(template, "presentationTimeOffset").unwrap_or(0);
    let end = length(representation)
        .map(|seconds| offset.saturating_add((seconds * timescale as f64).round() as u64));

    let ticks = match child(template, "SegmentTimeline") {
        Some(timeline) => listed(timeline, end)?,
        None => even(template, offset, end)?,
    };
    if ticks.is_empty() {
        bail!("the tidal mpd lists no segments");
    }
    let starts = ticks
        .iter()
        .map(|at| Duration::from_secs_f64(at.saturating_sub(offset) as f64 / timescale as f64))
        .collect();

    let base = base_url(representation)?;
    let fields = Fields {
        id: representation.attribute("id"),
        bandwidth: number(representation, "bandwidth"),
        number: None,
        time: None,
    };
    let init = resolve(base.as_ref(), &expand(init, fields)?)?;
    let media = ticks
        .iter()
        .zip(first..)
        .map(|(&time, number)| {
            let fields = Fields {
                number: Some(number),
                time: Some(time),
                ..fields
            };
            resolve(base.as_ref(), &expand(media, fields)?)
        })
        .collect::<Result<_>>()?;
    Ok(Manifest::Segments {
        init,
        media,
        starts,
        codec: codec.to_owned(),
    })
}

/// Where every segment a `SegmentTimeline` lists starts, in ticks. A negative `r` repeats its
/// segment up to where the next one starts, or up to `end` for the last.
fn listed(timeline: Node, end: Option<u64>) -> Result<Vec<u64>> {
    let steps: Vec<_> = timeline
        .children()
        .filter(|node| node.has_tag_name("S"))
        .collect();
    let mut ticks = Vec::new();
    let mut at = 0u64;
    for (index, step) in steps.iter().enumerate() {
        if let Some(start) = number(*step, "t") {
            at = start;
        }
        let length = number(*step, "d")
            .filter(|length| *length > 0)
            .context("a tidal segment has no duration")?;
        let repeats: i64 = step
            .attribute("r")
            .and_then(|repeats| repeats.parse().ok())
            .unwrap_or(0);
        let count = match u64::try_from(repeats) {
            Ok(repeats) => repeats.saturating_add(1),
            Err(_) => steps
                .get(index + 1)
                .and_then(|next| number(*next, "t"))
                .or(end)
                .context("a tidal segment repeats with nothing to say where it stops")?
                .saturating_sub(at)
                .div_ceil(length),
        };
        for _ in 0..count {
            if ticks.len() >= SEGMENT_CEILING {
                bail!("the tidal mpd lists more than {SEGMENT_CEILING} segments");
            }
            ticks.push(at);
            at = at.saturating_add(length);
        }
    }
    Ok(ticks)
}

/// Where every segment of a template that gives one `duration` for all of them starts, in
/// ticks, up to `end`.
fn even(template: Node, offset: u64, end: Option<u64>) -> Result<Vec<u64>> {
    let length = number(template, "duration")
        .filter(|length| *length > 0)
        .context("the tidal mpd neither lists its segments nor says how long they are")?;
    let end = end.context("the tidal mpd does not say how long the track is")?;
    let count = end.saturating_sub(offset).div_ceil(length);
    if count > SEGMENT_CEILING as u64 {
        bail!("the tidal mpd lists more than {SEGMENT_CEILING} segments");
    }
    Ok((0..count)
        .map(|index| offset.saturating_add(index.saturating_mul(length)))
        .collect())
}

/// How long the period holding `representation` runs, in seconds, from the period's own
/// `duration` or else the whole presentation's.
fn length(representation: Node) -> Option<f64> {
    representation
        .ancestors()
        .find(|node| node.has_tag_name("Period"))
        .and_then(|period| period.attribute("duration"))
        .or_else(|| {
            representation
                .document()
                .root_element()
                .attribute("mediaPresentationDuration")
        })
        .and_then(seconds)
}

/// The url a representation's relative urls resolve against: every `BaseURL` from the MPD
/// down to the representation, each resolved against the one above it. `None` when there is
/// none.
fn base_url(representation: Node) -> Result<Option<Url>> {
    let chain: Vec<_> = representation
        .ancestors()
        .filter(Node::is_element)
        .collect();
    let mut base: Option<Url> = None;
    for node in chain.into_iter().rev() {
        let Some(text) = child(node, "BaseURL")
            .and_then(|base| base.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let joined = match &base {
            Some(base) => base.join(text),
            None => Url::parse(text),
        };
        base = Some(joined.with_context(|| format!("cannot read the tidal base url {text}"))?);
    }
    Ok(base)
}

/// A url as it can be asked for: unchanged when it is absolute, so a signed query is sent
/// exactly as Tidal wrote it, and joined to `base` when it is not.
fn resolve(base: Option<&Url>, url: &str) -> Result<String> {
    if Url::parse(url).is_ok() {
        return Ok(url.to_owned());
    }
    let base =
        base.with_context(|| format!("the tidal mpd names the relative url {url} and no base"))?;
    let joined = base
        .join(url)
        .with_context(|| format!("cannot resolve the tidal url {url}"))?;
    Ok(joined.into())
}

/// Fills in the `$…$` identifiers of a segment template. A numeric one may carry a `%0Nd`
/// width, and `$$` stands for a dollar sign.
fn expand(template: &str, fields: Fields) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('$') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = after
            .find('$')
            .context("a tidal segment template leaves a `$` open")?;
        let tag = &after[..close];
        rest = &after[close + 1..];
        let (name, format) = match tag.split_once('%') {
            Some((name, format)) => (name, Some(format)),
            None => (tag, None),
        };
        let value = match name {
            "" => {
                out.push('$');
                continue;
            }
            "RepresentationID" => {
                out.push_str(
                    fields
                        .id
                        .context("the tidal mpd names no representation id")?,
                );
                continue;
            }
            "Number" => fields.number,
            "Time" => fields.time,
            "Bandwidth" => fields.bandwidth,
            other => bail!("a tidal segment template asks for ${other}$"),
        }
        .with_context(|| {
            format!("a tidal segment template asks for ${name}$ where there is none")
        })?;
        let width: usize = match format {
            None => 0,
            Some(format) => format
                .strip_suffix('d')
                .and_then(|width| width.parse().ok())
                .filter(|width| *width <= WIDEST)
                .with_context(|| format!("a tidal segment template formats ${tag}$"))?,
        };
        write!(out, "{value:0width$}").ok();
    }
    out.push_str(rest);
    Ok(out)
}

/// Reads an ISO 8601 duration such as `PT3M25.4S` into seconds. Years and months have no fixed
/// length, so one that is not zero reads as `None`.
fn seconds(text: &str) -> Option<f64> {
    let text = text.trim().strip_prefix('P')?;
    let (date, time) = text.split_once('T').unwrap_or((text, ""));
    let date = span(
        date,
        &[('Y', 0.0), ('M', 0.0), ('W', 604_800.0), ('D', 86_400.0)],
    )?;
    let time = span(time, &[('H', 3_600.0), ('M', 60.0), ('S', 1.0)])?;
    Some(date + time)
}

/// Adds up the number and unit pairs of one half of a duration, each unit worth its scale in
/// seconds. A unit it does not know, or a non-zero amount of one worth nothing, reads as `None`.
fn span(text: &str, units: &[(char, f64)]) -> Option<f64> {
    let mut total = 0.0;
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find(|c: char| c.is_ascii_alphabetic())?;
        let amount: f64 = rest[..end].parse().ok()?;
        let unit = rest[end..].chars().next()?;
        let scale = units
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, scale)| *scale)?;
        if scale == 0.0 && amount != 0.0 {
            return None;
        }
        total += amount * scale;
        rest = &rest[end + unit.len_utf8()..];
    }
    Some(total)
}

/// The first child of `node` named `name`.
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|child| child.has_tag_name(name))
}

/// An attribute of `node` read as a whole number.
fn number(node: Node, name: &str) -> Option<u64> {
    node.attribute(name)?.trim().parse().ok()
}
