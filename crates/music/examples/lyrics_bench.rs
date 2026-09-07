use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use music::spotify::{AuthConfig, LibrespotClient, auth};
use music::{
    Lyrics, LyricsHit, LyricsProvider, LyricsQuery, MusicApi, Track, TrackKey, binimum, kugou,
    lrclib, musixmatch, netease,
};

const OWN_TRUST: u32 = 25;
const NATIVE: &str = "Spotify";
const SOURCES: [&str; 6] = [
    NATIVE,
    "Apple Music",
    "Musixmatch",
    "LrcLib",
    "Kugou",
    "NetEase",
];

struct Measured {
    source: &'static str,
    elapsed: Duration,
    hits: Vec<LyricsHit>,
    error: Option<String>,
    best: Option<&'static str>,
    top: Option<Lyrics>,
}

struct Row {
    title: String,
    artist: String,
    measured: Vec<Measured>,
    winner: Option<&'static str>,
    won: Option<Lyrics>,
    raw: Option<Lyrics>,
    ranked: usize,
}

fn providers() -> Vec<Arc<dyn LyricsProvider>> {
    vec![
        Arc::new(binimum::Binimum::new()),
        Arc::new(musixmatch::Musixmatch::new()),
        Arc::new(lrclib::LrcLib::new()),
        Arc::new(kugou::Kugou::new()),
        Arc::new(netease::NetEase::new()),
    ]
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let sample: usize = std::env::var("SAMPLE")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40);
    let limit: u32 = std::env::var("LIMIT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2000);

    let session = auth::restore(&AuthConfig::from_env())
        .await
        .context("cannot restore the cached session")?
        .context("no cached credentials; sign in with sonora first")?;
    let client = Arc::new(LibrespotClient::new(session));
    let providers = providers();

    if let Ok(wanted) = std::env::var("FIND") {
        for track in client.search(&wanted).await?.into_iter().take(sample) {
            eprintln!("{} — {} ({:?})", track.name, track.artists, track.duration);
            inspect(&measure(&track, &providers, &client).await);
        }
        return Ok(());
    }

    let started = Instant::now();
    let saved = client.saved_tracks(limit).await?;
    eprintln!(
        "liked songs: {} fetched in {:?}",
        saved.len(),
        started.elapsed()
    );

    let only = std::env::var("TRACK").unwrap_or_default().to_lowercase();
    let saved: Vec<Track> = match only.is_empty() {
        true => saved,
        false => saved
            .into_iter()
            .filter(|track| {
                track.name.to_lowercase().contains(&only)
                    || track.artists.to_lowercase().contains(&only)
            })
            .collect(),
    };
    let picked = spread(&saved, sample);
    eprintln!("sampling {} of them\n", picked.len());

    let dump = std::env::var("DUMP").is_ok();
    let mut rows = Vec::new();
    for (index, track) in picked.iter().enumerate() {
        let row = measure(track, &providers, &client).await;
        eprintln!(
            "{:>3}/{} {} — {} → {} in {} ms",
            index + 1,
            picked.len(),
            clip(&row.title, 34),
            clip(&row.artist, 24),
            row.winner.unwrap_or("(none)"),
            wall(&row.measured).as_millis()
        );
        if dump {
            inspect(&row);
        }
        rows.push(row);
    }

    report(&rows);
    csv(&rows)?;
    Ok(())
}

async fn measure(
    track: &Track,
    providers: &[Arc<dyn LyricsProvider>],
    client: &Arc<LibrespotClient>,
) -> Row {
    let id = track.id.clone().unwrap_or_default();
    let query = LyricsQuery {
        title: track.name.clone(),
        artist: track.artists.clone(),
        album: (!track.album.is_empty()).then(|| track.album.clone()),
        duration: track.duration,
        track: Some(TrackKey {
            provider: "spotify",
            id: id.clone(),
        }),
        language: None,
    };

    let mut tasks = tokio::task::JoinSet::new();
    for provider in providers {
        let provider = provider.clone();
        let query = query.clone();
        tasks.spawn(async move {
            let started = Instant::now();
            let found = provider.search(&query).await;
            Measured {
                source: provider.name(),
                elapsed: started.elapsed(),
                hits: found.as_ref().map(Vec::clone).unwrap_or_default(),
                error: found.err().map(|error| format!("{error:#}")),
                best: None,
                top: None,
            }
        });
    }
    {
        let client = client.clone();
        let held = id.clone();
        let query = query.clone();
        tasks.spawn(async move {
            let started = Instant::now();
            let found = client.track_lyrics(&held).await;
            let elapsed = started.elapsed();
            let (hits, error) = match found {
                Ok(Some(lyrics)) if !lyrics.is_empty() => (vec![own(lyrics, &query)], None),
                Ok(_) => (Vec::new(), None),
                Err(error) => (Vec::new(), Some(format!("{error:#}"))),
            };
            Measured {
                source: NATIVE,
                elapsed,
                hits,
                error,
                best: None,
                top: None,
            }
        });
    }

    let mut measured = Vec::new();
    while let Some(found) = tasks.join_next().await {
        if let Ok(found) = found {
            measured.push(found);
        }
    }
    measured.sort_by_key(|found| found.elapsed);
    for found in &mut measured {
        let ranked = music::lyrics::rank(&query, found.hits.clone());
        found.best = ranked.first().map(|hit| quality(&hit.lyrics));
        found.top = ranked.first().map(|hit| hit.lyrics.clone());
    }

    let all: Vec<LyricsHit> = measured
        .iter()
        .flat_map(|found| found.hits.iter().cloned())
        .collect();
    let raw = music::lyrics::rank(&query, all);
    let mut ranked = raw.clone();
    music::lyrics::reshape(&mut ranked);

    Row {
        title: track.name.clone(),
        artist: track.artists.clone(),
        winner: ranked.first().map(|hit| hit.source),
        won: ranked.first().map(|hit| hit.lyrics.clone()),
        raw: raw.first().map(|hit| hit.lyrics.clone()),
        ranked: ranked.len(),
        measured,
    }
}

