use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use lyric_sources::{kugou, lrclib, lrcred, musixmatch, netease};
use music::{Lyrics, LyricsHit, LyricsProvider, LyricsQuery, MusicApi, Track, TrackKey};
use spotify::{AuthConfig, LibrespotClient, auth};
use youtube::YouTubeClient;
use ytmusic::YtMusic;

const LISTED: usize = 4;

struct Probe {
    source: &'static str,
    elapsed: Duration,
    hits: Vec<LyricsHit>,
    error: Option<String>,
}

fn providers() -> Vec<Arc<dyn LyricsProvider>> {
    let http = reqwest::Client::new();
    vec![
        Arc::new(spotify::SpotifyLyrics::from_env()),
        Arc::new(youtube::YouTubeLyrics::new()),
        Arc::new(lrcred::LrcRed::new(http.clone())),
        Arc::new(musixmatch::Musixmatch::new(http.clone())),
        Arc::new(lrclib::LrcLib::new(http.clone())),
        Arc::new(kugou::Kugou::new(http.clone())),
        Arc::new(netease::NetEase::new(http)),
    ]
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let Some(link) = std::env::args().nth(1) else {
        bail!("usage: lyrics-prober <spotify or youtube link, or a search query>");
    };

    let (track, provider) = resolve(&link).await?;
    let query = LyricsQuery {
        title: track.name.clone(),
        artist: track.artists.clone(),
        album: (!track.album.is_empty()).then(|| track.album.clone()),
        duration: track.duration,
        track: track.id.clone().map(|id| TrackKey { provider, id }),
    };

    println!(
        "{} — {} [{}] {} ({})",
        track.name,
        track.artists,
        track.album,
        clock(track.duration),
        track
            .id
            .as_deref()
            .map(|id| format!("{provider}:{id}"))
            .unwrap_or_else(|| "no id".to_owned()),
    );
    println!();

    let mut probes = probe(&query).await;
    probes.sort_by_key(|probe| probe.elapsed);

    println!(
        "{:<12} {:>6} {:>5}  {:>6} {:>4} {:<8} matched title / artist",
        "provider", "ms", "hits", "score", "keep", "kind"
    );
    for found in &probes {
        report(&query, found);
    }
    println!();

    let all: Vec<LyricsHit> = probes
        .iter()
        .flat_map(|found| found.hits.iter().cloned())
        .collect();
    let ranked = lyrics::rank(&query, all);
    match ranked.is_empty() {
        true => println!(
            "nothing survived ranking{}",
            match lyrics::instrumental(&query, &ranked) {
                true => ", the track reads as instrumental",
                false => "",
            }
        ),
        false => {
            println!("ranked:");
            for (place, hit) in ranked.iter().enumerate() {
                println!(
                    "  {:>2}. {:<12} {:>6} {:<8} {}",
                    place + 1,
                    hit.source,
                    lyrics::score(&query, hit),
                    kind(&hit.lyrics),
                    shape(&hit.lyrics),
                );
            }

            let mut reshaped = ranked.clone();
            lyrics::reshape(&mut reshaped);
            let winner = &reshaped[0];
            println!();
            println!(
                "winner: {} ({}, {}){}",
                winner.source,
                kind(&winner.lyrics),
                shape(&winner.lyrics),
                match winner.lyrics == ranked[0].lyrics {
                    true => String::new(),
                    false => format!(" reshaped onto {}", ranked[0].source),
                }
            );
        }
    }

    Ok(())
}

async fn resolve(link: &str) -> Result<(Track, &'static str)> {
    if let Some(id) = youtube_id(link) {
        let api = Arc::new(YtMusic::anonymous());
        let track = YouTubeClient::new(api)
            .track(&id)
            .await
            .context("cannot look the video up as a guest")?;
        return Ok((track, "youtube"));
    }

    let session = auth::restore(&AuthConfig::from_env())
        .await
        .context("cannot restore the cached session")?
        .context("no cached credentials; sign in with sonora first")?;
    let client = Arc::new(LibrespotClient::new(session));

    let track = match spotify_id(link) {
        Some(id) => client
            .track(&id)
            .await
            .context("cannot look the track up")?,
        None => client
            .search(link)
            .await
            .context("cannot search for the track")?
            .into_iter()
            .next()
            .context("nothing found for that query")?,
    };

    Ok((track, "spotify"))
}

