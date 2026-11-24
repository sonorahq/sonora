//! The rules of a station, the radio a queue is topped up from so it never runs dry.

use std::collections::HashSet;

use anyhow::{Context as _, Result};
use music::{MusicApi, RadioSeed, Track};

/// How few tracks may be left to play before the next stretch of radio is asked for. Asking this
/// early keeps the wait for the station off the gap between two tracks.
pub const LOOKAHEAD: usize = 10;

/// How many continuations in a row may bring nothing the queue has not heard before the station
/// is dropped. An empty stretch lands with the queue still short, which asks for the next one
/// straight away, so a provider serving the same tracks over would otherwise never be let go of.
pub const DRY_LIMIT: u8 = 3;

/// How many of a collection's songs seed one stretch of its blended radio.
const BLEND_WIDTH: usize = 3;

/// How many of a collection's songs a blended radio is seeded from before it ends.
const BLEND_SEEDS: usize = 24;

/// What a blended radio's continuation starts with, so it is never mistaken for one the provider
/// handed out. The index of the next seed to use follows on its own line, then every seed, one
/// per line, so that none of them is ever played as part of the radio.
const BLEND_MARK: &str = "sonora-blend";

/// How many stretches in a row have brought nothing new once one lands with `fresh` tracks the
/// queue had not heard, `dry` being the count before it.
pub fn dry_after(dry: u8, fresh: usize) -> u8 {
    match fresh {
        0 => dry.saturating_add(1),
        _ => 0,
    }
}

/// Whether a station that came back empty `dry` times in a row is dropped.
pub fn ran_dry(dry: u8) -> bool {
    dry >= DRY_LIMIT
}

/// Whether a queue with `ahead` tracks left to play is short enough to ask for more.
pub fn runs_low(ahead: usize) -> bool {
    ahead < LOOKAHEAD
}

/// Keeps the playable tracks that are not among `heard`, the ids a queue already holds.
pub fn unheard(tracks: &mut Vec<Track>, heard: &HashSet<String>) {
    tracks.retain(|track| {
        track.playable
            && track
                .id
                .as_ref()
                .is_some_and(|id| !heard.contains(id.as_str()))
    });
}

/// The radio of the `seed` collection `id`, from its start or from the continuation `from`.
///
/// A provider that builds such a radio itself (`native`) is asked first. Any other, or one that
/// fails or answers with nothing, gets a blend of the track radios of a few of the collection's
/// songs picked at random. It opens with the first of them alone, the way a song's radio opens
/// with the song, so playback starts as soon as the collection is read and the radio follows.
/// Each later stretch blends a few more of them until the seeds run out, and none of the seeds
/// is dealt out again.
pub async fn collection_radio(
    client: &dyn MusicApi,
    seed: RadioSeed,
    id: &str,
    native: bool,
    from: Option<&str>,
) -> Result<(Vec<Track>, Option<String>)> {
    if let Some(from) = from {
        return match from.strip_prefix(BLEND_MARK) {
            Some(rest) => {
                let mut lines = rest.lines().filter(|line| !line.is_empty());
                let at = lines.next().and_then(|at| at.parse().ok()).unwrap_or(0);
                blend(client, lines.map(str::to_owned).collect(), at).await
            }
            None => client.collection_radio(seed, id, Some(from)).await,
        };
    }
    if native {
        match client.collection_radio(seed, id, None).await {
            Ok((tracks, next)) if !tracks.is_empty() => return Ok((tracks, next)),
            Ok(_) => log::warn!("station: the {seed:?} radio of {id} came back empty, blending"),
            Err(error) => log::warn!("station: no {seed:?} radio of {id}, blending: {error:#}"),
        }
    }
    let songs = match seed {
        RadioSeed::Album => client.album_tracks(id).await?,
        RadioSeed::Playlist => client.playlist_tracks(id).await?,
        RadioSeed::Artist => client.artist_tracks(id).await?,
    };
    let mut seen = HashSet::new();
    let mut songs = songs
        .into_iter()
        .filter(|song| song.playable)
        .filter(|song| song.id.as_ref().is_some_and(|id| seen.insert(id.clone())))
        .collect::<Vec<_>>();
    fastrand::shuffle(&mut songs);
    songs.truncate(BLEND_SEEDS);
    let seeds = songs
        .iter()
        .filter_map(|song| song.id.clone())
        .collect::<Vec<_>>();
    let opener = songs
        .into_iter()
        .next()
        .context("the collection has no song to seed a radio")?;
    Ok((vec![opener], Some(continuation(&seeds, 0))))
}

