use std::time::Duration;

use anyhow::{Context as _, Result};
use ring::digest::{SHA256, digest};
use serde::Deserialize;

const ENDPOINT: &str = "https://sponsor.ajay.app/api/skipSegments";
const TIMEOUT: Duration = Duration::from_secs(5);
const CATEGORIES: [&str; 3] = ["music_offtopic", "sponsor", "selfpromo"];
/// Allows for rounding in the track metadata and community submissions.
const DURATION_TOLERANCE: Duration = Duration::from_secs(2);

/// A community-marked section to skip, with an exclusive end position.
#[derive(Clone, Copy, Debug)]
pub struct SkipSegment {
    pub start: Duration,
    pub end: Duration,
    pub video_duration: Duration,
}

/// Reads sponsors, self-promotion and non-music sections without changing the audio stream.
#[derive(Clone, Default)]
pub struct SponsorBlock {
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct Video {
    #[serde(rename = "videoID")]
    id: String,
    segments: Vec<Found>,
}

#[derive(Deserialize)]
struct Found {
    segment: [f64; 2],
    category: String,
    #[serde(rename = "actionType")]
    action: String,
    #[serde(default, rename = "videoDuration")]
    video_duration: f64,
}

impl SkipSegment {
    /// Unknown or different recording lengths must never cause an automatic seek.
    pub fn matches_duration(&self, duration: Duration) -> bool {
        !duration.is_zero() && self.video_duration.abs_diff(duration) <= DURATION_TOLERANCE
    }
}

impl SponsorBlock {
    pub async fn segments(&self, video: &str) -> Result<Vec<SkipSegment>> {
        let categories = serde_json::to_string(&CATEGORIES)?;
        let hash = digest(&SHA256, video.as_bytes());
        // Only the first four hex digits leave the app; select the exact video locally.
        let prefix = format!("{:02x}{:02x}", hash.as_ref()[0], hash.as_ref()[1]);
        let response = self
            .http
            .get(format!("{ENDPOINT}/{prefix}"))
            .query(&[
                ("categories", categories.as_str()),
                ("actionTypes", "[\"skip\"]"),
            ])
            .timeout(TIMEOUT)
            .send()
            .await
            .context("cannot reach sponsorblock")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        let videos: Vec<Video> = response
            .error_for_status()
            .context("sponsorblock refused the lookup")?
            .json()
            .await
            .context("cannot read the sponsorblock response")?;
        Ok(videos
            .into_iter()
            .find(|found| found.id == video)
            .map(|found| normalize(found.segments))
            .unwrap_or_default())
    }
}

/// Rejects invalid ranges or unknown recording lengths and sorts sections for playback.
fn normalize(found: Vec<Found>) -> Vec<SkipSegment> {
    let mut segments: Vec<SkipSegment> = found
        .into_iter()
        .filter_map(|found| {
            if !CATEGORIES.contains(&found.category.as_str()) || found.action != "skip" {
                return None;
            }
            let start = Duration::try_from_secs_f64(found.segment[0]).ok()?;
            let end = Duration::try_from_secs_f64(found.segment[1]).ok()?;
            let video_duration = Duration::try_from_secs_f64(found.video_duration).ok()?;
            (end > start
                && !video_duration.is_zero()
                && end <= video_duration.saturating_add(DURATION_TOLERANCE))
            .then_some(SkipSegment {
                start,
                end,
                video_duration,
            })
        })
        .collect();
    segments.sort_by_key(|segment| segment.start);
    segments
}
