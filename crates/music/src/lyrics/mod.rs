mod japanese;
pub mod lrc;
pub(crate) mod romanize;
mod shape;
pub(crate) mod sheet;
pub(crate) mod ttml;

use std::collections::HashSet;
use std::time::Duration;

use crate::{Lyrics, LyricsHit, LyricsLine, LyricsQuery, LyricsWord};

const CLOSE_ENOUGH: u64 = 3;
const WAY_OFF: u64 = 10;
const TITLE: u32 = 40;
const ARTIST: u32 = 30;
const ALBUM: u32 = 15;
const SYNCED: u32 = 200;
const WORDED: u32 = 400;
const DRIFTED: u32 = 50;
const TRUNCATED: u32 = 500;
const TRUSTED: u32 = 25;

pub fn rank(query: &LyricsQuery, hits: Vec<LyricsHit>) -> Vec<LyricsHit> {
    let declared = instrumental(query, &hits);
    let kept = hits
        .into_iter()
        .filter(|hit| eligible(query, hit))
        .collect();
    let mut scored: Vec<(i64, LyricsHit)> = preferred(kept, declared)
        .into_iter()
        .map(|hit| (score(query, &hit), hit))
        .collect();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| left.source.cmp(right.source))
            .then_with(|| left.title.cmp(&right.title))
    });

    let mut seen = HashSet::new();
    scored
        .into_iter()
        .map(|(_, hit)| hit)
        .filter(|hit| seen.insert(fingerprint(&hit.lyrics)))
        .collect()
}

fn preferred(hits: Vec<LyricsHit>, instrumental: bool) -> Vec<LyricsHit> {
    match instrumental || hits.iter().any(|hit| !hit.fallback) {
        true => hits.into_iter().filter(|hit| !hit.fallback).collect(),
        false => hits,
    }
}

pub fn reshape(hits: &mut [LyricsHit]) {
    let Some(guide) = hits
        .iter()
        .find(|hit| hit.lyrics.synced() && !hit.lyrics.worded())
        .map(|hit| hit.lyrics.clone())
    else {
        return;
    };
    for hit in hits
        .iter_mut()
        .filter(|hit| hit.lyrics.worded() && hit.trust < TRUSTED && !layered(&hit.lyrics))
    {
        if let Some(conformed) = shape::conform(&hit.lyrics, &guide) {
            hit.lyrics = conformed;
        }
    }
}

fn layered(lyrics: &Lyrics) -> bool {
    let Lyrics::Synced { lines } = lyrics else {
        return false;
    };
    lines
        .iter()
        .any(|line| !line.secondary.is_empty() || !line.voice.lead())
}

pub fn eligible(query: &LyricsQuery, hit: &LyricsHit) -> bool {
    matched(query, hit) && !hit.lyrics.is_empty() && !low_quality(&hit.lyrics)
}

pub fn instrumental(query: &LyricsQuery, hits: &[LyricsHit]) -> bool {
    let matching = || hits.iter().filter(|hit| matched(query, hit));
    matching().any(|hit| hit.instrumental)
        && !matching().any(|hit| !hit.fallback && !hit.lyrics.is_empty())
}

fn matched(query: &LyricsQuery, hit: &LyricsHit) -> bool {
    if !alike(&hit.title, &query.title) || !artists_alike(&hit.artist, &query.artist) {
        return false;
    }
    hit.duration.is_none_or(|duration| {
        query.duration.is_zero() || duration.as_secs().abs_diff(query.duration.as_secs()) <= WAY_OFF
    })
}

