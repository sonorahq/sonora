//! Text helpers the lists share: case-folded comparison and search, fuzzy ranking, lapses.

use std::cmp::Ordering;
use std::time::Duration;

/// Below this a lapse is worth a decimal, above it the fraction says nothing.
const FINE: u64 = 10;

/// How long something took, in seconds, for a caller to put its own unit after. Under ten
/// seconds it keeps one decimal, since that is where the difference reads; above, it rounds.
pub(crate) fn lapsed(took: Duration) -> String {
    let seconds = took.as_secs_f32();
    match took.as_secs() < FINE {
        true => format!("{seconds:.1}"),
        false => format!("{}", seconds.round() as u64),
    }
}

/// What one matched letter of a fuzzy query is worth.
const LETTER: u32 = 1;
/// What a letter matched right after the previous one is worth on top.
const ADJACENT: u32 = 3;
/// What a letter matched at the start of a word is worth on top.
const BOUNDARY: u32 = 2;

/// How well `needle` is found in `haystack`, or `None` when it is not. The match folds case
/// and every whitespace-separated word of the needle has to be found in order, letter by letter
/// at the least. A word found whole scores above one found scattered, and a letter that starts
/// a word of the haystack counts extra, so "tray" ranks Close to tray above Traffic lights.
pub(crate) fn fuzzy(haystack: &str, needle: &str) -> Option<u32> {
    let hay: Vec<char> = haystack.chars().flat_map(char::to_lowercase).collect();
    needle
        .split_whitespace()
        .map(|word| {
            let word: Vec<char> = word.chars().flat_map(char::to_lowercase).collect();
            word_score(&hay, &word)
        })
        .sum()
}

/// Whether `needle` (already lowercase) appears in `haystack`, ignoring the haystack's case.
pub(crate) fn holds(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    haystack
        .char_indices()
        .any(|(at, _)| starts(&haystack[at..], needle))
}

/// Orders two strings as their lowercase forms compare, for sorting by name.
pub(crate) fn folded(left: &str, right: &str) -> Ordering {
    let mut left = left.chars().flat_map(char::to_lowercase);
    let mut right = right.chars().flat_map(char::to_lowercase);

    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(here), Some(there)) if here == there => continue,
            (Some(here), Some(there)) => return here.cmp(&there),
        }
    }
}

fn word_score(hay: &[char], word: &[char]) -> Option<u32> {
    if word.is_empty() {
        return Some(0);
    }
    if let Some(start) = hay.windows(word.len()).position(|window| window == word) {
        let letters = word.len() as u32;
        return Some(letters * LETTER + (letters - 1) * ADJACENT + boundary(hay, start));
    }

    let mut score = 0;
    let mut from = 0;
    let mut last = None;
    for &wanted in word {
        let found = from + hay[from..].iter().position(|&letter| letter == wanted)?;
        score += LETTER + boundary(hay, found);
        if last.is_some_and(|last| last + 1 == found) {
            score += ADJACENT;
        }
        last = Some(found);
        from = found + 1;
    }
    Some(score)
}

fn boundary(hay: &[char], at: usize) -> u32 {
    match at.checked_sub(1).map(|before| hay[before]) {
        Some(before) if before.is_alphanumeric() => 0,
        _ => BOUNDARY,
    }
}

fn starts(haystack: &str, needle: &str) -> bool {
    let mut lowered = haystack.chars().flat_map(char::to_lowercase);
    let mut wanted = needle.chars();

    loop {
        match (lowered.next(), wanted.next()) {
            (_, None) => return true,
            (Some(here), Some(there)) if here == there => continue,
            _ => return false,
        }
    }
}
