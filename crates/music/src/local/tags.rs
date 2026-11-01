use std::fs::File;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::id3::v2::{Frame, SyncTextContentType, SynchronizedTextFrame, TimestampFormat};
use lofty::mpeg::MpegFile;
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{ItemValue, Tag, TagItem};

use crate::engine::Loudness;
use crate::lyrics::{lrc, ttml};
use crate::{Encoding, Lyrics, LyricsLine, LyricsWord, TrackTags, Voice};

use super::wire;

const BREAKS: [char; 2] = ['\n', '\r'];

pub fn read(path: &Path) -> Result<TrackTags> {
    let tagged = Probe::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .read()
        .with_context(|| format!("cannot read the tags in {}", path.display()))?;
    let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) else {
        return Ok(TrackTags::default());
    };

    Ok(TrackTags {
        title: text(tag.title()),
        artists: wire::artists(tag),
        album: text(tag.album()),
        album_artist: held(tag, ItemKey::AlbumArtist),
        track_number: number(tag.track()),
        track_total: number(tag.track_total()),
        disc_number: number(tag.disk()),
        disc_total: number(tag.disk_total()),
        year: tag
            .date()
            .filter(|date| date.year > 0)
            .map(|date| date.year.to_string())
            .unwrap_or_default(),
        genre: text(tag.genre()),
        composer: held(tag, ItemKey::Composer),
        publisher: held(tag, ItemKey::Publisher),
        isrc: held(tag, ItemKey::Isrc),
        comment: text(tag.comment()),
        lyrics: held(tag, ItemKey::Lyrics),
    })
}

/// A file's tags and properties without its cover art, for `encoding` and `loudness`.
pub fn tagged(path: &Path) -> Option<TaggedFile> {
    Probe::open(path)
        .ok()?
        .options(ParseOptions::new().read_cover_art(false))
        .read()
        .ok()
}

/// How a file is encoded, with the bit depth of a lossless codec or the bit rate of a lossy one.
/// `None` when the file leaves out the figure its codec is shown with.
pub fn encoding(tagged: &TaggedFile) -> Option<Encoding> {
    let properties = tagged.properties();
    let depth = properties.bit_depth();
    let kbps = properties.audio_bitrate();
    Some(match tagged.file_type() {
        FileType::Flac => Encoding::Flac {
            depth: depth?,
            rate: properties.sample_rate()?,
        },
        FileType::Wav | FileType::Aiff => Encoding::Pcm { depth: depth? },
        FileType::Mpeg => Encoding::Mp3 { kbps: kbps? },
        FileType::Aac => Encoding::Aac { kbps: kbps? },
        // An MP4 holds AAC or ALAC, and only ALAC has a bit depth.
        FileType::Mp4 => match depth {
            Some(depth) => Encoding::Alac { depth },
            None => Encoding::Aac { kbps: kbps? },
        },
        FileType::Vorbis => Encoding::Vorbis { kbps: kbps? },
        FileType::Opus => Encoding::Opus { kbps: kbps? },
        _ => Encoding::Other,
    })
}

/// The ReplayGain a file's tags carry, from whichever of its tags has it. The track gain wins,
/// and the album gain stands in when that is all the file was tagged with.
pub fn loudness(tagged: &TaggedFile) -> Option<Loudness> {
    tagged.tags().iter().find_map(|tag| {
        let read = |key| tag.get_string(key).and_then(decibels);
        let (gain, peak) = match read(ItemKey::ReplayGainTrackGain) {
            Some(gain) => (gain, ItemKey::ReplayGainTrackPeak),
            None => (
                read(ItemKey::ReplayGainAlbumGain)?,
                ItemKey::ReplayGainAlbumPeak,
            ),
        };
        let peak = tag
            .get_string(peak)
            .and_then(|peak| peak.trim().parse().ok());
        Some(Loudness::replay_gain(gain, peak))
    })
}