pub fn score(query: &LyricsQuery, hit: &LyricsHit) -> i64 {
    let mut score: i64 = i64::from(hit.trust);
    if let Some(duration) = hit.duration {
        let drift = duration.as_secs().abs_diff(query.duration.as_secs());
        if drift <= CLOSE_ENOUGH {
            score += i64::from(100 - (drift as u32) * 10);
        } else if drift > WAY_OFF {
            score -= i64::from(DRIFTED);
        }
    }
    if alike(&hit.title, &query.title) {
        score += i64::from(TITLE);
    }
    if artists_alike(&hit.artist, &query.artist) {
        score += i64::from(ARTIST);
    }
    if let Some(album) = &query.album
        && let Some(named) = &hit.album
        && alike(named, album)
    {
        score += i64::from(ALBUM);
    }
    if hit.lyrics.synced() {
        score += i64::from(SYNCED);
    }
    if hit.lyrics.worded() {
        score += i64::from(WORDED);
    }
    if truncated(&hit.lyrics, query.duration) {
        score -= i64::from(TRUNCATED);
    }
    score
}

fn truncated(lyrics: &Lyrics, duration: Duration) -> bool {
    let Some(span) = lyrics.span() else {
        return false;
    };
    !duration.is_zero() && span.as_secs_f64() < duration.as_secs_f64() * 0.6
}

fn low_quality(lyrics: &Lyrics) -> bool {
    let Lyrics::Synced { lines } = lyrics else {
        return false;
    };
    let texts = lines.iter().flat_map(|line| {
        std::iter::once(line.text.as_str()).chain(
            line.secondary
                .iter()
                .map(|secondary| secondary.text.as_str()),
        )
    });
    let (total, noisy) = texts.fold((0usize, 0usize), |(total, noisy), text| {
        (total + 1, noisy + usize::from(stretched_shout(text)))
    });
    noisy >= 3 && noisy.saturating_mul(6) >= total
}

fn stretched_shout(text: &str) -> bool {
    let mut uppercase = false;
    let mut lowercase = false;
    let mut previous = None;
    let mut run = 0usize;
    let mut longest = 0usize;
    for letter in text.chars().filter(|letter| letter.is_alphabetic()) {
        uppercase |= letter.is_uppercase();
        lowercase |= letter.is_lowercase();
        let folded = letter.to_lowercase().next().unwrap_or(letter);
        if previous == Some(folded) {
            run += 1;
        } else {
            previous = Some(folded);
            run = 1;
        }
        longest = longest.max(run);
    }
    uppercase && !lowercase && longest >= 3
}

