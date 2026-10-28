use deunicode::deunicode_char;

use crate::{LyricsLine, RomanizedText, WritingSystem};

pub(super) fn apply(lines: &mut [LyricsLine]) {
    let system = detect(lines.iter().flat_map(|line| {
        std::iter::once(line.text.as_str())
            .chain(line.secondary.iter().map(|lane| lane.text.as_str()))
    }));
    let Some(system) = system else { return };

    for line in lines {
        line.romanized = convert(&line.text, system);
        for lane in &mut line.secondary {
            lane.romanized = convert(&lane.text, system);
        }
    }
}

/// The romanization of unsynced lyrics, converted line by line so its text holds exactly one line
/// for each line of `text`. A line that needs no romanization is left blank.
pub(crate) fn plain(text: &str) -> Option<RomanizedText> {
    let system = detect(std::iter::once(text))?;
    let lines = text
        .lines()
        .map(|line| convert(line, system).map(|romanized| romanized.text))
        .collect::<Vec<_>>();
    lines.iter().any(Option::is_some).then(|| RomanizedText {
        text: lines
            .into_iter()
            .map(Option::unwrap_or_default)
            .collect::<Vec<_>>()
            .join("\n"),
        writing_system: system,
    })
}

fn detect<'a>(texts: impl Iterator<Item = &'a str>) -> Option<WritingSystem> {
    let text = texts.collect::<Vec<_>>().join("\n");
    if text.chars().any(super::japanese::kana) {
        return Some(WritingSystem::Japanese);
    }
    dominant_system(&text)
}

fn convert(text: &str, system: WritingSystem) -> Option<RomanizedText> {
    let romanized = match system {
        WritingSystem::Japanese => super::japanese::romanize(text),
        _ => universal(text),
    };
    let romanized = tidy(&romanized);
    (!romanized.is_empty() && romanized != tidy(text)).then_some(RomanizedText {
        text: romanized,
        writing_system: system,
    })
}

fn dominant_system(text: &str) -> Option<WritingSystem> {
    let mut counts = [0usize; WritingSystem::ALL.len()];
    for letter in text.chars().filter(|letter| non_latin_letter(*letter)) {
        counts[system_index(system_of(letter))] += 1;
    }
    counts
        .into_iter()
        .enumerate()
        .filter(|(_, count)| *count > 0)
        .max_by_key(|(_, count)| *count)
        .map(|(index, _)| WritingSystem::ALL[index])
}

fn system_of(letter: char) -> WritingSystem {
    match letter as u32 {
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af => {
            WritingSystem::Chinese
        }
        0x1100..=0x11ff | 0x3130..=0x318f | 0xa960..=0xa97f | 0xac00..=0xd7af | 0xd7b0..=0xd7ff => {
            WritingSystem::Korean
        }
        0x0400..=0x052f | 0x1c80..=0x1c8f | 0x2de0..=0x2dff | 0xa640..=0xa69f => {
            WritingSystem::Cyrillic
        }
        0x0370..=0x03ff | 0x1f00..=0x1fff => WritingSystem::Greek,
        0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff | 0xfb50..=0xfdff | 0xfe70..=0xfeff => {
            WritingSystem::Arabic
        }
        _ => WritingSystem::Other,
    }
}

const fn system_index(system: WritingSystem) -> usize {
    match system {
        WritingSystem::Japanese => 0,
        WritingSystem::Chinese => 1,
        WritingSystem::Korean => 2,
        WritingSystem::Cyrillic => 3,
        WritingSystem::Greek => 4,
        WritingSystem::Arabic => 5,
        WritingSystem::Other => 6,
    }
}

fn universal(text: &str) -> String {
    let mut romanized = String::new();
    for letter in text.chars() {
        match non_latin_letter(letter) {
            true => romanized.push_str(deunicode_char(letter).unwrap_or("")),
            false => romanized.push(letter),
        }
    }
    romanized
}

fn non_latin_letter(letter: char) -> bool {
    letter.is_alphabetic() && !latin(letter)
}

fn latin(letter: char) -> bool {
    matches!(
        letter as u32,
        0x0041..=0x024f | 0x1d00..=0x1eff | 0x2c60..=0x2c7f | 0xa720..=0xa7ff | 0xab30..=0xab6f
    )
}

fn tidy(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{LyricsLane, Voice};

    use super::*;

    #[test]
    fn romanizes_japanese_with_song_wide_script_detection() {
        let system = detect(["こんにちは世界", "君のそばにいるよ"].into_iter())
            .expect("kana identifies Japanese");

        assert_eq!(
            convert("こんにちは世界", system)
                .as_ref()
                .map(|text| text.text.as_str()),
            Some("konnichiwa sekai")
        );
    }

    #[test]
    fn generates_primary_and_background_romanization() {
        let mut lines = vec![LyricsLine {
            start: Duration::ZERO,
            end: None,
            text: "Привет".to_owned(),
            romanized: None,
            words: None,
            secondary: vec![LyricsLane {
                start: Duration::ZERO,
                end: None,
                text: "мир".to_owned(),
                romanized: None,
                words: None,
            }],
            voice: Voice::Lead,
        }];

        apply(&mut lines);

        assert_eq!(
            lines[0].romanized.as_ref().map(|text| text.text.as_str()),
            Some("Privet")
        );
        assert_eq!(
            lines[0].secondary[0]
                .romanized
                .as_ref()
                .map(|text| text.text.as_str()),
            Some("mir")
        );
        assert_eq!(
            lines[0].romanized.as_ref().map(|text| text.writing_system),
            Some(WritingSystem::Cyrillic)
        );
    }

    #[test]
    fn romanizes_cyrillic_locally() {
        let romanized = plain("Привет, мир!").expect("Cyrillic is romanized");
        assert_eq!(romanized.text, "Privet, mir!");
        assert_eq!(romanized.writing_system, WritingSystem::Cyrillic);
    }

    #[test]
    fn distinguishes_selectable_writing_systems() {
        let cases = [
            ("你好", WritingSystem::Chinese),
            ("안녕하세요", WritingSystem::Korean),
            ("Γειά", WritingSystem::Greek),
            ("مرحبا", WritingSystem::Arabic),
            ("שלום", WritingSystem::Other),
        ];

        for (text, expected) in cases {
            assert_eq!(detect(std::iter::once(text)), Some(expected));
        }
    }

    #[test]
    fn latin_lyrics_do_not_get_a_duplicate_line() {
        assert_eq!(plain("Déjà vu"), None);
    }
}
