use std::ops::Range;
use std::time::Duration;

use crate::{LyricsLane, LyricsLine, LyricsWord, Voice};

use super::romanize;
const HOMOGLYPHS: &[(char, char)] = &[
    ('а', 'a'),
    ('в', 'b'),
    ('е', 'e'),
    ('к', 'k'),
    ('м', 'm'),
    ('н', 'h'),
    ('о', 'o'),
    ('р', 'p'),
    ('с', 'c'),
    ('т', 't'),
    ('у', 'y'),
    ('х', 'x'),
    ('і', 'i'),
    ('ј', 'j'),
    ('ѕ', 's'),
    ('ԁ', 'd'),
    ('А', 'A'),
    ('В', 'B'),
    ('Е', 'E'),
    ('К', 'K'),
    ('М', 'M'),
    ('Н', 'H'),
    ('О', 'O'),
    ('Р', 'P'),
    ('С', 'C'),
    ('Т', 'T'),
    ('У', 'Y'),
    ('Х', 'X'),
    ('І', 'I'),
    ('Ј', 'J'),
    ('Ѕ', 'S'),
    ('Α', 'A'),
    ('Β', 'B'),
    ('Ε', 'E'),
    ('Ζ', 'Z'),
    ('Η', 'H'),
    ('Ι', 'I'),
    ('Κ', 'K'),
    ('Μ', 'M'),
    ('Ν', 'N'),
    ('Ο', 'O'),
    ('Ρ', 'P'),
    ('Τ', 'T'),
    ('Υ', 'Y'),
    ('Χ', 'X'),
    ('ο', 'o'),
    ('ρ', 'p'),
    ('τ', 't'),
    ('ι', 'i'),
];

const WIDE_MARKS: &[(char, char)] = &[
    ('\u{ff07}', '\''),
    ('\u{02bc}', '\''),
    ('\u{2032}', '\''),
    ('\u{00b4}', '\''),
    ('`', '\''),
];

pub fn parse(lrc: &str) -> Vec<LyricsLine> {
    let mut lines: Vec<LyricsLine> = lrc.lines().flat_map(read).collect();
    normalize(&mut lines);
    lines
}

pub fn normalize(lines: &mut Vec<LyricsLine>) {
    lines.sort_by_key(|line| line.start);
    close(lines);
    lines.retain(|line| !structural(&line.text));
    if lines.iter().any(LyricsLine::worded) {
        for line in lines.iter_mut() {
            separate_background(line);
        }
    }

    let mut normalized: Vec<LyricsLine> = Vec::with_capacity(lines.len());
    let mut leading = Vec::new();
    for mut line in lines.drain(..) {
        if line.text.trim().is_empty() {
            if line.secondary.is_empty() {
                continue;
            }
            match normalized.last_mut() {
                Some(previous) => previous.secondary.append(&mut line.secondary),
                None => leading.append(&mut line.secondary),
            }
            continue;
        }
        if !leading.is_empty() {
            line.secondary.append(&mut leading);
        }
        normalized.push(line);
    }
    demark(&mut normalized);
    collapse(&mut normalized);
    unspoof(&mut normalized);
    unwiden(&mut normalized);
    bracket(&mut normalized);
    recapitalize(&mut normalized);
    romanize::apply(&mut normalized);
    *lines = normalized;
}

fn bracket(lines: &mut [LyricsLine]) {
    for lane in lines.iter_mut().flat_map(|line| line.secondary.iter_mut()) {
        if lane.text.trim().is_empty() || lane.text.contains(['(', ')']) {
            continue;
        }
        let Some(words) = lane
            .words
            .as_mut()
            .filter(|words| words.iter().any(|word| !word.text.trim().is_empty()))
        else {
            lane.text = format!("({})", lane.text);
            continue;
        };
        if let Some(first) = words.iter_mut().find(|word| !word.text.trim().is_empty()) {
            first.text.insert(0, '(');
        }
        if let Some(last) = words
            .iter_mut()
            .rev()
            .find(|word| !word.text.trim().is_empty())
        {
            last.text = format!("{})", last.text.trim_end());
        }
        lane.text = words.iter().map(|word| word.text.as_str()).collect();
    }
}