fn own(lyrics: Lyrics, query: &LyricsQuery) -> LyricsHit {
    LyricsHit {
        source: NATIVE,
        trust: OWN_TRUST,
        lyrics,
        instrumental: false,
        fallback: false,
        title: query.title.clone(),
        artist: query.artist.clone(),
        album: query.album.clone(),
        duration: (!query.duration.is_zero()).then_some(query.duration),
        writers: Vec::new(),
    }
}

fn quality(lyrics: &Lyrics) -> &'static str {
    match (lyrics.worded(), lyrics.synced()) {
        (true, _) => "worded",
        (false, true) => "synced",
        (false, false) => "plain",
    }
}

fn shaped(lyrics: &Lyrics) -> (usize, usize, usize) {
    let Lyrics::Synced { lines } = lyrics else {
        return (0, 0, 0);
    };
    let inline = lines.iter().filter(|line| line.text.contains('(')).count();
    let laned = lines
        .iter()
        .filter(|line| !line.secondary.is_empty())
        .count();
    let piled = lines
        .iter()
        .filter(|line| {
            line.words.as_deref().is_some_and(|words| {
                words
                    .windows(2)
                    .filter(|pair| pair[0].start == pair[1].start && pair[0].end == pair[1].end)
                    .count()
                    >= 2
            })
        })
        .count();
    (inline, laned, piled)
}

fn head(lyrics: &Lyrics, rows: usize) -> Vec<String> {
    match lyrics {
        Lyrics::Synced { lines } => lines
            .iter()
            .take(rows)
            .map(|line| {
                let sung = match line.words.as_deref() {
                    Some(words) if !words.is_empty() => format!(
                        " [{} words: {}]",
                        words.len(),
                        words
                            .iter()
                            .take(3)
                            .map(|word| format!("{:?}@{}ms", word.text, word.start.as_millis()))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                    _ => String::new(),
                };
                let lanes: String = line
                    .secondary
                    .iter()
                    .map(|lane| format!("\n            lane {:?}", lane.text))
                    .collect();
                format!("{}{sung}{lanes}", line.text)
            })
            .collect(),
        Lyrics::Plain { text, .. } => text.lines().take(rows).map(str::to_owned).collect(),
    }
}

fn inspect(row: &Row) {
    for found in &row.measured {
        let Some(top) = found.top.as_ref() else {
            continue;
        };
        println!(
            "    {} ({}) in {} ms",
            found.source,
            found.best.unwrap_or("-"),
            found.elapsed.as_millis()
        );
        for line in head(top, 4) {
            println!("        {}", clip(&line, 150));
        }
    }
    if let Some(won) = row.won.as_ref() {
        let (inline, laned, piled) = shaped(won);
        println!(
            "    DISPLAYED ({}) inline-parens={inline} lanes={laned} piled={piled}",
            row.winner.unwrap_or("-")
        );
        for line in head(won, 4) {
            println!("        {}", clip(&line, 150));
        }
    }
}

fn spread(tracks: &[Track], sample: usize) -> Vec<Track> {
    let usable: Vec<&Track> = tracks
        .iter()
        .filter(|track| {
            track
                .id
                .as_deref()
                .is_some_and(|id| !music::is_local_id(id))
        })
        .collect();
    if usable.len() <= sample {
        return usable.into_iter().cloned().collect();
    }
    let step = usable.len() as f64 / sample as f64;
    (0..sample)
        .map(|index| usable[(index as f64 * step) as usize].clone())
        .collect()
}

fn wall(measured: &[Measured]) -> Duration {
    measured
        .iter()
        .map(|found| found.elapsed)
        .max()
        .unwrap_or_default()
}

fn quantile(sorted: &[u128], part: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) as f64 * part).round() as usize;
    sorted[index]
}