async fn probe(query: &LyricsQuery) -> Vec<Probe> {
    let mut tasks = tokio::task::JoinSet::new();
    for provider in providers() {
        let query = query.clone();
        tasks.spawn(async move {
            let started = Instant::now();
            let found = provider.search(&query).await;
            Probe {
                source: provider.name(),
                elapsed: started.elapsed(),
                hits: found.as_ref().map(Vec::clone).unwrap_or_default(),
                error: found.err().map(|error| format!("{error:#}")),
            }
        });
    }

    let mut probes = Vec::new();
    while let Some(found) = tasks.join_next().await {
        if let Ok(found) = found {
            probes.push(found);
        }
    }
    probes
}

fn report(query: &LyricsQuery, found: &Probe) {
    let millis = found.elapsed.as_millis();
    if let Some(error) = &found.error {
        println!("{:<12} {millis:>6} {:>5}  {error}", found.source, "—");
        return;
    }
    if found.hits.is_empty() {
        println!("{:<12} {millis:>6} {:>5}", found.source, 0);
        return;
    }

    let mut scored: Vec<(i64, &LyricsHit)> = found
        .hits
        .iter()
        .map(|hit| (lyrics::score(query, hit), hit))
        .collect();
    scored.sort_by(|(left, _), (right, _)| right.cmp(left));

    for (place, (score, hit)) in scored.iter().take(LISTED).enumerate() {
        let head = match place {
            0 => format!("{:<12} {millis:>6} {:>5}", found.source, found.hits.len()),
            _ => format!("{:<12} {:>6} {:>5}", "", "", ""),
        };
        println!(
            "{head}  {score:>6} {:>4} {:<8} {} — {}{}",
            match lyrics::eligible(query, hit) {
                true => "yes",
                false => "no",
            },
            kind(&hit.lyrics),
            hit.title,
            hit.artist,
            hit.duration
                .map(|duration| format!(" ({})", clock(duration)))
                .unwrap_or_default(),
        );
    }
    if let Some(rest) = scored.len().checked_sub(LISTED).filter(|rest| *rest > 0) {
        println!("{:<12} {:>6} {:>5}  {rest:>6} more", "", "", "");
    }
}

fn kind(lyrics: &Lyrics) -> &'static str {
    match (lyrics.worded(), lyrics.synced()) {
        (true, _) => "worded",
        (false, true) => "synced",
        (false, false) => "plain",
    }
}

fn shape(lyrics: &Lyrics) -> String {
    let Lyrics::Synced { lines } = lyrics else {
        return "unsynced".to_owned();
    };
    let words: usize = lines
        .iter()
        .map(|line| line.words.as_ref().map_or(0, Vec::len))
        .sum();
    let voices = lines.iter().filter(|line| !line.voice.lead()).count();
    let secondary: usize = lines.iter().map(|line| line.secondary.len()).sum();

    format!(
        "{} lines, {words} words, {voices} background, {secondary} lanes, spans {}",
        lines.len(),
        lyrics.span().map(clock).unwrap_or_else(|| "—".to_owned()),
    )
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn spotify_id(link: &str) -> Option<String> {
    if let Some(rest) = link.strip_prefix("spotify:track:") {
        return Some(rest.to_owned());
    }
    if let Some(rest) = link.split("open.spotify.com/track/").nth(1) {
        return Some(cut(rest));
    }
    let bare = link.len() == 22 && link.chars().all(|letter| letter.is_ascii_alphanumeric());
    bare.then(|| link.to_owned())
}

fn youtube_id(link: &str) -> Option<String> {
    if let Some(rest) = link.split("youtu.be/").nth(1) {
        return Some(cut(rest));
    }
    if !link.contains("youtube.com") && !link.contains("music.youtube.com") {
        return None;
    }
    link.split("v=").nth(1).map(cut)
}

fn cut(rest: &str) -> String {
    rest.split(['?', '&', '/', '#'])
        .next()
        .unwrap_or(rest)
        .to_owned()
}