/// A timed `SYLT` frame wins over the lyrics text, which is timed when it parses as TTML or LRC.
pub fn lyrics(path: &Path) -> Result<Option<Lyrics>> {
    if let Some(lines) = synchronized(path) {
        return Ok(Some(Lyrics::Synced {
            lines: lines.into(),
        }));
    }
    let text = read(path)?.lyrics;
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if let Some(lyrics) = ttml_lyrics(path, text) {
        return Ok(Some(lyrics));
    }
    let lines = lrc::parse(text);
    Ok(Some(match lines.is_empty() {
        true => Lyrics::plain(text),
        false => Lyrics::Synced {
            lines: lines.into(),
        },
    }))
}

/// Decodes text that looks like a TTML document, as Apple Music downloaders store in the tags and
/// beside the track. A document that does not parse or holds no line is logged and gives `None`.
pub(super) fn ttml_lyrics(origin: &Path, text: &str) -> Option<Lyrics> {
    let text = text.trim_start_matches('\u{feff}').trim_start();
    if !text.starts_with("<tt") && !text.starts_with("<?xml") {
        return None;
    }
    let lyrics = match ttml::parse(text) {
        Ok(sheet) => sheet.lyrics,
        Err(error) => {
            log::warn!(
                "lyrics: cannot decode the TTML of {}: {error:#}",
                origin.display()
            );
            return None;
        }
    };
    let empty = match &lyrics {
        Lyrics::Plain { text, .. } => text.trim().is_empty(),
        Lyrics::Synced { lines } => lines.is_empty(),
    };
    if empty {
        log::warn!("lyrics: the TTML of {} holds no line", origin.display());
        return None;
    }
    Some(lyrics)
}

/// Only millisecond stamps are read: MPEG frame stamps would need the stream's frame length.
fn synchronized(path: &Path) -> Option<Vec<LyricsLine>> {
    let probe = Probe::open(path).ok()?.guess_file_type().ok()?;
    if probe.file_type() != Some(FileType::Mpeg) {
        return None;
    }
    let mut file = File::open(path).ok()?;
    let mpeg = MpegFile::read_from(&mut file, ParseOptions::new()).ok()?;
    let frame = mpeg
        .id3v2()?
        .into_iter()
        .filter_map(|frame| {
            let Frame::Binary(binary) = frame else {
                return None;
            };
            if frame.id().as_str() != "SYLT" {
                return None;
            }
            SynchronizedTextFrame::parse(&binary.data, frame.flags()).ok()
        })
        .filter(|synced| synced.timestamp_format == TimestampFormat::MS)
        // Many taggers never set the type, so untyped frames stay; chords and events do not.
        .filter(|synced| {
            matches!(
                synced.content_type,
                SyncTextContentType::Lyrics | SyncTextContentType::Other
            )
        })
        .min_by_key(|synced| synced.content_type != SyncTextContentType::Lyrics)?;
    let mut lines = sylt_lines(frame.content);
    lrc::normalize(&mut lines);
    (!lines.is_empty()).then_some(lines)
}

/// A newline at either end of an entry breaks the line, which is how a frame timed by syllable
/// marks its lines; a frame without any is timed by line.
fn sylt_lines(content: Vec<(u32, String)>) -> Vec<LyricsLine> {
    let worded = content
        .iter()
        .any(|(_, text)| text.starts_with(BREAKS) || text.ends_with(BREAKS));
    if !worded {
        return content
            .into_iter()
            .map(|(start, text)| line(millis(start), text.trim().to_owned(), None))
            .filter(|line| !line.text.is_empty())
            .collect();
    }

    let mut groups: Vec<Vec<LyricsWord>> = Vec::new();
    let mut open = false;
    for (start, text) in content {
        let start = millis(start);
        if let Some(previous) = groups.iter_mut().rev().find_map(|words| words.last_mut()) {
            previous.end = start.max(previous.start);
        }
        if !open || text.starts_with(BREAKS) {
            groups.push(Vec::new());
        }
        open = !text.ends_with(BREAKS);
        let text = text.trim_matches(BREAKS);
        if text.is_empty() {
            continue;
        }
        groups
            .last_mut()
            .expect("a line is always open")
            .push(LyricsWord {
                start,
                end: start,
                text: text.to_owned(),
            });
    }
    groups
        .into_iter()
        .filter_map(|words| {
            let start = words.first()?.start;
            let text: String = words.iter().map(|word| word.text.as_str()).collect();
            Some(line(start, text.trim().to_owned(), Some(words)))
        })
        .collect()
}

