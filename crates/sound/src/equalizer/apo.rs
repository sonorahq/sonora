//! Equalizer APO's configuration text, the format AutoEQ, Room EQ Wizard and most headphone
//! databases publish. `parse` reads it the way APO's `BiQuadFilterFactory` and `BiQuadFilter`
//! do, so an imported profile sounds the way it does in APO, and `export` writes a curve back.

use std::f64::consts::FRAC_1_SQRT_2;
use std::fmt::Write as _;

use super::{Curve, Filter, Invalid, Kind, MAX_FILTERS, PREAMP_RANGE, Preamp, Width};

/// The characters APO's number patterns take. A frequency also takes a non-breaking space,
/// which some locales write between thousands.
const NUMBER: &[char] = &[
    '+', '-', '.', 'e', 'E', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9',
];
const NBSP: char = '\u{a0}';
/// The S APO gives a shelf that names no width, found by experiment with Room EQ Wizard.
const SHELF_SLOPE: f64 = 0.9;
/// The Q APO gives a notch that names no width.
const NOTCH_Q: f64 = 30.;
/// The commands APO acts on that one curve for every channel cannot carry.
const UNSUPPORTED: [&str; 15] = [
    "Channel",
    "Device",
    "Stage",
    "Include",
    "Copy",
    "Delay",
    "Convolution",
    "GraphicEQ",
    "If",
    "ElseIf",
    "Else",
    "EndIf",
    "Eval",
    "VSTPlugin",
    "LoudnessCorrection",
];
/// The most bytes of text `parse` reads. Real profiles are a few kilobytes, so anything past
/// this is not one and is refused before it can stall the thread reading it.
pub const MAX_TEXT: usize = 1024 * 1024;

/// A text read into a curve, with every line the curve leaves out.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub curve: Curve,
    pub skipped: Vec<Skipped>,
}

/// A line the curve leaves out, numbered from one.
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    pub line: usize,
    pub reason: Reason,
}

/// Why a line was left out.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// A filter of a type APO does not know.
    UnknownType(String),
    /// A filter line that does not start with `ON` or `OFF` and a type.
    Unreadable,
    NoFrequency,
    /// A peak or a shelf without a gain.
    NoGain,
    /// A peak or an all-pass without a Q or a bandwidth.
    NoWidth,
    /// A value written as an expression in backticks, which Sonora does not work out.
    Expression,
    /// A preamp that is not a number.
    NoPreamp,
    /// A value outside what Sonora plays.
    Invalid(Invalid),
    /// A command or filter type APO knows that one curve for every channel cannot carry, by
    /// its name.
    Unsupported(String),
}

/// Why a text gave no curve at all.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// Neither a preamp nor a filter was read. Carries what was left out.
    Nothing(Vec<Skipped>),
    /// More filters that are on than a curve holds, by count. Dropping some would make a
    /// different curve.
    TooMany(usize),
    /// A text longer than `MAX_TEXT` bytes, left unread.
    TooLarge,
}

