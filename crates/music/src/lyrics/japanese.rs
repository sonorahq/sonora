const READINGS: &[(&str, &str, &str)] = &[
    ("君", "", "きみ"),
    ("月", "", "つき"),
    ("人", "", "ひと"),
    ("名", "", "な"),
    ("日", "", "ひ"),
    ("愛", "して", "あい"),
    ("愛", "した", "あい"),
    ("愛", "しま", "あい"),
    ("降", "っ", "ふ"),
];

const CASE: &[&str] = &[
    "から", "まで", "より", "では", "には", "とは", "は", "が", "を", "に", "へ", "と", "の",
];

const AUX: &[&str] = &[
    "なんだ",
    "なんて",
    "なんか",
    "だから",
    "だけど",
    "だけ",
    "でも",
    "ので",
    "のに",
    "けど",
    "です",
    "だ",
    "で",
];

const FINALS: &[char] = &['よ', 'ね'];

const LONE: &[&str] = &[
    "も", "や", "か", "ね", "よ", "さ", "な", "わ", "ぞ", "ば", "ら",
];

const KEEP: &[&str] = &[
    "こと", "もの", "ひと", "あと", "そと", "おと", "もと", "なに", "みの", "その",
];

const TAILS: &[(&str, &str)] = &[("では", "dewa"), ("には", "niwa"), ("とは", "towa")];

const WORDS: &[(&str, &str)] = &[
    ("は", "wa"),
    ("へ", "e"),
    ("を", "o"),
    ("こんにちは", "konnichiwa"),
    ("こんばんは", "konbanwa"),
];

const MARKS: &[(char, &str)] = &[
    ('、', ","),
    ('。', "."),
    ('，', ","),
    ('．', "."),
    ('！', "!"),
    ('？', "?"),
    ('：', ":"),
    ('；', ";"),
    ('（', "("),
    ('）', ")"),
    ('「', "\""),
    ('」', "\""),
    ('『', "\""),
    ('』', "\""),
    ('・', " "),
];

const HEPBURN: &[(&str, &str)] = &[
    ("きゃ", "kya"),
    ("きゅ", "kyu"),
    ("きょ", "kyo"),
    ("しゃ", "sha"),
    ("しゅ", "shu"),
    ("しょ", "sho"),
    ("ちゃ", "cha"),
    ("ちゅ", "chu"),
    ("ちょ", "cho"),
    ("にゃ", "nya"),
    ("にゅ", "nyu"),
    ("にょ", "nyo"),
    ("ひゃ", "hya"),
    ("ひゅ", "hyu"),
    ("ひょ", "hyo"),
    ("みゃ", "mya"),
    ("みゅ", "myu"),
    ("みょ", "myo"),
    ("りゃ", "rya"),
    ("りゅ", "ryu"),
    ("りょ", "ryo"),
    ("ぎゃ", "gya"),
    ("ぎゅ", "gyu"),
    ("ぎょ", "gyo"),
    ("じゃ", "ja"),
    ("じゅ", "ju"),
    ("じょ", "jo"),
    ("ぢゃ", "ja"),
    ("ぢゅ", "ju"),
    ("ぢょ", "jo"),
    ("びゃ", "bya"),
    ("びゅ", "byu"),
    ("びょ", "byo"),
    ("ぴゃ", "pya"),
    ("ぴゅ", "pyu"),
    ("ぴょ", "pyo"),
    ("ふぁ", "fa"),
    ("ふぃ", "fi"),
    ("ふぇ", "fe"),
    ("ふぉ", "fo"),
    ("ふゅ", "fyu"),
    ("うぃ", "wi"),
    ("うぇ", "we"),
    ("うぉ", "wo"),
    ("ゔぁ", "va"),
    ("ゔぃ", "vi"),
    ("ゔぇ", "ve"),
    ("ゔぉ", "vo"),
    ("てぃ", "ti"),
    ("でぃ", "di"),
    ("とぅ", "tu"),
    ("どぅ", "du"),
    ("しぇ", "she"),
    ("じぇ", "je"),
    ("ちぇ", "che"),
    ("つぁ", "tsa"),
    ("つぃ", "tsi"),
    ("つぇ", "tse"),
    ("つぉ", "tso"),
    ("あ", "a"),
    ("い", "i"),
    ("う", "u"),
    ("え", "e"),
    ("お", "o"),
    ("か", "ka"),
    ("き", "ki"),
    ("く", "ku"),
    ("け", "ke"),
    ("こ", "ko"),
    ("さ", "sa"),
    ("し", "shi"),
    ("す", "su"),
    ("せ", "se"),
    ("そ", "so"),
    ("た", "ta"),
    ("ち", "chi"),
    ("つ", "tsu"),
    ("て", "te"),
    ("と", "to"),
    ("な", "na"),
    ("に", "ni"),
    ("ぬ", "nu"),
    ("ね", "ne"),
    ("の", "no"),
    ("は", "ha"),
    ("ひ", "hi"),
    ("ふ", "fu"),
    ("へ", "he"),
    ("ほ", "ho"),
    ("ま", "ma"),
    ("み", "mi"),
    ("む", "mu"),
    ("め", "me"),
    ("も", "mo"),
    ("や", "ya"),
    ("ゆ", "yu"),
    ("よ", "yo"),
    ("ら", "ra"),
    ("り", "ri"),
    ("る", "ru"),
    ("れ", "re"),
    ("ろ", "ro"),
    ("わ", "wa"),
    ("ゐ", "i"),
    ("ゑ", "e"),
    ("を", "o"),
    ("が", "ga"),
    ("ぎ", "gi"),
    ("ぐ", "gu"),
    ("げ", "ge"),
    ("ご", "go"),
    ("ざ", "za"),
    ("じ", "ji"),
    ("ず", "zu"),
    ("ぜ", "ze"),
    ("ぞ", "zo"),
    ("だ", "da"),
    ("ぢ", "ji"),
    ("づ", "zu"),
    ("で", "de"),
    ("ど", "do"),
    ("ば", "ba"),
    ("び", "bi"),
    ("ぶ", "bu"),
    ("べ", "be"),
    ("ぼ", "bo"),
    ("ぱ", "pa"),
    ("ぴ", "pi"),
    ("ぷ", "pu"),
    ("ぺ", "pe"),
    ("ぽ", "po"),
    ("ゔ", "vu"),
    ("ぁ", "a"),
    ("ぃ", "i"),
    ("ぅ", "u"),
    ("ぇ", "e"),
    ("ぉ", "o"),
    ("ゃ", "ya"),
    ("ゅ", "yu"),
    ("ょ", "yo"),
    ("ゎ", "wa"),
];

