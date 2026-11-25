use music::LyricsLine;

const LABELS: &[&str] = &[
    "artist:",
    "title:",
    "album:",
    "song:",
    "by:",
    "lyrics:",
    "written by",
    "composed by",
    "arranged by",
    "produced by",
    "lyrics by",
    "music by",
    "mixed by",
    "mastered by",
    "recorded by",
    "engineered by",
    "vocals by",
    "composer",
    "lyricist",
    "arranger",
    "producer",
    "作词",
    "作曲",
    "编曲",
    "作詞",
    "編曲",
    "制作",
    "監製",
    "监制",
    "混音",
    "母带",
    "和声",
];

const QUIET: &[&str] = &[
    "instrumental",
    "this song is instrumental",
    "纯音乐",
    "此歌曲为没有填词的纯音乐",
];

const TAGS: &[&str] = &[
    "version",
    "edition",
    "edit",
    "edited",
    "explicit",
    "clean",
    "radio",
    "remaster",
    "remastered",
    "remix",
    "mix",
    "demo",
    "live",
    "acoustic",
    "deluxe",
    "bonus",
    "single",
    "album",
    "original",
    "mono",
    "stereo",
    "karaoke",
    "cover",
];

const STRONG: &[&str] = &[
    "version",
    "edition",
    "edit",
    "edited",
    "explicit",
    "remaster",
    "remastered",
    "remix",
    "demo",
    "deluxe",
    "bonus",
    "mono",
    "stereo",
    "karaoke",
];

const RIGHTS: &[&str] = &[
    "all rights reserved",
    "used by permission",
    "administered by",
    "published by",
    "publishing",
    "copyright",
    "\u{a9}",
    "\u{2117}",
];

const HEADERS: usize = 16;
const LABEL_REACH: usize = 48;

pub fn instrumental(lines: &[LyricsLine]) -> bool {
    !lines.is_empty()
        && lines.iter().all(|line| {
            let text = line.text.trim().to_lowercase();
            text.is_empty() || QUIET.iter().any(|mark| text.contains(mark))
        })
}

pub fn headed(lines: &mut Vec<LyricsLine>, title: &str, artists: &[String]) -> bool {
    for _ in 0..HEADERS {
        let Some(first) = lines.first().map(|line| line.text.clone()) else {
            return true;
        };
        if labelled(&first) || tagged(&first) || credited(&first) {
            lines.remove(0);
            continue;
        }
        if !named(&first, title, artists) {
            let heads = lines
                .get(1)
                .map(|line| line.text.as_str())
                .is_some_and(|next| header(next, title, artists));
            if only_titled(&first, title) && heads {
                lines.remove(0);
                continue;
            }
            return true;
        }
        let claimed = first
            .split(['-', '–', '—'])
            .map(str::trim)
            .filter(|part| !part.is_empty() && !artists.iter().any(|artist| loosely(part, artist)))
            .max_by_key(|part| part.len());
        if let Some(claimed) = claimed
            && !related(claimed, title)
        {
            return false;
        }
        lines.remove(0);
    }
    true
}

fn header(text: &str, title: &str, artists: &[String]) -> bool {
    labelled(text) || tagged(text) || credited(text) || named(text, title, artists)
}

fn named(line: &str, title: &str, artists: &[String]) -> bool {
    artists
        .iter()
        .any(|artist| loosely(line, artist) && (artist.chars().count() > 3 || loosely(line, title)))
}

fn only_titled(text: &str, title: &str) -> bool {
    let title = super::undecorated(title);
    !title.is_empty() && super::undecorated(text) == title
}

fn credited(text: &str) -> bool {
    let lowered = text.to_lowercase();
    RIGHTS.iter().any(|mark| lowered.contains(mark)) || attributed(&lowered) || latin_labelled(text)
}

fn attributed(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.match_indices("by")
        .filter(|(at, _)| *at == 0 || !bytes[at - 1].is_ascii_alphanumeric())
        .any(|(at, word)| {
            text[at + word.len()..]
                .trim_start_matches(' ')
                .starts_with([':', '\u{ff1a}'])
        })
}

fn latin_labelled(text: &str) -> bool {
    let Some(label) = text.split('\u{ff1a}').next().filter(|label| *label != text) else {
        return false;
    };
    label.chars().count() <= LABEL_REACH
        && label.chars().any(char::is_alphanumeric)
        && !label.chars().any(wide)
}

fn wide(letter: char) -> bool {
    matches!(letter,
        '\u{3040}'..='\u{30ff}'
        | '\u{3400}'..='\u{4dbf}'
        | '\u{4e00}'..='\u{9fff}'
        | '\u{ac00}'..='\u{d7af}')
}

fn tagged(text: &str) -> bool {
    let words: Vec<String> = text
        .split(|letter: char| !letter.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() || words.len() > 4 {
        return false;
    }
    words.iter().all(|word| TAGS.contains(&word.as_str()))
        && words.iter().any(|word| STRONG.contains(&word.as_str()))
}

fn related(claimed: &str, name: &str) -> bool {
    let words = |text: &str| {
        text.split(|letter: char| !letter.is_alphanumeric())
            .map(|word| word.to_lowercase())
            .filter(|word| word.chars().count() >= 4)
            .collect::<Vec<_>>()
    };
    let (left, right) = (words(claimed), words(name));
    if left.is_empty() || right.is_empty() {
        return true;
    }
    left.iter().any(|left| {
        right
            .iter()
            .any(|right| left.starts_with(right.as_str()) || right.starts_with(left.as_str()))
    })
}

fn labelled(text: &str) -> bool {
    let text = text.trim().to_ascii_lowercase();
    LABELS.iter().any(|label| text.starts_with(label))
}

fn loosely(haystack: &str, needle: &str) -> bool {
    let plain = |text: &str| {
        text.chars()
            .filter(|letter| letter.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let needle = plain(needle);
    !needle.is_empty() && plain(haystack).contains(&needle)
}