/// Reads an Equalizer APO configuration the way APO does: `LS` and `HS` move their corner to
/// the centre, the preamp is the sum of the `Preamp` lines, and an `OFF` filter is kept only
/// while the curve has room for it after every `ON` one.
pub fn parse(text: &str) -> Result<Parsed, Refused> {
    if text.len() > MAX_TEXT {
        return Err(Refused::TooLarge);
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut preamp: Option<(usize, f64)> = None;
    let mut filters = Vec::new();
    let mut skipped = Vec::new();

    for (index, line) in text.split('\n').enumerate() {
        let number = index + 1;
        let line = line.strip_suffix('\r').unwrap_or(line);
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.replace(',', ".");
        let read = match key.trim() {
            "Preamp" => decibels(&value)
                .map(|db| preamp = Some((number, preamp.map_or(0., |(_, sum)| sum) + db)))
                .ok_or(Reason::NoPreamp),
            key if key.starts_with("Filter") => filter(&value).map(|read| filters.extend(read)),
            "Channel" | "Device" if value.trim() == "all" => Ok(()),
            key if UNSUPPORTED.contains(&key) => Err(Reason::Unsupported(key.to_owned())),
            _ => Ok(()),
        };
        if let Err(reason) = read {
            skipped.push(Skipped {
                line: number,
                reason,
            });
        }
    }

    let on = filters.iter().filter(|filter| filter.enabled()).count();
    if on > MAX_FILTERS {
        return Err(Refused::TooMany(on));
    }
    let mut room = MAX_FILTERS - on;
    filters.retain(|filter| {
        let keep = filter.enabled() || room > 0;
        if !filter.enabled() && keep {
            room -= 1;
        }
        keep
    });
    if filters.is_empty() && preamp.is_none() {
        return Err(Refused::Nothing(skipped));
    }
    let preamp = match preamp {
        None => Preamp::Manual(0.),
        Some((_, db)) if PREAMP_RANGE.contains(&db) => Preamp::Manual(db),
        Some((line, db)) => {
            skipped.push(Skipped {
                line,
                reason: Reason::Invalid(Invalid::Preamp(db)),
            });
            Preamp::Auto
        }
    };
    skipped.sort_by_key(|skipped| skipped.line);
    let curve = Curve::new(preamp, filters).expect("the filter count and preamp are checked");
    Ok(Parsed { curve, skipped })
}

/// Writes `curve` as an Equalizer APO configuration that `parse` reads back to the same curve.
/// Shelves are written as `LSC` and `HSC`, whose frequency is the centre, and an automatic
/// preamp as the decibels it comes to, since APO has no automatic one.
pub fn export(curve: &Curve) -> String {
    let mut text = format!("Preamp: {} dB\n", curve.preamp_db());
    for (index, filter) in curve.filters().iter().enumerate() {
        let state = match filter.enabled() {
            true => "ON",
            false => "OFF",
        };
        let name = match filter.kind() {
            Kind::Peak => "PK",
            Kind::LowShelf => "LSC",
            Kind::HighShelf => "HSC",
            Kind::LowPass => "LP",
            Kind::HighPass => "HP",
            Kind::Notch => "NO",
            Kind::BandPass => "BP",
            Kind::AllPass => "AP",
        };
        let _ = write!(
            text,
            "Filter {}: {state} {name} Fc {} Hz",
            index + 1,
            hertz(filter.frequency())
        );
        if filter.kind().has_gain() {
            let _ = write!(text, " Gain {} dB", filter.gain());
        }
        let _ = match filter.width() {
            Width::Q(q) => writeln!(text, " Q {q}"),
            Width::Octaves(octaves) => writeln!(text, " BW Oct {octaves}"),
        };
    }
    text
}

/// Text decoded line by line: UTF-8 where a line is valid UTF-8, and Latin-1 where it is not,
/// the way APO falls back to the system's ANSI code page, so `1\xA0000 Hz` still reads as 1 kHz.
pub fn decode(bytes: &[u8]) -> String {
    let lines: Vec<String> = bytes
        .split(|byte| *byte == b'\n')
        .map(|line| match std::str::from_utf8(line) {
            Ok(line) => line.to_owned(),
            Err(_) => line.iter().copied().map(char::from).collect(),
        })
        .collect();
    lines.join("\n")
}

/// A frequency as APO reads it. Room EQ Wizard writes thousands with a dot, so five or more
/// characters with a dot three from the end and no exponent are thousands: `1.000` is 1 kHz.
/// Commas are read as dots before this, as APO does.
pub fn hertz_read(text: &str) -> Option<f64> {
    let text: String = text.chars().filter(|c| *c != NBSP).collect();
    let value = leading(&text)?;
    let thousands =
        text.len() >= 5 && !text.contains(['e', 'E']) && text.as_bytes()[text.len() - 4] == b'.';
    Some(match thousands {
        true => value * 1_000.,
        false => value,
    })
}

/// One `Filter` line's value, after its commas became dots. `None` for APO's `None` type, an
/// unused slot, and for an `OFF` line that does not read, since APO never reads one.
fn filter(text: &str) -> Result<Option<Filter>, Reason> {
    let (enabled, rest) = switch(text).ok_or(Reason::Unreadable)?;
    match read_filter(enabled, rest) {
        Err(_) if !enabled => Ok(None),
        read => read,
    }
}

/// The filter after its `ON` or `OFF`.
fn read_filter(enabled: bool, rest: &str) -> Result<Option<Filter>, Reason> {
    let (name, rest) = type_name(rest).ok_or(Reason::Unreadable)?;
    if name == "None" {
        return Ok(None);
    }
    let (kind, corner) = kind(name).ok_or_else(|| match name {
        "IIR" => Reason::Unsupported(name.to_owned()),
        _ => Reason::UnknownType(name.to_owned()),
    })?;
    if rest.contains('`') {
        return Err(Reason::Expression);
    }

    let frequency = field(rest, &["Fc"], &["H", "z"], true)
        .and_then(hertz_read)
        .ok_or(Reason::NoFrequency)?;
    let gain = match (field(rest, &["Gain"], &["dB"], false), kind) {
        (Some(_), Kind::LowPass | Kind::HighPass | Kind::Notch | Kind::AllPass) => 0.,
        (Some(gain), _) => leading(gain).unwrap_or(0.),
        (None, Kind::Peak | Kind::LowShelf | Kind::HighShelf) => return Err(Reason::NoGain),
        (None, _) => 0.,
    };

    // APO keeps one width and lets a bandwidth override a Q and a slope override both, each
    // only where the type takes it. A width of zero means none was given.
    let mut width = field(rest, &["Q"], &[], false).map_or(0., |q| leading(q).unwrap_or(0.));
    let mut spread = false;
    if let Some(octaves) = field(rest, &["BW", "Oct"], &[], false)
        && !kind.is_shelf()
    {
        width = leading(octaves).unwrap_or(0.);
        spread = true;
    }
    if let Some(slope) = slope(rest)
        && kind.is_shelf()
    {
        width = leading(slope).unwrap_or(0.);
        spread = true;
    }

    let (frequency, width) = match kind {
        Kind::LowShelf | Kind::HighShelf => shelf(kind, frequency, gain, width, spread, corner),
        _ => {
            let width = match (width == 0., kind) {
                (true, Kind::Peak | Kind::AllPass) => return Err(Reason::NoWidth),
                (true, Kind::Notch) => NOTCH_Q,
                (true, _) => FRAC_1_SQRT_2,
                (false, _) => width,
            };
            let width = match spread {
                true => Width::Octaves(width),
                false => Width::Q(width),
            };
            (frequency, width)
        }
    };
    Filter::new(kind, frequency, gain, width, enabled)
        .map(Some)
        .map_err(Reason::Invalid)
}

/// A shelf's centre frequency and Q from what its line gave: a Q, a slope in decibels (12 dB is
/// S = 1), or nothing, which is S = 0.9 at the frequency as written. `LS` and `HS` with a Q or a
/// slope name a corner, which APO moves to the centre by `10^(|gain| / (80·S))`.
fn shelf(
    kind: Kind,
    frequency: f64,
    gain: f64,
    width: f64,
    slope: bool,
    corner: bool,
) -> (f64, Width) {
    let a = 10f64.powf(gain / 40.);
    let (q, s) = match (width == 0., slope) {
        (true, _) => (None, SHELF_SLOPE),
        (false, true) => (None, width / 12.),
        (false, false) => (
            Some(width),
            1. / ((1. / (width * width) - 2.) / (a + 1. / a) + 1.),
        ),
    };
    let shift = 10f64.powf(gain.abs() / 80. / s);
    let frequency = match (corner && width != 0., kind) {
        (false, _) => frequency,
        (true, Kind::LowShelf) => frequency * shift,
        (true, _) => frequency / shift,
    };
    let q = q.unwrap_or_else(|| 1. / ((a + 1. / a) * (1. / s - 1.) + 2.).sqrt());
    (frequency, Width::Q(q))
}

/// Whether a filter line is `ON` or `OFF`, and what follows that.
fn switch(text: &str) -> Option<(bool, &str)> {
    let text = text.trim_start();
    match (text.strip_prefix("ON"), text.strip_prefix("OFF")) {
        (Some(rest), _) => Some((true, rest)),
        (_, Some(rest)) => Some((false, rest)),
        _ => None,
    }
}

/// The type name after the switch and the rest of the line, as APO's `ON\s+([A-Za-z]+)`
/// reads them. `OFF` is read the same way, as a filter that is off.
fn type_name(rest: &str) -> Option<(&str, &str)> {
    let name = rest.trim_start();
    if name.len() == rest.len() {
        return None;
    }
    let end = name
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(name.len());
    (end > 0).then(|| (&name[..end], &name[end..]))
}

/// A type name as APO reads it, and whether a shelf's frequency is its corner rather than its
/// centre, which is every shelf name not ending in `C`.
fn kind(name: &str) -> Option<(Kind, bool)> {
    let kind = match name {
        "PK" | "PEQ" | "Modal" => Kind::Peak,
        "LP" | "LPQ" => Kind::LowPass,
        "HP" | "HPQ" => Kind::HighPass,
        "BP" => Kind::BandPass,
        "LS" | "LSC" => Kind::LowShelf,
        "HS" | "HSC" => Kind::HighShelf,
        "NO" => Kind::Notch,
        "AP" => Kind::AllPass,
        _ => return None,
    };
    Some((kind, kind.is_shelf() && !name.ends_with('C')))
}

/// The number in the first place `words` stand after whitespace and the number is followed by
/// `unit`, as APO's `\s+Fc\s*([-+0-9.eE]+)\s*H\s*z` and its siblings find it. Whitespace
/// between `words` is required, between the parts of `unit` only allowed.
fn field<'a>(text: &'a str, words: &[&str], unit: &[&str], frequency: bool) -> Option<&'a str> {
    text.match_indices(words[0]).find_map(|(at, _)| {
        text[..at]
            .chars()
            .next_back()
            .filter(|c| c.is_whitespace())?;
        let rest = spaced(&text[at..], words, true)?;
        number(rest, unit, frequency)
    })
}