/// Groups source characters by how they join romanized words.
#[derive(Clone, Copy, PartialEq)]
enum Script {
    Kanji,
    Kata,
    Kana,
    Mark,
    Open,
    Gap,
}

impl Script {
    fn content(self) -> bool {
        matches!(self, Self::Kanji | Self::Kata)
    }
}

/// Keeps a source run with its contextual reading and attachment to the following kana.
struct Piece {
    script: Script,
    source: String,
    kana: String,
    bound: bool,
}

pub(super) fn romanize(text: &str) -> String {
    let Some(pieces) = pieces(text) else {
        let kana = kakasi::convert(text).hiragana;
        return polish(
            &kana
                .split_whitespace()
                .map(roman)
                .collect::<Vec<_>>()
                .join(" "),
        );
    };
    let words = words(&pieces);
    polish(
        &words
            .iter()
            .map(|word| roman(word))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn kanji(letter: char) -> bool {
    matches!(letter as u32, 0x3005 | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff)
}

pub(super) fn kana(letter: char) -> bool {
    hiragana(letter) || katakana(letter)
}

fn katakana(letter: char) -> bool {
    matches!(letter as u32, 0x30a1..=0x30fa | 0xff66..=0xff9d)
}

fn hiragana(letter: char) -> bool {
    matches!(letter as u32, 0x3041..=0x309f)
}

/// Separates opening punctuation so it joins the following word rather than the preceding one.
fn script_of(letter: char) -> Script {
    match letter {
        _ if letter.is_whitespace() => Script::Gap,
        '「' | '『' | '（' | '(' | '[' | '［' | '{' | '｛' => Script::Open,
        _ if kanji(letter) => Script::Kanji,
        _ if katakana(letter) || letter == 'ー' => Script::Kata,
        _ if hiragana(letter) => Script::Kana,
        _ => Script::Mark,
    }
}

fn runs(source: &str) -> Vec<(usize, usize, Script)> {
    let mut runs: Vec<(usize, usize, Script)> = Vec::new();
    for (index, letter) in source.char_indices() {
        let script = script_of(letter);
        match runs.last_mut() {
            Some((_, end, last)) if *last == script && script != Script::Gap => {
                *end = index + letter.len_utf8();
            }
            _ => runs.push((index, index + letter.len_utf8(), script)),
        }
    }
    runs
}

fn pieces(source: &str) -> Option<Vec<Piece>> {
    let runs = runs(source);
    let mut suffixes: Vec<String> = runs
        .iter()
        .map(|(start, _, _)| kakasi::convert(&source[*start..]).hiragana)
        .collect();
    suffixes.push(String::new());

    let mut pieces: Vec<Piece> = Vec::with_capacity(runs.len());
    for (index, (start, end, script)) in runs.iter().enumerate() {
        let take = suffixes[index]
            .chars()
            .count()
            .checked_sub(suffixes[index + 1].chars().count())?;
        let source = source[*start..*end].to_owned();
        let kana: String = suffixes[index].chars().take(take).collect();
        let bound = *script == Script::Kanji && kakasi::convert(&source).hiragana != kana;
        pieces.push(Piece {
            script: *script,
            source,
            kana,
            bound,
        });
    }

    let joined: String = pieces.iter().map(|piece| piece.kana.as_str()).collect();
    if joined != kakasi::convert(source).hiragana {
        return None;
    }

    for index in 0..pieces.len() {
        if let Some(reading) = reading(&pieces, index) {
            pieces[index].kana = reading;
        }
    }
    Some(pieces)
}

fn reading(pieces: &[Piece], index: usize) -> Option<String> {
    let piece = &pieces[index];
    if piece.script != Script::Kanji {
        return None;
    }
    if pieces
        .get(index.wrapping_sub(1))
        .and_then(|before| before.source.chars().next_back())
        .is_some_and(|letter| letter.is_ascii_digit())
    {
        return None;
    }
    let next = pieces
        .get(index + 1)
        .filter(|next| next.script == Script::Kana)
        .map(|next| next.kana.as_str())
        .unwrap_or_default();
    READINGS
        .iter()
        .find(|(kanji, after, _)| *kanji == piece.source && next.starts_with(after))
        .map(|(_, _, reading)| (*reading).to_owned())
}

/// Groups contextual readings into words while keeping punctuation beside its text.
fn words(pieces: &[Piece]) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut previous: Option<&Piece> = None;
    for piece in pieces {
        match piece.script {
            Script::Gap => {}
            Script::Open => words.push(piece.source.clone()),
            Script::Mark => match piece.source.chars().any(char::is_alphanumeric) {
                true => words.push(piece.source.clone()),
                false => attach(&mut words, &piece.source),
            },
            Script::Kanji | Script::Kata => words.push(piece.kana.clone()),
            Script::Kana => {
                let follows = previous.is_some_and(|before| before.script.content());
                let bound = previous.is_some_and(|before| before.bound);
                let mut fresh = !follows;
                for part in around(&piece.kana) {
                    match part == "を" {
                        true => {
                            words.push(part);
                            fresh = true;
                        }
                        false => match fresh {
                            true => words.push(part),
                            false => split(&part, &mut words, bound),
                        },
                    }
                }
            }
        }
        previous = Some(piece);
    }
    joined(words)
}

fn joined(words: Vec<String>) -> Vec<String> {
    let mut merged: Vec<String> = Vec::with_capacity(words.len());
    for word in words {
        match merged.last().is_some_and(|last| opening(last)) {
            true => {
                let head = merged.pop().unwrap_or_default();
                merged.push(head + &word);
            }
            false => merged.push(word),
        }
    }
    merged
}

/// Recognizes punctuation that attaches to the following word.
fn opening(word: &str) -> bool {
    word.chars().all(|letter| {
        matches!(
            letter,
            '「' | '『' | '（' | '(' | '[' | '［' | '{' | '｛' | '"'
        )
    })
}

fn around(run: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = run;
    while let Some(at) = rest.find('を') {
        if at > 0 {
            parts.push(rest[..at].to_owned());
        }
        parts.push("を".to_owned());
        rest = &rest[at + 'を'.len_utf8()..];
    }
    if !rest.is_empty() {
        parts.push(rest.to_owned());
    }
    parts
}

fn split(run: &str, words: &mut Vec<String>, bound: bool) {
    if LONE.contains(&run) {
        words.push(run.to_owned());
        return;
    }
    let mut rest = run;
    let mut opened = false;
    if !bound {
        if let Some(particle) = CASE.iter().find(|particle| rest.starts_with(**particle)) {
            words.push((*particle).to_owned());
            rest = &rest[particle.len()..];
            opened = true;
        } else if let Some(particle) = AUX
            .iter()
            .find(|particle| rest.starts_with(**particle))
            .filter(|particle| particle.chars().count() > 1 || detachable(words))
        {
            let finals: String = rest[particle.len()..]
                .chars()
                .take_while(|letter| FINALS.contains(letter))
                .collect();
            words.push(format!("{particle}{finals}"));
            rest = &rest[particle.len() + finals.len()..];
            opened = true;
        }
    }

    let (body, particles) = peeled(rest, opened);
    if !body.is_empty() {
        match opened {
            true => words.push(body.to_owned()),
            false => attach(words, body),
        }
    }
    words.extend(particles);
}

fn peeled(run: &str, opened: bool) -> (&str, Vec<String>) {
    let mut body = run;
    let mut particles: Vec<String> = Vec::new();
    while let Some(particle) = peelable(body, opened) {
        body = &body[..body.len() - particle.len()];
        particles.insert(0, particle.to_owned());
        if particle == "だ" || peelable(body, opened) != Some("だ") {
            break;
        }
    }
    (body, particles)
}

fn peelable(run: &str, opened: bool) -> Option<&'static str> {
    let particle = CASE
        .iter()
        .chain(std::iter::once(&"だ"))
        .find(|particle| run.ends_with(**particle))?;
    let body = &run[..run.len() - particle.len()];
    let last = body.chars().next_back();
    let joins = matches!(last, Some('ん') | Some('っ'));
    let vowel = last == Some('い') && *particle == "だ";
    let stranded = opened && !body.is_empty() && body.chars().count() < 2;
    let word = last.is_some_and(|letter| KEEP.contains(&format!("{letter}{particle}").as_str()));
    (!joins && !vowel && !stranded && !word).then_some(*particle)
}