fn unwiden(lines: &mut [LyricsLine]) {
    for line in lines.iter_mut() {
        narrow(&mut line.words, &mut line.text);
        for lane in &mut line.secondary {
            narrow(&mut lane.words, &mut lane.text);
        }
    }
}

fn narrow(words: &mut Option<Vec<LyricsWord>>, text: &mut String) {
    for word in words.iter_mut().flatten() {
        word.text = narrowed(&word.text);
    }
    *text = narrowed(text);
}

fn narrowed(text: &str) -> String {
    text.chars()
        .map(|letter| {
            WIDE_MARKS
                .iter()
                .find(|(from, _)| *from == letter)
                .map_or(letter, |(_, to)| *to)
        })
        .collect()
}

fn recapitalize(lines: &mut [LyricsLine]) {
    if !mostly_capitalized(lines) {
        return;
    }
    for line in lines.iter_mut() {
        capitalize(&mut line.words, &mut line.text);
        for lane in &mut line.secondary {
            capitalize(&mut lane.words, &mut lane.text);
        }
    }
}

fn mostly_capitalized(lines: &[LyricsLine]) -> bool {
    let (upper, lower) = lines
        .iter()
        .filter_map(|line| opener(&line.text))
        .filter(|letter| letter.is_alphabetic())
        .fold((0usize, 0usize), |(upper, lower), letter| match letter {
            letter if letter.is_uppercase() => (upper + 1, lower),
            letter if letter.is_lowercase() => (upper, lower + 1),
            _ => (upper, lower),
        });
    upper > 0 && upper >= lower
}

fn opener(text: &str) -> Option<char> {
    text.chars().find(|letter| letter.is_alphanumeric())
}

fn capitalize(words: &mut Option<Vec<LyricsWord>>, text: &mut String) {
    if let Some(word) = words
        .iter_mut()
        .flatten()
        .find(|word| word.text.chars().any(char::is_alphanumeric))
    {
        word.text = uppercased(&word.text);
    }
    *text = uppercased(text);
}

fn uppercased(text: &str) -> String {
    let Some((at, letter)) = text
        .char_indices()
        .find(|(_, letter)| letter.is_alphanumeric())
        .filter(|(_, letter)| letter.is_lowercase())
    else {
        return text.to_owned();
    };
    format!(
        "{}{}{}",
        &text[..at],
        letter.to_uppercase(),
        &text[at + letter.len_utf8()..]
    )
}

fn demark(lines: &mut [LyricsLine]) {
    for line in lines.iter_mut() {
        unmark(&mut line.words, &mut line.text);
        for lane in &mut line.secondary {
            unmark(&mut lane.words, &mut lane.text);
        }
    }
}

fn unmark(words: &mut Option<Vec<LyricsWord>>, text: &mut String) {
    let whole: String = match words.as_ref() {
        Some(words) => words.iter().map(|word| word.text.as_str()).collect(),
        None => text.clone(),
    };
    let marks = marks(&whole);
    if marks.is_empty() {
        return;
    }
    match words {
        Some(words) => {
            let mut cursor = 0;
            for word in words.iter_mut() {
                let at = cursor;
                cursor += word.text.len();
                word.text = outside(&word.text, at, &marks);
            }
            words.retain(|word| !word.text.is_empty());
            *text = words.iter().map(|word| word.text.as_str()).collect();
        }
        None => *text = outside(text, 0, &marks),
    }
}

fn marks(text: &str) -> Vec<Range<usize>> {
    let mut marks = Vec::new();
    let mut opened = None;
    for (index, letter) in text.char_indices() {
        match letter {
            '<' => opened = Some(index),
            '>' => {
                if let Some(start) = opened.take() {
                    let inner = &text[start + 1..index];
                    let numeric = !inner.is_empty()
                        && inner
                            .trim_start_matches(['+', '-'])
                            .chars()
                            .all(|letter| letter.is_ascii_digit());
                    if numeric {
                        marks.push(start..index + letter.len_utf8());
                    }
                }
            }
            _ => {}
        }
    }
    marks
}

