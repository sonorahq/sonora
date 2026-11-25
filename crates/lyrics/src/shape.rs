use std::time::Duration;

use music::{Lyrics, LyricsLine, LyricsWord, Voice};

const LEAST: usize = 6;
const MATCHED: f64 = 0.7;
const KEPT: f64 = 0.5;
const SEATED: f64 = 0.9;
const LIMIT: usize = 3000;
const CELLS: usize = 2_000_000;
const TICKS: &[char] = &['\'', '\u{2019}', '\u{ff07}', '\u{2018}', '\u{00b4}', '`'];

struct Sung {
    start: Duration,
    end: Duration,
    key: String,
}

struct Slot {
    at: usize,
    key: String,
}

pub fn conform(worded: &Lyrics, guide: &Lyrics) -> Option<Lyrics> {
    let Lyrics::Synced { lines: guide } = guide else {
        return None;
    };
    let Lyrics::Synced { lines: worded } = worded else {
        return None;
    };
    if guide.is_empty() || worded.is_empty() {
        return None;
    }

    let sung = sung(worded);
    let slotted: Vec<Vec<Slot>> = guide.iter().map(|line| tokens(&line.text)).collect();
    let places: Vec<(usize, usize)> = slotted
        .iter()
        .enumerate()
        .flat_map(|(line, slots)| (0..slots.len()).map(move |slot| (line, slot)))
        .collect();
    if sung.len() < LEAST || places.len() < LEAST || sung.len() > LIMIT || places.len() > LIMIT {
        return None;
    }
    if sung.len() * places.len() > CELLS {
        return None;
    }

    let left: Vec<&str> = sung.iter().map(|word| word.key.as_str()).collect();
    let right: Vec<&str> = places
        .iter()
        .map(|(line, slot)| slotted[*line][*slot].key.as_str())
        .collect();
    let pairs = paired(&left, &right);
    if pairs.len() < LEAST
        || (pairs.len() as f64) < MATCHED * places.len() as f64
        || (pairs.len() as f64) < KEPT * sung.len() as f64
        || !seated(&pairs, &places, &slotted)
    {
        return None;
    }

    let mut spans: Vec<Option<(Duration, Duration)>> = vec![None; places.len()];
    for (word, place) in &pairs {
        spans[*place] = Some((sung[*word].start, sung[*word].end));
    }
    let spans = filled(spans, &hints(guide, &slotted));

    let mut lines: Vec<LyricsLine> = Vec::with_capacity(guide.len());
    let mut cursor = 0;
    for (index, line) in guide.iter().enumerate() {
        let slots = &slotted[index];
        if slots.is_empty() {
            continue;
        }
        let mine = &spans[cursor..cursor + slots.len()];
        cursor += slots.len();
        let words = worded_from(&line.text, slots, mine);
        let start = words.first().map(|word| word.start)?;
        let end = words.iter().map(|word| word.end).max()?;
        lines.push(LyricsLine {
            start,
            end: Some(end.max(start)),
            text: line.text.clone(),
            romanized: None,
            words: Some(words),
            secondary: Vec::new(),
            voice: Voice::Lead,
        });
    }
    super::lrc::normalize(&mut lines);
    if lines.len() < 2 {
        return None;
    }

    Some(Lyrics::Synced {
        lines: lines.into(),
    })
}

fn seated(pairs: &[(usize, usize)], places: &[(usize, usize)], slotted: &[Vec<Slot>]) -> bool {
    let mut held = vec![false; slotted.len()];
    for (_, place) in pairs {
        held[places[*place].0] = true;
    }
    let counted = slotted.iter().filter(|slots| !slots.is_empty()).count();
    let anchored = held
        .iter()
        .zip(slotted)
        .filter(|(held, slots)| **held && !slots.is_empty())
        .count();
    (anchored as f64) >= SEATED * counted as f64
}

fn sung(lines: &[LyricsLine]) -> Vec<Sung> {
    let mut sung = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match line.words.as_deref().filter(|words| !words.is_empty()) {
            Some(words) => sung.extend(timed(words)),
            None => {
                let until = line
                    .end
                    .or_else(|| lines.get(index + 1).map(|next| next.start))
                    .unwrap_or(line.start)
                    .max(line.start);
                sung.extend(spread(&line.text, line.start, until));
            }
        }
        for lane in &line.secondary {
            match lane.words.as_deref().filter(|words| !words.is_empty()) {
                Some(words) => sung.extend(timed(words)),
                None => {
                    let until = lane.end.unwrap_or(lane.start).max(lane.start);
                    sung.extend(spread(&lane.text, lane.start, until));
                }
            }
        }
    }
    sung
}

fn timed(words: &[LyricsWord]) -> Vec<Sung> {
    words
        .iter()
        .flat_map(|word| {
            tokens(&word.text).into_iter().map(move |slot| Sung {
                start: word.start,
                end: word.end.max(word.start),
                key: slot.key,
            })
        })
        .collect()
}

fn spread(text: &str, start: Duration, end: Duration) -> Vec<Sung> {
    let slots = tokens(text);
    let total: usize = slots.iter().map(|slot| slot.key.chars().count()).sum();
    if total == 0 {
        return Vec::new();
    }
    let span = end.saturating_sub(start);
    let mut sung = Vec::with_capacity(slots.len());
    let mut passed = 0usize;
    for slot in slots {
        let length = slot.key.chars().count();
        let from = start + span.mul_f64(passed as f64 / total as f64);
        passed += length;
        let to = start + span.mul_f64(passed as f64 / total as f64);
        sung.push(Sung {
            start: from,
            end: to,
            key: slot.key,
        });
    }
    sung
}