fn attach(words: &mut Vec<String>, text: &str) {
    match words.last_mut() {
        Some(last) => last.push_str(text),
        None => words.push(text.to_owned()),
    }
}

fn detachable(words: &[String]) -> bool {
    words
        .last()
        .and_then(|word| word.chars().last())
        .is_some_and(|letter| !matches!(letter, 'ん' | 'っ'))
}

fn roman(word: &str) -> String {
    if let Some((_, romaji)) = WORDS.iter().find(|(kana, _)| *kana == word) {
        return (*romaji).to_owned();
    }
    if let Some((tail, romaji)) = TAILS.iter().find(|(tail, _)| word.ends_with(*tail)) {
        return format!("{}{romaji}", roman(&word[..word.len() - tail.len()]));
    }
    let letters: Vec<char> = word.chars().collect();
    let mut romaji = String::with_capacity(word.len());
    let mut index = 0;
    while index < letters.len() {
        match letters[index] {
            'っ' | 'ッ' => {
                match syllable(&letters, index + 1) {
                    Some(next) if next.starts_with("ch") => romaji.push('t'),
                    Some(next) => {
                        if let Some(letter) = next
                            .chars()
                            .next()
                            .filter(|letter| !matches!(letter, 'a' | 'i' | 'u' | 'e' | 'o'))
                        {
                            romaji.push(letter);
                        }
                    }
                    None => {}
                }
                index += 1;
            }
            'ん' | 'ン' => {
                romaji.push('n');
                if syllable(&letters, index + 1)
                    .and_then(|next| next.chars().next())
                    .is_some_and(|letter| matches!(letter, 'a' | 'i' | 'u' | 'e' | 'o' | 'y'))
                {
                    romaji.push('\'');
                }
                index += 1;
            }
            'ー' => {
                if let Some(last) = romaji.chars().last() {
                    romaji.push(last);
                }
                index += 1;
            }
            letter => {
                let pair: String = letters[index..(index + 2).min(letters.len())]
                    .iter()
                    .collect();
                match HEPBURN.iter().find(|(kana, _)| *kana == pair) {
                    Some((_, sound)) => {
                        romaji.push_str(sound);
                        index += 2;
                    }
                    None => {
                        match HEPBURN.iter().find(|(kana, _)| *kana == letter.to_string()) {
                            Some((_, sound)) => romaji.push_str(sound),
                            None => match MARKS.iter().find(|(mark, _)| *mark == letter) {
                                Some((_, ascii)) => romaji.push_str(ascii),
                                None => romaji.push(letter),
                            },
                        }
                        index += 1;
                    }
                }
            }
        }
    }
    romaji
}

fn syllable(letters: &[char], index: usize) -> Option<&'static str> {
    let pair: String = letters
        .get(index..(index + 2).min(letters.len()))?
        .iter()
        .collect();
    let single = letters.get(index)?.to_string();
    HEPBURN
        .iter()
        .find(|(kana, _)| *kana == pair)
        .or_else(|| HEPBURN.iter().find(|(kana, _)| *kana == single))
        .map(|(_, sound)| *sound)
}

fn polish(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ,", ",")
        .replace(" .", ".")
        .replace(" !", "!")
        .replace(" ?", "?")
}