fn outside(text: &str, at: usize, marks: &[Range<usize>]) -> String {
    text.char_indices()
        .filter(|(index, _)| !marks.iter().any(|mark| mark.contains(&(at + index))))
        .map(|(_, letter)| letter)
        .collect()
}

fn collapse(lines: &mut [LyricsLine]) {
    for line in lines.iter_mut() {
        tighten(&mut line.words, &mut line.text);
        for lane in &mut line.secondary {
            tighten(&mut lane.words, &mut lane.text);
        }
    }
}

fn tighten(words: &mut Option<Vec<LyricsWord>>, text: &mut String) {
    let spaced = words.as_ref().is_some_and(|words| {
        words
            .iter()
            .any(|word| word.text.contains(char::is_whitespace))
    });
    match words {
        Some(words) if spaced => {
            let mut trailing = true;
            for index in 0..words.len() {
                let mut tidied = unspace_punctuation(&squeezed(&words[index].text));
                if trailing && tidied.starts_with(' ') {
                    tidied.remove(0);
                }
                if tidied.chars().next().is_some_and(closing_punctuation)
                    && let Some(previous) = words[..index]
                        .iter_mut()
                        .rev()
                        .find(|word| !word.text.is_empty())
                {
                    previous.text = previous.text.trim_end().to_owned();
                }
                trailing = tidied.ends_with(' ');
                words[index].text = tidied;
            }
            if let Some(last) = words.last_mut()
                && last.text.ends_with(' ')
            {
                last.text.pop();
            }
            *text = words.iter().map(|word| word.text.as_str()).collect();
        }
        _ => *text = unspace_punctuation(&squeezed(text)).trim().to_owned(),
    }
}

fn unspace_punctuation(text: &str) -> String {
    let mut tightened = String::with_capacity(text.len());
    let mut spacing = String::new();
    for letter in text.chars() {
        if letter.is_whitespace() {
            spacing.push(letter);
            continue;
        }
        if !closing_punctuation(letter) {
            tightened.push_str(&spacing);
        }
        spacing.clear();
        tightened.push(letter);
    }
    tightened.push_str(&spacing);
    tightened
}

fn closing_punctuation(letter: char) -> bool {
    matches!(
        letter,
        ')' | ']' | '}' | ',' | '.' | '!' | '?' | ';' | ':' | '%'
    )
}

fn squeezed(text: &str) -> String {
    let mut squeezed = String::with_capacity(text.len());
    let mut spacing = false;
    for letter in text.chars() {
        match letter.is_whitespace() {
            true => spacing = true,
            false => {
                if spacing {
                    squeezed.push(' ');
                    spacing = false;
                }
                squeezed.push(letter);
            }
        }
    }
    if spacing {
        squeezed.push(' ');
    }
    squeezed
}

fn unspoof(lines: &mut [LyricsLine]) {
    for line in lines.iter_mut() {
        if !spoofed(&line.text) {
            continue;
        }
        line.text = latinized(&line.text);
        if let Some(words) = line.words.as_mut() {
            for word in words.iter_mut() {
                word.text = latinized(&word.text);
            }
        }
    }
    for lane in lines.iter_mut().flat_map(|line| line.secondary.iter_mut()) {
        if spoofed(&lane.text) {
            lane.text = latinized(&lane.text);
        }
    }
}

fn spoofed(text: &str) -> bool {
    let mut latin = false;
    let mut masked = false;
    for letter in text.chars().filter(|letter| letter.is_alphabetic()) {
        if letter.is_ascii_alphabetic() {
            latin = true;
            continue;
        }
        match HOMOGLYPHS.iter().any(|(from, _)| *from == letter) {
            true => masked = true,
            false => return false,
        }
    }
    latin && masked
}

fn latinized(text: &str) -> String {
    text.chars()
        .map(|letter| {
            HOMOGLYPHS
                .iter()
                .find(|(from, _)| *from == letter)
                .map(|(_, to)| *to)
                .unwrap_or(letter)
        })
        .collect()
}