/// A shelf's slope in decibels, the number right after the type in `LS 6dB`.
fn slope(text: &str) -> Option<&str> {
    number(text, &["dB"], false)
}

/// The number at the start of `text`, after any whitespace, when `unit` follows it.
fn number<'a>(text: &'a str, unit: &[&str], frequency: bool) -> Option<&'a str> {
    let text = text.trim_start();
    let end = text
        .find(|c: char| !(NUMBER.contains(&c) || (frequency && c == NBSP)))
        .unwrap_or(text.len());
    let (number, rest) = text.split_at(end);
    (end > 0 && spaced(rest.trim_start(), unit, false).is_some()).then_some(number)
}

/// The text after `parts`, one after the other, with whitespace between them that `apart`
/// requires or else only allows.
fn spaced<'a>(text: &'a str, parts: &[&str], apart: bool) -> Option<&'a str> {
    let mut rest = text;
    for (index, part) in parts.iter().enumerate() {
        if index > 0 {
            let trimmed = rest.trim_start();
            if apart && trimmed.len() == rest.len() {
                return None;
            }
            rest = trimmed;
        }
        rest = rest.strip_prefix(part)?;
    }
    Some(rest)
}

/// The longest start of `text` that is a decimal number, the way C's `wcstod` reads one. It
/// scans the sign, digits, dot and exponent once and parses only that span.
fn leading(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let digits = |from: usize| {
        bytes[from..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count()
    };
    let mut end = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let whole = digits(end);
    end += whole;
    let mut fraction = 0;
    if bytes.get(end) == Some(&b'.') {
        fraction = digits(end + 1);
        end += 1 + fraction;
    }
    if whole + fraction == 0 {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(end + 1), Some(b'+' | b'-')));
        let exponent = digits(end + 1 + sign);
        if exponent > 0 {
            end += 1 + sign + exponent;
        }
    }
    text[..end].parse().ok()
}