fn fingerprint(lyrics: &Lyrics) -> String {
    let trim = |text: &str| {
        text.chars()
            .filter(|letter| letter.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    match lyrics {
        Lyrics::Plain { text, .. } => format!("plain:{}", trim(text)),
        Lyrics::Synced { lines } => {
            let worded = lines.iter().any(LyricsLine::worded);
            let text: String = lines
                .iter()
                .flat_map(|line| {
                    std::iter::once(line.text.as_str()).chain(
                        line.secondary
                            .iter()
                            .map(|secondary| secondary.text.as_str()),
                    )
                })
                .map(trim)
                .collect();
            format!("synced:{worded}:{text}")
        }
    }
}

fn alike(left: &str, right: &str) -> bool {
    let (left, right) = (undecorated(left), undecorated(right));
    if left.is_empty() || right.is_empty() {
        return false;
    }
    if left == right {
        return true;
    }
    let (short, long) = match left.len() <= right.len() {
        true => (&left, &right),
        false => (&right, &left),
    };
    long.contains(short.as_str()) && short.len() * 2 >= long.len()
}

fn artists_alike(left: &str, right: &str) -> bool {
    alike(left, right)
        || artist_names(left).any(|left| artist_names(right).any(|right| alike(left, right)))
}

fn artist_names(artists: &str) -> impl Iterator<Item = &str> {
    artists
        .split([',', '&', ';'])
        .map(str::trim)
        .filter(|artist| !artist.is_empty())
}

pub(super) fn undecorated(text: &str) -> String {
    let text = text.split(" - ").next().unwrap_or(text);
    let mut depth = 0usize;
    text.chars()
        .filter(|letter| match letter {
            '(' | '[' => {
                depth += 1;
                false
            }
            ')' | ']' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .filter(|letter| letter.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn active(lines: &[LyricsLine], at: Duration) -> Option<usize> {
    lines.iter().rposition(|line| line.start <= at)
}

pub fn active_word(words: &[LyricsWord], at: Duration) -> Option<usize> {
    words
        .iter()
        .rposition(|word| word.start <= at)
        .filter(|index| at < words[*index].end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lyrics, Voice};

    fn hit(title: &str, artist: &str, seconds: u64, synced: bool) -> LyricsHit {
        LyricsHit {
            source: "test",
            trust: 0,
            lyrics: match synced {
                true => Lyrics::Synced {
                    lines: vec![line(0, seconds.saturating_sub(2), title)].into(),
                },
                false => Lyrics::plain(format!("la {title}")),
            },
            instrumental: false,
            fallback: false,
            title: title.to_owned(),
            artist: artist.to_owned(),
            album: None,
            duration: Some(Duration::from_secs(seconds)),
            writers: Vec::new(),
        }
    }

    fn line(start: u64, end: u64, text: &str) -> LyricsLine {
        LyricsLine {
            start: Duration::from_secs(start),
            end: Some(Duration::from_secs(end)),
            text: text.to_owned(),
            romanized: None,
            words: None,
            secondary: Vec::new(),
            voice: Voice::Lead,
        }
    }

    fn query() -> LyricsQuery {
        LyricsQuery {
            title: "Jaded".to_owned(),
            artist: "Spiritbox".to_owned(),
            album: None,
            duration: Duration::from_secs(263),
            track: None,
            language: None,
        }
    }

    #[test]
    fn the_closest_duration_wins() {
        let hits = vec![
            hit("Jaded", "Spiritbox", 200, true),
            hit("Jaded", "Spiritbox", 263, false),
        ];
        let ranked = rank(&query(), hits);
        assert_eq!(ranked[0].duration, Some(Duration::from_secs(263)));
    }

    #[test]
    fn synced_breaks_a_tie() {
        let hits = vec![
            hit("Jaded", "Spiritbox", 263, false),
            hit("Jaded", "Spiritbox", 263, true),
        ];
        let ranked = rank(&query(), hits);
        assert!(ranked[0].lyrics.synced());
    }

    #[test]
    fn a_wrong_track_falls_behind() {
        let hits = vec![
            hit("Something Else", "Nobody", 263, true),
            hit("Jaded", "Spiritbox", 261, false),
        ];
        let ranked = rank(&query(), hits);
        assert_eq!(ranked[0].title, "Jaded");
        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn karaoke_cannot_rescue_the_wrong_artist() {
        let mut wrong = hit("Jaded", "Aerosmith", 263, true);
        let Lyrics::Synced { lines } = &wrong.lyrics else {
            unreachable!()
        };
        let mut lines = lines.to_vec();
        lines[0].words = Some(vec![LyricsWord {
            start: Duration::ZERO,
            end: Duration::from_secs(1),
            text: "wrong song".to_owned(),
        }]);
        wrong.lyrics = Lyrics::Synced {
            lines: lines.into(),
        };

        let ranked = rank(&query(), vec![wrong, hit("Jaded", "Spiritbox", 263, false)]);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].artist, "Spiritbox");
    }

    #[test]
    fn noisy_karaoke_cannot_beat_clean_lyrics() {
        let mut noisy = hit("In Waves", "Trivium", 302, true);
        let mut lines: Vec<LyricsLine> = (0..12)
            .map(|index| line(index, index + 1, "Pulling everyone down with me"))
            .collect();
        for text in ["IN WAVESSSSS!", "PERPETUALLYYY!!!", "AHHHHHHH!!!!!"] {
            let mut shouted = line(lines.len() as u64, lines.len() as u64 + 1, text);
            shouted.words = Some(vec![LyricsWord {
                start: shouted.start,
                end: shouted.end.expect("the test line has an end"),
                text: text.to_owned(),
            }]);
            lines.push(shouted);
        }
        noisy.lyrics = Lyrics::Synced {
            lines: lines.into(),
        };

        let query = LyricsQuery {
            title: "In Waves".to_owned(),
            artist: "Trivium".to_owned(),
            album: Some("In Waves".to_owned()),
            duration: Duration::from_secs(302),
            track: None,
            language: None,
        };
        let ranked = rank(&query, vec![noisy, hit("In Waves", "Trivium", 302, true)]);

        assert_eq!(ranked.len(), 1);
        assert!(!low_quality(&ranked[0].lyrics));
    }

    #[test]
    fn one_stylized_shout_does_not_reject_an_otherwise_clean_sheet() {
        let mut lines: Vec<LyricsLine> = (0..12)
            .map(|index| line(index, index + 1, "ordinary line"))
            .collect();
        lines.push(line(12, 13, "NOOO!!!"));

        assert!(!low_quality(&Lyrics::Synced {
            lines: lines.into()
        }));
    }

    #[test]
    fn a_same_length_song_by_the_same_artist_is_still_rejected() {
        let query = LyricsQuery {
            title: "Versailles".to_owned(),
            artist: "Pinback".to_owned(),
            album: Some("Nautical Antiques".to_owned()),
            duration: Duration::from_secs(213),
            track: None,
            language: None,
        };

        let ranked = rank(
            &query,
            vec![
                hit("Loro", "Pinback", 214, true),
                hit("Versailles", "Pinback", 213, false),
            ],
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].title, "Versailles");
    }

    #[test]
    fn a_different_recording_is_not_treated_as_the_same_track() {
        let ranked = rank(
            &query(),
            vec![
                hit("Jaded", "Spiritbox", 220, true),
                hit("Jaded", "Spiritbox", 263, false),
            ],
        );

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].duration, Some(Duration::from_secs(263)));
    }

    #[test]
    fn one_shared_artist_is_enough_for_a_collaboration() {
        let mut query = query();
        query.artist = "Spiritbox, Megan Thee Stallion".to_owned();

        let ranked = rank(&query, vec![hit("Jaded", "Spiritbox", 263, false)]);

        assert_eq!(ranked.len(), 1);
    }

    #[test]
    fn worded_beats_merely_synced() {
        let mut worded = hit("Jaded", "Spiritbox", 263, true);
        let Lyrics::Synced { lines } = &worded.lyrics else {
            unreachable!()
        };
        let mut lines = lines.to_vec();
        lines[0].text = "another take".to_owned();
        lines[0].words = Some(vec![LyricsWord {
            start: Duration::ZERO,
            end: Duration::from_secs(1),
            text: "another take".to_owned(),
        }]);
        worded.lyrics = Lyrics::Synced {
            lines: lines.into(),
        };

        let ranked = rank(&query(), vec![hit("Jaded", "Spiritbox", 263, true), worded]);
        assert!(ranked[0].lyrics.worded());
    }

    #[test]
    fn a_matching_album_pulls_ahead() {
        let mut named = hit("Jaded", "Spiritbox", 263, true);
        named.album = Some("Eternal Blue".to_owned());
        let mut plain = hit("Jaded", "Spiritbox", 263, true);
        plain.lyrics = Lyrics::Synced {
            lines: vec![line(0, 261, "a different upload")].into(),
        };

        let mut query = query();
        query.album = Some("Eternal Blue".to_owned());
        let ranked = rank(&query, vec![plain, named]);
        assert_eq!(ranked[0].album.as_deref(), Some("Eternal Blue"));
    }

    #[test]
    fn a_truncated_sync_falls_behind() {
        let mut cut = hit("Jaded", "Spiritbox", 263, true);
        cut.lyrics = Lyrics::Synced {
            lines: vec![line(0, 40, "stops far too early")].into(),
        };

        let ranked = rank(&query(), vec![cut, hit("Jaded", "Spiritbox", 263, true)]);
        assert!(ranked[0].lyrics.span() > Some(Duration::from_secs(200)));
    }

    #[test]
    fn equal_hits_keep_a_stable_order() {
        let mut left = hit("Jaded", "Spiritbox", 263, true);
        left.source = "Beta";
        let mut right = left.clone();
        right.source = "Alpha";
        right.lyrics = Lyrics::Synced {
            lines: vec![line(0, 261, "a different upload")].into(),
        };

        let ranked = rank(&query(), vec![left.clone(), right.clone()]);
        let reversed = rank(&query(), vec![right, left]);
        assert_eq!(ranked[0].source, "Alpha");
        assert_eq!(reversed[0].source, "Alpha");
    }

    #[test]
    fn twin_uploads_collapse_into_one() {
        let hits = vec![
            hit("Jaded", "Spiritbox", 263, true),
            hit("Jaded", "Spiritbox", 263, true),
        ];
        assert_eq!(rank(&query(), hits).len(), 1);
    }

    #[test]
    fn trust_settles_an_otherwise_even_match() {
        let mut direct = hit("Jaded", "Spiritbox", 263, true);
        direct.trust = 25;
        direct.lyrics = Lyrics::Synced {
            lines: vec![line(0, 261, "a different upload")].into(),
        };
        direct.source = "Direct";

        let ranked = rank(&query(), vec![hit("Jaded", "Spiritbox", 263, true), direct]);
        assert_eq!(ranked[0].source, "Direct");
    }

    #[test]
    fn a_short_title_does_not_match_a_long_one() {
        assert!(alike("Don't Stop", "dont stop"));
        assert!(alike("Jaded", "JADED"));
        assert!(!alike("Jaded", "Rotoscope"));
        assert!(!alike("Love", "Love Story (Taylor's Version)"));
        assert!(alike("Jaded", "Jaded - Single"));
        assert!(alike("Jaded (Remastered 2024)", "Jaded"));
        assert!(alike("Love Story", "Love Story (Taylor's Version)"));
    }

    #[test]
    fn the_active_line_follows_the_clock() {
        let lines = vec![line(0, 5, "one"), line(5, 9, "two")];
        assert_eq!(active(&lines, Duration::from_secs(2)), Some(0));
        assert_eq!(active(&lines, Duration::from_secs(6)), Some(1));
        assert_eq!(active(&lines, Duration::from_secs(30)), Some(1));
    }

    #[test]
    fn a_sung_line_holds_until_the_next_one_starts() {
        let mut padded = line(0, 12, "one");
        padded.words = Some(vec![LyricsWord {
            start: Duration::ZERO,
            end: Duration::from_secs(5),
            text: "one".to_owned(),
        }]);

        assert_eq!(
            active(std::slice::from_ref(&padded), Duration::from_secs(4)),
            Some(0)
        );
        assert_eq!(
            active(std::slice::from_ref(&padded), Duration::from_secs(8)),
            Some(0)
        );

        let pair = vec![padded, line(10, 14, "two")];
        assert_eq!(active(&pair, Duration::from_secs(8)), Some(0));
        assert_eq!(active(&pair, Duration::from_secs(10)), Some(1));
    }

    #[test]
    fn the_active_word_follows_the_clock() {
        let words = vec![
            LyricsWord {
                start: Duration::from_millis(0),
                end: Duration::from_millis(400),
                text: "one ".to_owned(),
            },
            LyricsWord {
                start: Duration::from_millis(400),
                end: Duration::from_millis(900),
                text: "two".to_owned(),
            },
        ];
        assert_eq!(active_word(&words, Duration::from_millis(100)), Some(0));
        assert_eq!(active_word(&words, Duration::from_millis(500)), Some(1));
        assert_eq!(active_word(&words, Duration::from_millis(2000)), None);
    }
}