fn structural(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || !text.chars().any(char::is_alphanumeric) {
        return true;
    }
    let label = text
        .trim_matches(|letter: char| {
            letter.is_whitespace() || matches!(letter, '[' | ']' | '-' | '—')
        })
        .to_ascii_lowercase();
    let wrapped = (text.starts_with('[') && text.ends_with(']'))
        || (text.starts_with('-') && text.ends_with('-'));
    wrapped
        && matches!(
            label.as_str(),
            "intro"
                | "verse"
                | "pre-chorus"
                | "chorus"
                | "refrain"
                | "hook"
                | "bridge"
                | "breakdown"
                | "solo"
                | "instrumental"
                | "outro"
        )
}

fn separate_background(line: &mut LyricsLine) {
    let original = line.text.clone();
    let spans = parenthetical_spans(&original);
    if spans.is_empty() {
        return;
    }
    let placements = line
        .words
        .as_deref()
        .and_then(|words| place_words(&original, words));
    let mut lanes = Vec::with_capacity(spans.len());

    for span in &spans {
        let text = original[span.inner.clone()].trim().to_owned();
        if text.is_empty() {
            continue;
        }
        let words = placements.as_ref().and_then(|placements| {
            let words: Vec<LyricsWord> = line
                .words
                .as_ref()?
                .iter()
                .zip(placements)
                .filter_map(|(word, placed)| {
                    let start = placed.start.max(span.inner.start);
                    let end = placed.end.min(span.inner.end);
                    let text = (start < end).then(|| original[start..end].to_owned())?;
                    (!text.trim().is_empty()).then_some(LyricsWord {
                        start: word.start,
                        end: word.end,
                        text,
                    })
                })
                .collect();
            (!words.is_empty()).then_some(words)
        });
        let start = words
            .as_ref()
            .and_then(|words| words.first())
            .map(|word| word.start)
            .unwrap_or(line.start);
        let end = words
            .as_ref()
            .and_then(|words| words.last())
            .map(|word| word.end)
            .or(line.end);
        lanes.push(LyricsLane {
            start,
            end,
            text,
            romanized: None,
            words,
        });
    }

    if lanes.is_empty() {
        return;
    }
    if let (Some(words), Some(placements)) = (&mut line.words, placements) {
        let kept: Vec<LyricsWord> = words
            .drain(..)
            .zip(placements)
            .filter_map(|(mut word, placed)| {
                word.text = word_without_spans(&original, &placed, &spans);
                (!word.text.trim().is_empty()).then_some(word)
            })
            .collect();
        *words = kept;
        if words.is_empty() {
            line.words = None;
        }
    }
    line.text = without_spans(&original, &spans);
    line.secondary.extend(lanes);
}

struct Parenthetical {
    outer: Range<usize>,
    inner: Range<usize>,
}

fn parenthetical_spans(text: &str) -> Vec<Parenthetical> {
    let mut spans = Vec::new();
    let mut opened: Option<(usize, usize)> = None;
    let mut depth = 0usize;
    for (index, letter) in text.char_indices() {
        match letter {
            '(' | '（' => {
                if depth == 0 {
                    opened = Some((index, index + letter.len_utf8()));
                }
                depth += 1;
            }
            ')' | '）' if depth > 0 => {
                depth -= 1;
                if depth == 0
                    && let Some((outer, inner)) = opened.take()
                {
                    spans.push(Parenthetical {
                        outer: outer..index + letter.len_utf8(),
                        inner: inner..index,
                    });
                }
            }
            _ => {}
        }
    }
    spans
}

fn place_words(text: &str, words: &[LyricsWord]) -> Option<Vec<Range<usize>>> {
    let mut placed = Vec::with_capacity(words.len());
    let mut cursor = 0;
    for word in words {
        let remainder = text.get(cursor..)?;
        let relative = remainder.find(&word.text)?;
        let start = cursor + relative;
        let end = start + word.text.len();
        placed.push(start..end);
        cursor = end;
    }
    Some(placed)
}