fn tokens(text: &str) -> Vec<Slot> {
    let mut slots = Vec::new();
    let mut open: Option<Slot> = None;
    for (at, letter) in text.char_indices() {
        if wide(letter) {
            slots.extend(open.take());
            slots.push(Slot {
                at,
                key: letter.to_lowercase().collect(),
            });
            continue;
        }
        if TICKS.contains(&letter) {
            continue;
        }
        match letter.is_alphanumeric() {
            true => open
                .get_or_insert_with(|| Slot {
                    at,
                    key: String::new(),
                })
                .key
                .extend(letter.to_lowercase()),
            false => slots.extend(open.take()),
        }
    }
    slots.extend(open);
    slots
}

fn wide(letter: char) -> bool {
    matches!(letter,
        '\u{3040}'..='\u{30ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}'
        | '\u{f900}'..='\u{faff}')
}

fn paired(left: &[&str], right: &[&str]) -> Vec<(usize, usize)> {
    let (rows, columns) = (left.len() + 1, right.len() + 1);
    let mut table = vec![0u32; rows * columns];
    for row in (0..left.len()).rev() {
        for column in (0..right.len()).rev() {
            let at = row * columns + column;
            table[at] = match akin(left[row], right[column]) {
                true => table[at + columns + 1] + 1,
                false => table[at + columns].max(table[at + 1]),
            };
        }
    }

    let mut pairs = Vec::new();
    let (mut row, mut column) = (0, 0);
    while row < left.len() && column < right.len() {
        let at = row * columns + column;
        if akin(left[row], right[column]) {
            pairs.push((row, column));
            row += 1;
            column += 1;
            continue;
        }
        match table[at + columns] >= table[at + 1] {
            true => row += 1,
            false => column += 1,
        }
    }
    pairs
}

fn akin(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let (short, long) = match left.len() <= right.len() {
        true => (left, right),
        false => (right, left),
    };
    short.len() >= 4 && long.len() - short.len() <= 2 && long.starts_with(short)
}

fn hints(guide: &[LyricsLine], slotted: &[Vec<Slot>]) -> Vec<(Duration, Duration)> {
    let mut hints = Vec::new();
    for (index, line) in guide.iter().enumerate() {
        let slots = &slotted[index];
        if slots.is_empty() {
            continue;
        }
        let until = line
            .end
            .or_else(|| guide.get(index + 1).map(|next| next.start))
            .unwrap_or(line.start)
            .max(line.start);
        let span = until.saturating_sub(line.start);
        let total: usize = slots
            .iter()
            .map(|slot| slot.key.chars().count().max(1))
            .sum();
        let mut passed = 0usize;
        for slot in slots {
            let start = line.start + span.mul_f64(passed as f64 / total as f64);
            passed += slot.key.chars().count().max(1);
            let end = line.start + span.mul_f64(passed as f64 / total as f64);
            hints.push((start, end));
        }
    }
    hints
}

fn shared(after: Duration, before: Duration, count: usize) -> Vec<(Duration, Duration)> {
    let span = before.saturating_sub(after);
    (0..count)
        .map(|step| {
            let from = after + span.mul_f64(step as f64 / count as f64);
            let to = after + span.mul_f64((step + 1) as f64 / count as f64);
            (from, to)
        })
        .collect()
}

fn filled(
    spans: Vec<Option<(Duration, Duration)>>,
    hints: &[(Duration, Duration)],
) -> Vec<(Duration, Duration)> {
    let mut settled: Vec<(Duration, Duration)> = Vec::with_capacity(spans.len());
    let mut index = 0;
    while index < spans.len() {
        if let Some(span) = spans[index] {
            settled.push(span);
            index += 1;
            continue;
        }
        let mut until = index;
        while until < spans.len() && spans[until].is_none() {
            until += 1;
        }
        let after = settled.last().map(|(_, end)| *end);
        let before = spans
            .get(until)
            .and_then(|span| *span)
            .map(|(start, _)| start);
        match (after, before) {
            (Some(after), Some(before)) if before > after => {
                settled.extend(shared(after, before, until - index));
            }
            _ => {
                for at in index..until {
                    let (mut start, mut end) = hints
                        .get(at)
                        .copied()
                        .unwrap_or_else(|| (after.unwrap_or_default(), after.unwrap_or_default()));
                    if let Some(after) = after {
                        start = start.max(after);
                        end = end.max(start);
                    }
                    if let Some(before) = before {
                        start = start.min(before);
                        end = end.min(before).max(start);
                    }
                    let floor = settled.last().map(|(_, end)| *end).unwrap_or(start);
                    settled.push((start.max(floor), end.max(start.max(floor))));
                }
            }
        }
        index = until;
    }
    settled
}

fn worded_from(text: &str, slots: &[Slot], spans: &[(Duration, Duration)]) -> Vec<LyricsWord> {
    let mut words = Vec::with_capacity(slots.len());
    for (index, slot) in slots.iter().enumerate() {
        let from = match index {
            0 => 0,
            _ => slot.at,
        };
        let until = slots
            .get(index + 1)
            .map_or(text.len(), |next| next.at.max(from));
        let (start, end) = spans[index];
        words.push(LyricsWord {
            start,
            end: end.max(start),
            text: text[from..until].to_owned(),
        });
    }
    words
}