fn line(start: Duration, text: String, words: Option<Vec<LyricsWord>>) -> LyricsLine {
    LyricsLine {
        start,
        end: None,
        text,
        romanized: None,
        words,
        secondary: Vec::new(),
        voice: Voice::Lead,
    }
}

fn millis(value: u32) -> Duration {
    Duration::from_millis(u64::from(value))
}

pub fn write(path: &Path, tags: &TrackTags) -> Result<()> {
    update(path, |tag| {
        set(tag, ItemKey::TrackTitle, &tags.title);
        set_artists(tag, &tags.artists);
        set(tag, ItemKey::AlbumTitle, &tags.album);
        if held(tag, ItemKey::AlbumArtist) != tags.album_artist.trim() {
            tag.remove_key(ItemKey::AlbumArtists);
            set(tag, ItemKey::AlbumArtist, &tags.album_artist);
        }
        set(tag, ItemKey::Genre, &tags.genre);
        set(tag, ItemKey::Composer, &tags.composer);
        set(tag, ItemKey::Publisher, &tags.publisher);
        set(tag, ItemKey::Isrc, &tags.isrc);
        set(tag, ItemKey::Comment, &tags.comment);
        set(tag, ItemKey::Lyrics, &tags.lyrics);

        counted(tag, ItemKey::TrackNumber, &tags.track_number);
        counted(tag, ItemKey::TrackTotal, &tags.track_total);
        counted(tag, ItemKey::DiscNumber, &tags.disc_number);
        counted(tag, ItemKey::DiscTotal, &tags.disc_total);
        set_year(tag, &tags.year);
    })
}

pub fn write_year(path: &Path, value: &str) -> Result<()> {
    update(path, |tag| set_year(tag, value))
}

/// Applies `change` to every tag the file holds, adding its primary tag when it has none, so a
/// player that reads a secondary tag such as ID3v1 sees the edit too. A leading ID3v2.3 tag is
/// saved as ID3v2.3 again, since players that cannot read ID3v2.4 fall back to the stale ID3v1.
fn update(path: &Path, change: impl Fn(&mut Tag)) -> Result<()> {
    let mut tagged = Probe::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .read()
        .with_context(|| format!("cannot read the tags in {}", path.display()))?;
    if tagged.primary_tag().is_none() {
        let kind = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(kind));
    }
    let kinds: Vec<_> = tagged.tags().iter().map(Tag::tag_type).collect();
    if kinds.is_empty() {
        anyhow::bail!("{} cannot hold tags", path.display());
    }
    for kind in kinds {
        if let Some(tag) = tagged.tag_mut(kind) {
            change(tag);
        }
    }

    let stacked = matches!(tagged.file_type(), FileType::Mpeg | FileType::Aac);
    let options = WriteOptions::default().use_id3v23(stacked && leads_with_id3v23(path));
    tagged
        .save_to_path(path, options)
        .with_context(|| format!("cannot save the tags in {}", path.display()))?;
    if stacked {
        drop_stacked_id3v2(path)?;
    }
    Ok(())
}

fn leads_with_id3v23(path: &Path) -> bool {
    let mut header = [0; 4];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok_and(|()| header == *b"ID3\x03")
}