fn without_spans(text: &str, spans: &[Parenthetical]) -> String {
    let mut primary = String::new();
    let mut cursor = 0;
    for span in spans {
        primary.push_str(&text[cursor..span.outer.start]);
        cursor = span.outer.end;
    }
    primary.push_str(&text[cursor..]);
    primary.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn word_without_spans(text: &str, placed: &Range<usize>, spans: &[Parenthetical]) -> String {
    let mut primary = String::new();
    let mut cursor = placed.start;
    for span in spans
        .iter()
        .filter(|span| span.outer.start < placed.end && span.outer.end > placed.start)
    {
        let end = span.outer.start.min(placed.end);
        if cursor < end {
            primary.push_str(&text[cursor..end]);
        }
        cursor = cursor.max(span.outer.end.min(placed.end));
    }
    if cursor < placed.end {
        primary.push_str(&text[cursor..placed.end]);
    }
    primary
}

fn close(lines: &mut [LyricsLine]) {
    for index in 0..lines.len().saturating_sub(1) {
        let next = lines[index + 1].start;
        if lines[index].end.is_none() {
            lines[index].end = Some(next);
        }
    }
    for line in lines.iter_mut() {
        let Some(end) = line.end else { continue };
        let Some(words) = line.words.as_mut() else {
            continue;
        };
        if let Some(last) = words.last_mut()
            && last.end <= last.start
        {
            last.end = end.max(last.start);
        }
    }
}

pub fn stamp_of(stamp: &str) -> Option<Duration> {
    let (minutes, rest) = stamp.split_once(':')?;
    let minutes: u64 = minutes.trim().parse().ok()?;
    let seconds: f64 = rest.replace(',', ".").parse().ok()?;
    if !seconds.is_finite() || seconds < 0. {
        return None;
    }
    Duration::try_from_secs_f64(minutes as f64 * 60. + seconds).ok()
}

struct Segment {
    at: Option<Duration>,
    text: String,
}

fn read(line: &str) -> Vec<LyricsLine> {
    let mut rest = line.trim();
    let mut stamps = Vec::new();
    while let Some(body) = rest.strip_prefix('[') {
        let Some((stamp, tail)) = body.split_once(']') else {
            break;
        };
        let Some(at) = stamp_of(stamp) else { break };
        stamps.push(at);
        rest = tail.trim_start();
    }

    let (text, words) = spoken(rest);
    stamps
        .into_iter()
        .map(|start| LyricsLine {
            start,
            end: None,
            text: text.clone(),
            romanized: None,
            words: words.clone().map(|words| shifted(words, start)),
            secondary: Vec::new(),
            voice: Voice::Lead,
        })
        .collect()
}

fn shifted(words: Vec<LyricsWord>, start: Duration) -> Vec<LyricsWord> {
    let Some(first) = words.first() else {
        return words;
    };
    let Some(drift) = start.checked_sub(first.start) else {
        return words;
    };
    match drift.is_zero() {
        true => words,
        false => words
            .into_iter()
            .map(|word| LyricsWord {
                start: word.start + drift,
                end: word.end + drift,
                text: word.text,
            })
            .collect(),
    }
}

fn spoken(body: &str) -> (String, Option<Vec<LyricsWord>>) {
    let segments = cut(body);
    let whole: String = segments
        .iter()
        .map(|segment| segment.text.as_str())
        .collect();
    let timed: Vec<&Segment> = segments
        .iter()
        .filter(|segment| segment.at.is_some())
        .collect();
    if timed.is_empty() {
        return (whole.trim().to_owned(), None);
    }

    let mut words = Vec::new();
    for (index, segment) in timed.iter().enumerate() {
        let start = segment.at.expect("a timed segment carries a stamp");
        if segment.text.is_empty() {
            if let Some(last) = words.last_mut() {
                let last: &mut LyricsWord = last;
                last.end = start.max(last.start);
            }
            continue;
        }
        let end = timed
            .get(index + 1)
            .and_then(|next| next.at)
            .filter(|next| *next > start)
            .unwrap_or(start);
        words.push(LyricsWord {
            start,
            end,
            text: segment.text.clone(),
        });
    }

    let whole = whole.trim_end().to_owned();
    (whole, (!words.is_empty()).then_some(words))
}

fn cut(body: &str) -> Vec<Segment> {
    let mut segments = vec![Segment {
        at: None,
        text: String::new(),
    }];
    let mut rest = body;
    while let Some(open) = rest.find('<') {
        let tail = &rest[open + 1..];
        let Some(shut) = tail.find('>') else { break };
        let last = segments.last_mut().expect("a segment is always open");
        match stamp_of(&tail[..shut]) {
            Some(at) => {
                last.text.push_str(&rest[..open]);
                segments.push(Segment {
                    at: Some(at),
                    text: String::new(),
                });
            }
            None => last.text.push_str(&rest[..open + shut + 2]),
        }
        rest = &tail[shut + 1..];
    }
    segments
        .last_mut()
        .expect("a segment is always open")
        .text
        .push_str(rest);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_stamp() {
        assert_eq!(stamp_of("01:02.50"), Some(Duration::from_millis(62_500)));
        assert_eq!(stamp_of("00:09"), Some(Duration::from_secs(9)));
        assert_eq!(stamp_of("bogus"), None);
    }

    #[test]
    fn parses_lrc_and_closes_every_line() {
        let lines = parse("[00:10.00] first\n[00:14.50] second\n[bad] skipped\n");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[0].end, Some(Duration::from_millis(14_500)));
        assert_eq!(lines[1].end, None);
    }

    #[test]
    fn an_empty_timed_line_becomes_a_gap_between_lyrics() {
        let lines = parse("[00:09.36] sung\n[00:11.97]\n[00:24.16] next\n");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].end, Some(Duration::from_millis(11_970)));
        assert_eq!(lines[1].start, Duration::from_millis(24_160));
    }

    #[test]
    fn parenthetical_words_become_an_independently_timed_background_lane() {
        let mut lines = vec![LyricsLine {
            start: Duration::from_secs(1),
            end: Some(Duration::from_secs(4)),
            text: "Lead (echo) after".to_owned(),
            romanized: None,
            words: Some(vec![
                LyricsWord {
                    start: Duration::from_secs(1),
                    end: Duration::from_secs(3),
                    text: "Lead ".to_owned(),
                },
                LyricsWord {
                    start: Duration::from_millis(1500),
                    end: Duration::from_millis(2500),
                    text: "(echo)".to_owned(),
                },
                LyricsWord {
                    start: Duration::from_secs(3),
                    end: Duration::from_secs(4),
                    text: " after".to_owned(),
                },
            ]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        }];

        normalize(&mut lines);

        assert_eq!(lines[0].text, "Lead after");
        assert_eq!(lines[0].words.as_ref().map(Vec::len), Some(2));
        assert_eq!(lines[0].secondary.len(), 1);
        assert_eq!(lines[0].secondary[0].text, "(Echo)");
        assert_eq!(lines[0].secondary[0].start, Duration::from_millis(1500));
        assert_eq!(
            lines[0].secondary[0].sung_end(),
            Some(Duration::from_millis(2500))
        );
    }

    #[test]
    fn removing_an_inline_background_lane_closes_the_punctuation_gap() {
        let mut lines = vec![LyricsLine {
            start: Duration::from_secs(1),
            end: Some(Duration::from_secs(4)),
            text: "Может, я murder (E), они все".to_owned(),
            romanized: None,
            words: Some(vec![
                LyricsWord {
                    start: Duration::from_secs(1),
                    end: Duration::from_secs(2),
                    text: "Может, я murder ".to_owned(),
                },
                LyricsWord {
                    start: Duration::from_secs(2),
                    end: Duration::from_millis(2500),
                    text: "(E)".to_owned(),
                },
                LyricsWord {
                    start: Duration::from_millis(2500),
                    end: Duration::from_secs(4),
                    text: ", они все".to_owned(),
                },
            ]),
            secondary: Vec::new(),
            voice: Voice::Lead,
        }];

        normalize(&mut lines);

        assert_eq!(lines[0].text, "Может, я murder, они все");
        assert_eq!(
            lines[0]
                .words
                .as_ref()
                .expect("timed primary words")
                .iter()
                .map(|word| word.text.as_str())
                .collect::<String>(),
            lines[0].text
        );
        assert_eq!(lines[0].secondary[0].text, "(E)");
    }

    #[test]
    fn a_standalone_parenthetical_line_attaches_to_the_previous_verse() {
        let mut lines = vec![
            LyricsLine {
                start: Duration::from_secs(1),
                end: Some(Duration::from_secs(2)),
                text: "Lead".to_owned(),
                romanized: None,
                words: Some(vec![LyricsWord {
                    start: Duration::from_secs(1),
                    end: Duration::from_secs(2),
                    text: "Lead".to_owned(),
                }]),
                secondary: Vec::new(),
                voice: Voice::Lead,
            },
            LyricsLine {
                start: Duration::from_secs(2),
                end: Some(Duration::from_secs(3)),
                text: "(echo)".to_owned(),
                romanized: None,
                words: Some(vec![LyricsWord {
                    start: Duration::from_secs(2),
                    end: Duration::from_secs(3),
                    text: "(echo)".to_owned(),
                }]),
                secondary: Vec::new(),
                voice: Voice::Lead,
            },
        ];

        normalize(&mut lines);

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].secondary[0].text, "(Echo)");
        assert_eq!(lines[0].sung_end(), Some(Duration::from_secs(3)));
    }

    #[test]
    fn section_labels_and_decorative_marks_are_not_lyrics() {
        let lines = parse(
            "[00:01.00][Chorus]\n[00:02.00].\n[00:03.00]∮\n[00:04.00]- solo -\n[00:08.00]Actual lyric",
        );

        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Actual lyric");
    }

    #[test]
    fn reads_word_tags() {
        let lines = parse("[00:12.50]<00:12.50>I <00:12.80>see <00:13.10>trees\n[00:15.00]next");

        let words = lines[0].words.as_ref().expect("the line is worded");
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].text, "I ");
        assert_eq!(words[1].start, Duration::from_millis(12_800));
        assert_eq!(words[2].end, Duration::from_secs(15));
        assert_eq!(lines[0].text, "I see trees");
    }

    #[test]
    fn a_bare_end_tag_closes_the_last_word() {
        let lines = parse("[00:01.00]<00:01.00>one <00:01.50>two<00:02.00>");

        let words = lines[0].words.as_ref().expect("the line is worded");
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].end, Duration::from_secs(2));
        assert_eq!(lines[0].text, "one two");
    }

    #[test]
    fn angle_brackets_that_are_not_stamps_stay_in_the_text() {
        let lines = parse("[00:01.00]a <3 b");

        assert_eq!(lines[0].text, "a <3 b");
        assert!(lines[0].words.is_none());
    }

    #[test]
    fn one_line_can_carry_several_stamps() {
        let lines = parse("[00:01.00][00:31.00]chorus");

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start, Duration::from_secs(1));
        assert_eq!(lines[1].start, Duration::from_secs(31));
        assert_eq!(lines[1].text, "chorus");
    }

    #[test]
    fn a_repeated_worded_line_moves_its_words_along() {
        let lines = parse("[00:01.00][00:31.00]<00:01.00>one <00:01.50>two");

        let words = lines[1].words.as_ref().expect("the line is worded");
        assert_eq!(words[0].start, Duration::from_secs(31));
        assert_eq!(words[1].start, Duration::from_millis(31_500));
    }

    #[test]
    fn a_stamp_past_the_longest_duration_is_not_a_stamp() {
        assert_eq!(stamp_of("00:1e30"), None);
        assert_eq!(stamp_of("18446744073709551615:00"), None);
        assert_eq!(parse("[00:1e30]lost\n[00:01.00]kept").len(), 1);
    }
}