/// A station a queue is following: the track it grew from, the continuation its next stretch is
/// fetched with, and how many stretches in a row brought nothing new.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Station {
    seed: String,
    next: String,
    dry: u8,
}

impl Station {
    /// The station grown from `seed`, which the provider continues at `next`.
    pub fn new(seed: impl Into<String>, next: impl Into<String>) -> Self {
        Self {
            seed: seed.into(),
            next: next.into(),
            dry: 0,
        }
    }

    /// The id of the track the station grew from.
    pub fn seed(&self) -> &str {
        &self.seed
    }

    /// The continuation the next stretch is asked for with.
    pub fn next(&self) -> &str {
        &self.next
    }

    /// The station after a stretch landed with `fresh` tracks the queue had not heard and the
    /// continuation `next`. It is gone when the provider has no more of it, or when it came back
    /// empty [`DRY_LIMIT`] times in a row.
    pub fn landed(self, fresh: usize, next: Option<String>) -> Option<Self> {
        let dry = dry_after(self.dry, fresh);
        next.filter(|_| !ran_dry(dry)).map(|next| Self {
            seed: self.seed,
            next,
            dry,
        })
    }
}

/// One stretch of a blended radio: the track radios of the [`BLEND_WIDTH`] seeds from `at`, asked
/// for at once and dealt out in turn, with every seed left out. A seed whose radio fails is
/// skipped, and the stretch fails only when every one of them did. It ends the radio once no
/// seed is left after it.
async fn blend(
    client: &dyn MusicApi,
    seeds: Vec<String>,
    at: usize,
) -> Result<(Vec<Track>, Option<String>)> {
    let round = seeds
        .get(at..(at + BLEND_WIDTH).min(seeds.len()))
        .unwrap_or_default();
    let radios = futures::future::join_all(
        round
            .iter()
            .map(|seed| async move { client.track_radio(seed, None).await }),
    )
    .await;
    let mut failure = None;
    let mut lanes = Vec::new();
    for (seed, radio) in round.iter().zip(radios) {
        match radio {
            Ok((mut tracks, _)) => {
                tracks.retain(|track| !seeded(track, &seeds));
                lanes.push(tracks);
            }
            Err(error) => {
                log::warn!("station: no radio of {seed} to blend: {error:#}");
                failure.get_or_insert(error);
            }
        }
    }
    if let (true, Some(error)) = (lanes.is_empty(), failure) {
        return Err(error);
    }
    let after = at + round.len();
    let next = (after < seeds.len()).then(|| continuation(&seeds, after));
    Ok((deal(lanes), next))
}

/// The continuation of a blended radio of `seeds` whose next stretch starts at seed `at`.
fn continuation(seeds: &[String], at: usize) -> String {
    format!("{BLEND_MARK}\n{at}\n{}", seeds.join("\n"))
}

/// Whether `track` is one of the collection's songs the radio grew from.
fn seeded(track: &Track, seeds: &[String]) -> bool {
    track.id.as_ref().is_some_and(|id| seeds.contains(id))
}

/// Deals the tracks of `lanes` out in turn, first of each, then second of each, leaving out any
/// track an earlier lane already dealt.
fn deal(lanes: Vec<Vec<Track>>) -> Vec<Track> {
    let mut seen = HashSet::new();
    let mut lanes = lanes.into_iter().map(Vec::into_iter).collect::<Vec<_>>();
    let mut dealt = Vec::new();
    loop {
        let mut any = false;
        for lane in &mut lanes {
            let Some(track) = lane.next() else {
                continue;
            };
            any = true;
            if track.id.as_ref().is_none_or(|id| seen.insert(id.clone())) {
                dealt.push(track);
            }
        }
        if !any {
            return dealt;
        }
    }
}