fn report(rows: &[Row]) {
    println!(
        "\n## per-provider latency (ms) over {} tracks\n",
        rows.len()
    );
    println!(
        "| provider | p50 | p90 | max | usable | worded | synced | plain | winner | sole blocker |"
    );
    println!("| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |");
    for source in SOURCES {
        let mut times: Vec<u128> = Vec::new();
        let mut useful = 0usize;
        let mut wins = 0usize;
        let mut blocking: Vec<u128> = Vec::new();
        let mut kinds = [0usize; 3];
        for row in rows {
            let Some(found) = row.measured.iter().find(|found| found.source == source) else {
                continue;
            };
            times.push(found.elapsed.as_millis());
            if !found.hits.is_empty() {
                useful += 1;
            }
            match found.best {
                Some("worded") => kinds[0] += 1,
                Some("synced") => kinds[1] += 1,
                Some("plain") => kinds[2] += 1,
                _ => {}
            }
            if row.winner == Some(source) {
                wins += 1;
            }
            let others = row
                .measured
                .iter()
                .filter(|other| other.source != source)
                .map(|other| other.elapsed.as_millis())
                .max()
                .unwrap_or(0);
            blocking.push(found.elapsed.as_millis().saturating_sub(others));
        }
        if times.is_empty() {
            continue;
        }
        times.sort_unstable();
        let mean = match blocking.is_empty() {
            true => 0,
            false => blocking.iter().sum::<u128>() / blocking.len() as u128,
        };
        println!(
            "| {source} | {} | {} | {} | {useful}/{} | {} | {} | {} | {wins} | {mean} |",
            quantile(&times, 0.5),
            quantile(&times, 0.9),
            times.last().copied().unwrap_or(0),
            times.len(),
            kinds[0],
            kinds[1],
            kinds[2]
        );
    }

    let mut walls: Vec<u128> = rows
        .iter()
        .map(|row| wall(&row.measured).as_millis())
        .collect();
    let mut firsts: Vec<u128> = Vec::new();
    let mut winners: Vec<u128> = Vec::new();
    for row in rows {
        if let Some(first) = row
            .measured
            .iter()
            .filter(|found| !found.hits.is_empty())
            .map(|found| found.elapsed.as_millis())
            .min()
        {
            firsts.push(first);
        }
        if let Some(found) = row
            .winner
            .and_then(|winner| row.measured.iter().find(|found| found.source == winner))
        {
            winners.push(found.elapsed.as_millis());
        }
    }
    walls.sort_unstable();
    firsts.sort_unstable();
    winners.sort_unstable();

    println!("\n## what the listener waits for (ms)\n");
    println!("| moment | p50 | p90 | max |");
    println!("| --- | --- | --- | --- |");
    for (label, values) in [
        ("first usable sheet anywhere", &firsts),
        ("the winning provider answers", &winners),
        ("every provider has answered", &walls),
    ] {
        println!(
            "| {label} | {} | {} | {} |",
            quantile(values, 0.5),
            quantile(values, 0.9),
            values.last().copied().unwrap_or(0)
        );
    }

    let mut karaoke = 0usize;
    let mut reshaped = 0usize;
    let mut bailed = Vec::new();
    let mut inline = 0usize;
    let mut piled = 0usize;
    for row in rows {
        let Some(raw) = row.raw.as_ref().filter(|raw| raw.worded()) else {
            continue;
        };
        karaoke += 1;
        match row.won.as_ref() != Some(raw) {
            true => reshaped += 1,
            false => bailed.push(row.title.as_str()),
        }
        if let Some(won) = row.won.as_ref() {
            let (lines, _, piles) = shaped(won);
            inline += lines;
            piled += piles;
        }
    }
    println!("\n## reshaping karaoke onto the line-synced sheet\n");
    println!("| outcome | tracks |");
    println!("| --- | --- |");
    println!("| karaoke sheets displayed | {karaoke} |");
    println!("| reshaped onto the guide | {reshaped} |");
    println!("| left as the provider sent it | {} |", bailed.len());
    println!("| lines with an inline parenthetical | {inline} |");
    println!("| lines with words sharing one instant | {piled} |");
    if !bailed.is_empty() {
        println!("\nnot reshaped: {}", bailed.join(", "));
    }

    let empty = rows.iter().filter(|row| row.ranked == 0).count();
    println!(
        "\ntracks with no usable lyrics at all: {empty}/{}",
        rows.len()
    );
}

fn csv(rows: &[Row]) -> Result<()> {
    let mut out = String::from("title,artist,source,ms,hits,best,error,winner\n");
    for row in rows {
        for found in &row.measured {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                quoted(&row.title),
                quoted(&row.artist),
                found.source,
                found.elapsed.as_millis(),
                found.hits.len(),
                found.best.unwrap_or(""),
                quoted(found.error.as_deref().unwrap_or("")),
                u8::from(row.winner == Some(found.source))
            ));
        }
    }
    let path = std::env::var("OUT").unwrap_or_else(|_| "/tmp/lyrics-bench.csv".to_owned());
    std::fs::write(&path, out)?;
    eprintln!("\nrows written to {path}");
    Ok(())
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

fn clip(text: &str, width: usize) -> String {
    match text.chars().count() > width {
        true => format!("{}…", text.chars().take(width - 1).collect::<String>()),
        false => format!("{text:<width$}"),
    }
}
