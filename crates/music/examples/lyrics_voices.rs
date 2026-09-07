use std::sync::Arc;
use std::time::Duration;

use music::{Lyrics, LyricsProvider, LyricsQuery, Voice, binimum, musixmatch};

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let wanted = [
        (
            "Shallow",
            "Lady Gaga & Bradley Cooper",
            "A Star Is Born",
            216,
        ),
        ("Señorita", "Shawn Mendes & Camila Cabello", "", 190),
        ("Under Pressure", "Queen & David Bowie", "", 248),
        (
            "Summer Nights",
            "John Travolta & Olivia Newton-John",
            "",
            216,
        ),
        (
            "Ain't No Mountain High Enough",
            "Marvin Gaye & Tammi Terrell",
            "",
            151,
        ),
        ("Blinding Lights", "The Weeknd", "After Hours", 200),
        ("夜に駆ける", "YOASOBI", "THE BOOK", 261),
        ("Clair de Lune", "Claude Debussy", "", 300),
        ("Zxqvortle Plimbath", "Nobody Realish", "", 123),
    ];
    let providers: Vec<Arc<dyn LyricsProvider>> = vec![
        Arc::new(binimum::Binimum::new()),
        Arc::new(musixmatch::Musixmatch::new()),
    ];

    for (title, artist, album, seconds) in wanted {
        let query = LyricsQuery {
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: (!album.is_empty()).then(|| album.to_owned()),
            duration: Duration::from_secs(seconds),
            track: None,
            language: None,
        };
        println!("\n== {title} — {artist}");
        let mut gathered = Vec::new();
        for provider in &providers {
            match provider.search(&query).await {
                Ok(hits) if hits.is_empty() => println!("   {:<12} nothing", provider.name()),
                Ok(hits) => {
                    for hit in &hits {
                        report(
                            provider.name(),
                            &hit.lyrics,
                            &hit.title,
                            &hit.artist,
                            &hit.writers,
                        );
                    }
                    gathered.extend(hits);
                }
                Err(error) => println!("   {:<12} failed: {error:#}", provider.name()),
            }
        }
        let mut ranked = music::lyrics::rank(&query, gathered);
        music::lyrics::reshape(&mut ranked);
        match ranked.first() {
            Some(best) => println!(
                "   -> winner {} ({} kept, {} counter lines, {} lanes)",
                best.source,
                ranked.len(),
                counted(&best.lyrics).0,
                counted(&best.lyrics).1
            ),
            None => println!("   -> winner none"),
        }
    }
}

fn counted(lyrics: &Lyrics) -> (usize, usize) {
    let Lyrics::Synced { lines } = lyrics else {
        return (0, 0);
    };
    (
        lines
            .iter()
            .filter(|line| matches!(line.voice, Voice::Counter))
            .count(),
        lines.iter().map(|line| line.secondary.len()).sum(),
    )
}

fn report(source: &str, lyrics: &Lyrics, title: &str, artist: &str, writers: &[String]) {
    let Lyrics::Synced { lines } = lyrics else {
        println!("   {source:<12} plain ({title} — {artist})");
        return;
    };
    let worded = lines.iter().filter(|line| line.words.is_some()).count();
    let lanes: usize = lines.iter().map(|line| line.secondary.len()).sum();
    let counter = lines
        .iter()
        .filter(|line| matches!(line.voice, Voice::Counter))
        .count();
    println!(
        "   {source:<12} {} lines, {worded} worded, {lanes} lanes, {counter} counter-voice, writers {} | {title} — {artist}",
        lines.len(),
        writers.len()
    );
    let mut shown = 0;
    let mut sides = Vec::new();
    for line in lines.iter() {
        let side = match line.voice {
            Voice::Lead => "L",
            Voice::Counter => "R",
        };
        sides.push(side);
        if shown < 3 && line.words.is_some() {
            let words: Vec<String> = line
                .words
                .iter()
                .flatten()
                .take(3)
                .map(|word| {
                    format!(
                        "{:?}@{}..{}",
                        word.text,
                        word.start.as_millis(),
                        word.end.as_millis()
                    )
                })
                .collect();
            println!("      {side} {:?} {}", line.text, words.join(" "));
            shown += 1;
        }
        if let Some(lane) = line.secondary.first().filter(|_| shown < 4) {
            println!("        lane {:?} @{}", lane.text, lane.start.as_millis());
            shown += 1;
        }
    }
    println!("      sides: {}", sides.join(""));
    let broken: Vec<String> = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            let words = line.words.as_ref()?;
            let out_of_order = words.windows(2).any(|pair| pair[1].start < pair[0].start);
            let inverted = words.iter().any(|word| word.end < word.start);
            let joined: String = words.iter().map(|word| word.text.as_str()).collect();
            let mismatched = joined.split_whitespace().collect::<Vec<_>>()
                != line.text.split_whitespace().collect::<Vec<_>>();
            (out_of_order || inverted || mismatched).then(|| {
                format!("line {index}: order={out_of_order} inverted={inverted} text={mismatched}")
            })
        })
        .collect();
    match broken.is_empty() {
        true => println!("      checks: ok"),
        false => println!("      checks: {}", broken.join("; ")),
    }
}