/// Removes every ID3v2 tag that directly follows the first one. Lofty reads stacked tags as one,
/// letting the later ones win, but saves only the first, so a stale tag behind it would undo the
/// edit on the next read. The saved tag already holds every frame merged from the others.
fn drop_stacked_id3v2(path: &Path) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    let Some(first) = id3v2_len(&bytes) else {
        return Ok(());
    };
    let mut end = first;
    while let Some(len) = bytes.get(end..).and_then(id3v2_len) {
        end += len;
    }
    if end == first {
        return Ok(());
    }
    let mut kept = Vec::with_capacity(bytes.len() - (end - first));
    kept.extend_from_slice(&bytes[..first]);
    kept.extend_from_slice(&bytes[end..]);
    std::fs::write(path, kept)
        .with_context(|| format!("cannot drop the stacked tags in {}", path.display()))
}

/// Writes the year, leaving the date alone while its year reads the same, so a month and day
/// survive, and so does a date the editor showed no year for.
fn set_year(tag: &mut Tag, value: &str) {
    let wanted = year(value);
    if wanted == tag.date().map(|date| date.year) {
        return;
    }
    match wanted {
        Some(year) => tag.set_date(Timestamp {
            year,
            month: None,
            day: None,
            hour: None,
            minute: None,
            second: None,
        }),
        None => tag.remove_date(),
    }
}

fn text(value: Option<std::borrow::Cow<'_, str>>) -> String {
    value
        .map(|value| value.trim().to_owned())
        .unwrap_or_default()
}

fn held(tag: &Tag, key: ItemKey) -> String {
    tag.get_string(key).map(str::to_owned).unwrap_or_default()
}

/// A ReplayGain value such as `-7.23 dB`, read as its number of decibels.
fn decibels(value: &str) -> Option<f32> {
    value
        .trim()
        .trim_end_matches(|c: char| c.is_ascii_alphabetic())
        .trim()
        .parse()
        .ok()
}

fn number(value: Option<u32>) -> String {
    value
        .filter(|value| *value > 0)
        .map(|value| value.to_string())
        .unwrap_or_default()
}

/// The full length of the ID3v2 tag at the start of `bytes`, header and footer included.
fn id3v2_len(bytes: &[u8]) -> Option<usize> {
    let header = bytes.get(..10)?;
    if &header[..3] != b"ID3" || header[6..].iter().any(|byte| *byte >= 0x80) {
        return None;
    }
    let size = header[6..]
        .iter()
        .fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
    let footer = match header[5] & 0x10 != 0 {
        true => 10,
        false => 0,
    };
    Some(10 + size + footer).filter(|len| *len <= bytes.len())
}

fn year(value: &str) -> Option<u16> {
    value.trim().parse().ok().filter(|year| *year > 0)
}

/// Writes one field, leaving it alone when it already reads as `value`, so a key the file holds
/// several values under is not cut down to the first one the editor showed.
fn set(tag: &mut Tag, key: ItemKey, value: &str) {
    let value = value.trim();
    if tag.get_string(key).map(str::trim) == Some(value) {
        return;
    }
    match value.is_empty() {
        true => {
            tag.remove_key(key);
        }
        false => {
            tag.insert_text(key, value.to_owned());
        }
    }
}

/// Writes one value per artist under both artist keys. An unchanged list leaves the original
/// credit untouched when editing another field.
fn set_artists(tag: &mut Tag, artists: &[String]) {
    let artists: Vec<String> = artists
        .iter()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    if wire::artists(tag) == artists {
        return;
    }
    tag.remove_key(ItemKey::TrackArtists);
    tag.remove_key(ItemKey::TrackArtist);
    for name in artists {
        tag.push(TagItem::new(
            ItemKey::TrackArtist,
            ItemValue::Text(name.clone()),
        ));
        tag.push(TagItem::new(ItemKey::TrackArtists, ItemValue::Text(name)));
    }
}