/// A preamp value as APO's `" %lf dB"` reads it, the unit optional.
fn decibels(text: &str) -> Option<f64> {
    let text = text.trim_start();
    let end = text
        .find(|c: char| !NUMBER.contains(&c))
        .unwrap_or(text.len());
    leading(&text[..end])
}

/// A frequency written so `hertz_read` reads it back unchanged: one more digit where the dot
/// would otherwise sit where a thousands separator does.
fn hertz(frequency: f64) -> String {
    let text = frequency.to_string();
    match text.len() >= 5 && text.as_bytes()[text.len() - 4] == b'.' {
        true => text + "0",
        false => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AutoEQ's profile for the Sennheiser HD 650, measured by oratory1990, as published at
    /// `results/oratory1990/over-ear/Sennheiser HD 650/Sennheiser HD 650 ParametricEQ.txt` in
    /// github.com/jaakkopasanen/AutoEq.
    const HD_650: &str = "Preamp: -6.1 dB
Filter 1: ON LSC Fc 105 Hz Gain 6.4 dB Q 0.70
Filter 2: ON PK Fc 8800 Hz Gain 5.1 dB Q 1.42
Filter 3: ON PK Fc 118 Hz Gain -3.1 dB Q 0.50
Filter 4: ON PK Fc 37 Hz Gain 0.7 dB Q 3.96
Filter 5: ON PK Fc 3169 Hz Gain -1.7 dB Q 3.89
Filter 6: ON HSC Fc 10000 Hz Gain -2.1 dB Q 0.70
Filter 7: ON PK Fc 1227 Hz Gain -1.2 dB Q 2.53
Filter 8: ON PK Fc 2055 Hz Gain 1.2 dB Q 3.23
Filter 9: ON PK Fc 587 Hz Gain 0.4 dB Q 1.19
Filter 10: ON PK Fc 5332 Hz Gain -1.1 dB Q 5.75
";

    #[test]
    fn reads_and_writes_equalizer_apo_text() {
        use Kind::*;
        let q = |kind, frequency, gain, q| {
            Ok(Filter::new(kind, frequency, gain, Width::Q(q), true).unwrap())
        };
        let lines = [
            ("ON PK Fc 1000 Hz Gain 3 dB Q 2", q(Peak, 1_000., 3., 2.)),
            ("ON PEQ Fc 100 Hz Gain 1 dB Q 1", q(Peak, 100., 1., 1.)),
            // A shelf without a width is S = 0.9 at the frequency as written, LS or LSC.
            (
                "ON LS Fc 300 Hz Gain 6 dB",
                q(LowShelf, 300., 6., 0.6688089099801692),
            ),
            (
                "ON LS 6dB Fc 300 Hz Gain 6 dB",
                q(LowShelf, 423.76126338682633, 6., 0.49263574367414176),
            ),
            (
                "ON LSC Fc 100 Hz Gain 3 dB BW Oct 2",
                q(LowShelf, 100., 3., 0.6703195605671526),
            ),
            (
                "ON PK Fc 1000 Hz Gain 3 dB Q 2 BW Oct 1",
                Ok(Filter::new(Peak, 1_000., 3., Width::Octaves(1.), true).unwrap()),
            ),
            (
                "ON PK 12 dB Fc 1000 Hz Gain 3 dB Q 2",
                q(Peak, 1_000., 3., 2.),
            ),
            (
                "ON LP Fc 1000 Hz Q 0",
                q(LowPass, 1_000., 0., FRAC_1_SQRT_2),
            ),
            ("ON HP Fc 80 Hz Gain 6 dB Q 0.5", q(HighPass, 80., 0., 0.5)),
            ("ON BP Fc 80 Hz Gain 99 dB Q 0.5", q(BandPass, 80., 0., 0.5)),
            ("ON NO Fc 60 Hz", q(Notch, 60., 0., 30.)),
            (
                "ON PK Fc 1,000 Hz Gain -1,5 dB Q 1,41",
                q(Peak, 1_000., -1.5, 1.41),
            ),
            ("ON PK Fc 2.500 Hz Gain 1 dB Q 1", q(Peak, 2_500., 1., 1.)),
            ("ON PK Fc 12.5 Hz Gain 1 dB Q 1", q(Peak, 12.5, 1., 1.)),
            (
                "ON PK Fc 1\u{a0}000 Hz Gain 1 dB Q 1",
                q(Peak, 1_000., 1., 1.),
            ),
            (
                "OFF PK Fc 500 Hz Gain 2 dB Q 1",
                Ok(Filter::new(Peak, 500., 2., Width::Q(1.), false).unwrap()),
            ),
            ("ON PK Fc 1000 Hz Gain 3 dB Q 0", Err(Reason::NoWidth)),
            ("ON PK Fc 1000 Hz Q 1", Err(Reason::NoGain)),
            ("ON PK Gain 1 dB Q 1", Err(Reason::NoFrequency)),
            ("ON XX Fc 100 Hz", Err(Reason::UnknownType("XX".to_owned()))),
            ("ON IIR Order 2", Err(Reason::Unsupported("IIR".to_owned()))),
            ("PK Fc 100 Hz Gain 1 dB Q 1", Err(Reason::Unreadable)),
            (
                "ON PK Fc 30000 Hz Gain 1 dB Q 1",
                Err(Reason::Invalid(Invalid::Frequency(30_000.))),
            ),
        ];
        for (line, expected) in lines {
            let read = match parse(&format!("Filter: {line}")) {
                Ok(parsed) => Ok(parsed.curve.filters()[0]),
                Err(Refused::Nothing(skipped)) => Err(skipped[0].reason.clone()),
                Err(refused) => panic!("{line}: {refused:?}"),
            };
            match (read, expected) {
                (Ok(read), Ok(expected)) => {
                    assert_eq!(read.kind(), expected.kind(), "{line}");
                    assert_eq!(read.enabled(), expected.enabled(), "{line}");
                    for (read, expected) in [
                        (read.frequency(), expected.frequency()),
                        (read.gain(), expected.gain()),
                        (read.width().value(), expected.width().value()),
                    ] {
                        assert!(
                            (read - expected).abs() <= 1e-9 * expected.abs(),
                            "{line}: {read} vs {expected}"
                        );
                    }
                }
                (read, expected) => assert_eq!(read, expected, "{line}"),
            }
        }

        // An LS with a Q names its corner, an LSC its centre (oracle: +3.963 vs +3.000 dB).
        for (line, at_300) in [
            ("ON LS Fc 300 Hz Gain 6 dB Q 0.7", 3.963),
            ("ON LSC Fc 300 Hz Gain 6 dB Q 0.7", 3.),
        ] {
            let curve = parse(&format!("Filter: {line}")).unwrap().curve;
            assert!(
                (curve.response(48_000).db(300.) - at_300).abs() < 5e-4,
                "{line}"
            );
        }

        let hd_650 = parse(HD_650).unwrap();
        assert!(hd_650.skipped.is_empty());
        assert_eq!(hd_650.curve.preamp(), Preamp::Manual(-6.1));
        assert_eq!(hd_650.curve.filters().len(), 10);
        assert_eq!(
            hd_650.curve.filters()[0],
            Filter::new(LowShelf, 105., 6.4, Width::Q(0.7), true).unwrap()
        );
        assert_eq!(
            hd_650.curve.filters()[5],
            Filter::new(HighShelf, 10_000., -2.1, Width::Q(0.7), true).unwrap()
        );
        let windows = format!("\u{feff}{}", HD_650.replace('\n', "\r\n"));
        assert_eq!(parse(&windows).unwrap(), hd_650);

        // AutoEQ's preamp is the highest boost on its 20 Hz × 1.02ⁿ grid at 44.1 kHz.
        let response = hd_650.curve.response(44_100);
        let peak = std::iter::successors(Some(20.), |f| Some(f * 1.02))
            .take_while(|f| *f <= 20_000.)
            .map(|f| response.db(f))
            .fold(f64::MIN, f64::max);
        assert!((peak - 6.0698).abs() < 5e-5, "{peak}");

        let summed =
            parse("Preamp: -3 dB\nPreamp: -1,5\nChannel: all\nGraphicEQ: 20 0; 40 1").unwrap();
        assert_eq!(summed.curve.preamp(), Preamp::Manual(-4.5));
        assert_eq!(
            summed.skipped,
            [Skipped {
                line: 4,
                reason: Reason::Unsupported("GraphicEQ".to_owned())
            }]
        );
        assert_eq!(
            parse("# a comment\nGraphicEQ: 20 0"),
            Err(Refused::Nothing(vec![Skipped {
                line: 2,
                reason: Reason::Unsupported("GraphicEQ".to_owned())
            }]))
        );
        let on = |n| format!("Filter {n}: ON PK Fc {} Hz Gain 1 dB Q 1\n", 100 * n);
        let crowded: String = (1..=21).map(on).collect();
        assert_eq!(parse(&crowded), Err(Refused::TooMany(21)));
        // APO plays only ON filters: an OFF one past the room, or one that does not read, goes.
        let full = (1..=20).map(on).collect::<String>()
            + "Filter: OFF PK Fc 50 Hz Gain 1 dB Q 1\nFilter: OFF XX\n";
        let full = parse(&full).unwrap();
        assert!(full.skipped.is_empty());
        assert_eq!(full.curve.filters().len(), MAX_FILTERS);
        assert!(full.curve.filters().iter().all(Filter::enabled));

        let mut every = Kind::ALL
            .map(|kind| Filter::new(kind, 12.345, -7.25, Width::Q(0.707), true).unwrap())
            .to_vec();
        every.push(Filter::new(Notch, 1_234.567, 0., Width::Octaves(0.333), false).unwrap());
        every.push(
            Filter::new(
                Peak,
                19_999.999,
                1. / 3.,
                Width::Q(std::f64::consts::PI),
                true,
            )
            .unwrap(),
        );
        let curve = Curve::new(Preamp::Manual(-6.0698), every).unwrap();
        assert_eq!(
            parse(&export(&curve)).unwrap(),
            Parsed {
                curve,
                skipped: Vec::new()
            }
        );
    }
}