fn counted(tag: &mut Tag, key: ItemKey, value: &str) {
    match value.trim().parse::<u32>().ok().filter(|value| *value > 0) {
        Some(value) => set(tag, key, &value.to_string()),
        None => set(tag, key, ""),
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    fn entries(content: &[(u32, &str)]) -> Vec<(u32, String)> {
        content
            .iter()
            .map(|(start, text)| (*start, (*text).to_owned()))
            .collect()
    }

    #[test]
    fn a_frame_without_newlines_is_timed_by_line() {
        let lines = sylt_lines(entries(&[(1000, "one"), (3000, " two ")]));

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "two");
        assert!(lines.iter().all(|line| line.words.is_none()));
    }

    #[test]
    fn a_leading_newline_opens_a_line_of_syllables() {
        let lines = sylt_lines(entries(&[
            (1000, "\nBeau"),
            (1200, "ti"),
            (1400, "ful "),
            (1600, "day"),
            (3000, "\nNext"),
        ]));

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "Beautiful day");
        assert_eq!(lines[1].start, Duration::from_millis(3000));
        let words = lines[0]
            .words
            .as_ref()
            .expect("the line is timed by syllable");
        assert_eq!(words.len(), 4);
        assert_eq!(words[3].end, Duration::from_millis(3000));
    }

    #[test]
    fn a_trailing_newline_closes_the_line() {
        let lines = sylt_lines(entries(&[(0, "a "), (500, "b\n"), (1000, "c")]));

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "a b");
        assert_eq!(lines[1].start, Duration::from_millis(1000));
    }

    /// Writes a FLAC file holding no audio and the Vorbis comments given, each as `KEY=value`.
    pub(in crate::local) fn flac(path: &Path, comments: &[&str]) {
        let mut info = vec![0x10, 0x00, 0x10, 0x00, 0, 0, 0, 0, 0, 0];
        let packed: u64 = (44_100 << 44) | (1 << 41) | (15 << 36);
        info.extend_from_slice(&packed.to_be_bytes());
        info.extend_from_slice(&[0; 16]);

        let vendor = b"sonora";
        let mut block = Vec::new();
        block.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        block.extend_from_slice(vendor);
        block.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for comment in comments {
            block.extend_from_slice(&(comment.len() as u32).to_le_bytes());
            block.extend_from_slice(comment.as_bytes());
        }

        let mut bytes = b"fLaC".to_vec();
        bytes.push(0x00);
        bytes.extend_from_slice(&(info.len() as u32).to_be_bytes()[1..]);
        bytes.extend_from_slice(&info);
        bytes.push(0x84);
        bytes.extend_from_slice(&(block.len() as u32).to_be_bytes()[1..]);
        bytes.extend_from_slice(&block);
        std::fs::write(path, bytes).unwrap();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn stored(path: &Path) -> Tag {
        Probe::open(path)
            .unwrap()
            .read()
            .unwrap()
            .primary_tag()
            .cloned()
            .expect("a tag")
    }

    #[test]
    fn saving_a_new_title_keeps_every_genre() {
        let dir = scratch("sonora-tags-test-genres");
        let path = dir.join("song.flac");
        flac(&path, &["TITLE=Song", "GENRE=Rock", "GENRE=Pop"]);

        let mut tags = read(&path).unwrap();
        tags.title = "Renamed".to_owned();
        write(&path, &tags).unwrap();

        let tag = stored(&path);
        assert_eq!(tag.title().as_deref(), Some("Renamed"));
        assert_eq!(
            tag.get_strings(ItemKey::Genre).collect::<Vec<_>>(),
            ["Rock", "Pop"]
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_a_new_title_keeps_the_full_date() {
        let dir = scratch("sonora-tags-test-date");
        let path = dir.join("song.flac");
        flac(&path, &["TITLE=Song", "DATE=2004-05-12"]);

        let mut tags = read(&path).unwrap();
        tags.title = "Renamed".to_owned();
        write(&path, &tags).unwrap();

        assert_eq!(
            stored(&path).get_string(ItemKey::RecordingDate),
            Some("2004-05-12")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn saving_a_new_year_replaces_the_date() {
        let dir = scratch("sonora-tags-test-year");
        let path = dir.join("song.flac");
        flac(&path, &["TITLE=Song", "DATE=2004-05-12"]);

        let mut tags = read(&path).unwrap();
        tags.year = "2010".to_owned();
        write(&path, &tags).unwrap();

        assert_eq!(
            stored(&path).get_string(ItemKey::RecordingDate),
            Some("2010")
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
