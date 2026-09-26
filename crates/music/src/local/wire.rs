use std::borrow::Cow;
use std::collections::HashSet;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::Accessor;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;

use super::id3;
use crate::{
    Album, ArtistRef, LOCAL_ALBUM_PREFIX, LOCAL_ARTIST_PREFIX, LOCAL_TRACK_PREFIX, ReleaseType,
    Track,
};

const COVER_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "cover.webp",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "folder.webp",
];

const ARTIST_NAMES: &[&str] = &[
    "artist.jpg",
    "artist.jpeg",
    "artist.png",
    "artist.webp",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "folder.webp",
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "cover.webp",
];

/// What separates one artist from the next in a track's credit, matched without regard to case.
/// An ampersand is left alone because it is as often part of one name as a join of two.
const CREDIT_MARKS: &[&str] = &[
    " featuring ",
    " feat. ",
    " feat ",
    " ft. ",
    " ft ",
    ",",
    ";",
];

/// The credit of an album whose tracks name no album artist and share no artist either.
const VARIOUS_ARTISTS: &str = "Various Artists";

const PLAYABLE_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "wav", "opus", "webm", "mka", "wv", "ape",
];

enum Field {
    TrackArtist,
    AlbumArtist,
}

fn is_playable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PLAYABLE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

pub fn track_id(path: &Path) -> String {
    format!("{LOCAL_TRACK_PREFIX}{}", path.display())
}

pub fn path_from_track_id(id: &str) -> Option<&Path> {
    id.strip_prefix(LOCAL_TRACK_PREFIX).map(Path::new)
}

/// Byte offset where a leading ID3v2 tag ends, or 0 if `path` doesn't start with one.
/// Playback skips straight past it: ID3v2 carries no audio data, and some taggers write a
/// frame (`WXXX` from gamerip tools is the one seen in the wild) that both `symphonia` and
/// `lofty` refuse to parse, which otherwise makes the whole file fail to open for decoding.
pub fn id3v2_end(path: &Path) -> u64 {
    use std::io::Read;

    let Ok(mut file) = std::fs::File::open(path) else {
        return 0;
    };
    let mut header = [0u8; 10];
    if file.read_exact(&mut header).is_err() || &header[0..3] != b"ID3" {
        return 0;
    }

    let syncsafe = |byte: u8| u32::from(byte & 0x7f);
    let size = (syncsafe(header[6]) << 21)
        | (syncsafe(header[7]) << 14)
        | (syncsafe(header[8]) << 7)
        | syncsafe(header[9]);
    let footer = header[5] & 0x10 != 0;

    let skip = u64::from(size) + 10 + if footer { 10 } else { 0 };
    log::warn!(
        "[local_music] file {} id3v2 bytes skiped: {}",
        path.display(),
        skip
    );

    skip
}

pub fn normalize(value: &str) -> String {
    value.trim().to_lowercase()
}

/// True when the first MPEG frame past `skip` carries a Xing/Info VBR header whose declared
/// frame count is `0` instead of omitted
pub fn has_lying_xing_frame_count(path: &Path, skip: u64) -> bool {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    if skip > 0 && file.seek(SeekFrom::Start(skip)).is_err() {
        return false;
    }

    let mut head = [0u8; 64];
    let Ok(read) = file.read(&mut head) else {
        return false;
    };
    let head = &head[..read];

    [b"Xing".as_slice(), b"Info".as_slice()]
        .into_iter()
        .filter_map(|marker| head.windows(4).position(|window| window == marker))
        .any(|pos| {
            let flags = head.get(pos + 4..pos + 8);
            let has_frame_count = flags.is_some_and(|flags| flags[3] & 0x1 != 0);
            let count = has_frame_count
                .then(|| head.get(pos + 8..pos + 12))
                .flatten();
            count.is_some_and(|bytes| bytes == [0, 0, 0, 0])
        })
}

pub fn album_id(artist: &str, name: &str) -> String {
    let mut hasher = DefaultHasher::new();
    normalize(artist).hash(&mut hasher);
    normalize(name).hash(&mut hasher);
    format!("{LOCAL_ALBUM_PREFIX}{:016x}", hasher.finish())
}

pub fn artist_id(name: &str) -> String {
    let mut hasher = DefaultHasher::new();
    normalize(name).hash(&mut hasher);
    format!("{LOCAL_ARTIST_PREFIX}{:016x}", hasher.finish())
}

pub fn artist_ref(name: &str) -> ArtistRef {
    ArtistRef {
        name: name.to_owned(),
        id: Some(artist_id(name)),
    }
}

fn artist_refs(name: &str, names: Vec<String>) -> Vec<ArtistRef> {
    let names = match names.is_empty() {
        true => vec![name.to_owned()],
        false => names,
    };
    names.iter().map(|name| artist_ref(name)).collect()
}

/// Returns the artists from tags. It reads `TrackArtists`/`AlbumArtists` first, then falls back to
/// `TrackArtist`/`AlbumArtist`. Empty means there is no artist information or it can't be read.
fn one_or_many(tag: Option<&Tag>, field: Field) -> Vec<String> {
    let Some(tag) = tag else { return Vec::new() };
    let (plural, single) = match field {
        Field::TrackArtist => (ItemKey::TrackArtists, ItemKey::TrackArtist),
        Field::AlbumArtist => (ItemKey::AlbumArtists, ItemKey::AlbumArtist),
    };
    let names = clean_multiple(tag.get_strings(plural));
    match names.is_empty() {
        true => clean_multiple(tag.get_strings(single)),
        false => names,
    }
}

fn clean(value: Option<Cow<'_, str>>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn clean_multiple<'a>(values: impl Iterator<Item = &'a str>) -> Vec<String> {
    values
        .filter_map(|value| clean(Some(Cow::Borrowed(value))))
        .collect()
}

fn infer_from_stem(stem: &str) -> (Option<String>, Option<String>) {
    let split = stem
        .rsplit_once(" - ")
        .or_else(|| stem.rsplit_once(" \u{2013} "))
        .or_else(|| stem.rsplit_once(" \u{2014} "))
        .or_else(|| stem.rsplit_once(" \u{ff0d} "));

    if let Some((left, right)) = split {
        let left = left.trim();
        let right = right.trim();
        if !left.is_empty() && !right.is_empty() {
            if left.chars().all(|c| c.is_ascii_digit()) {
                return (Some(right.to_owned()), None);
            }
            if right.chars().all(|c| c.is_ascii_digit()) {
                return (Some(left.to_owned()), None);
            }
            let title = numbered(left).unwrap_or_else(|| left.to_owned());
            return (Some(title), Some(right.to_owned()));
        }
    }
    numbered(stem)
        .map(|title| (Some(title), None))
        .unwrap_or((None, None))
}

fn numbered(stem: &str) -> Option<String> {
    let stem = stem.trim();
    for sep in [".", " - ", " \u{2013} ", " \u{2014} ", " \u{ff0d} "] {
        if let Some((digits, rest)) = stem.split_once(sep)
            && !digits.is_empty()
            && digits.len() <= 3
            && digits.chars().all(|c| c.is_ascii_digit())
        {
            let rest = rest.trim();
            if !rest.is_empty() {
                return Some(rest.to_owned());
            }
        }
    }
    None
}

struct FallbackProbe {
    duration: std::time::Duration,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: u32,
    disc_number: u32,
    year: Option<i32>,
    cover_data: Option<(Vec<u8>, String)>,
}

fn probe_symphonia(path: &Path) -> Option<FallbackProbe> {
    if let Some(probe) = probe_symphonia_at(path, 0) {
        return Some(probe);
    }

    // skips till end of id3v2 tag
    let skip = id3v2_end(path);

    if skip == 0 {
        // tag not found, file is broken
        return None;
    }
    probe_symphonia_at(path, skip)
}

fn probe_symphonia_at(path: &Path, skip: u64) -> Option<FallbackProbe> {
    let mut file = std::fs::File::open(path).ok()?;
    if skip > 0 {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::Start(skip)).ok()?;
    }
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }
    let fmt_opts = FormatOptions::default();
    let meta_opts = MetadataOptions::default();
    let mut probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .ok()?;

    let mut duration = std::time::Duration::ZERO;
    for track in probed.format.tracks() {
        if track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_NULL {
            continue;
        }
        if let (Some(n_frames), Some(tb)) =
            (track.codec_params.n_frames, track.codec_params.time_base)
        {
            let time = tb.calc_time(n_frames);
            duration = std::time::Duration::from_secs(time.seconds)
                .saturating_add(std::time::Duration::from_secs_f64(time.frac));
            if !duration.is_zero() {
                break;
            }
        } else if let (Some(n_frames), Some(rate)) =
            (track.codec_params.n_frames, track.codec_params.sample_rate)
            && rate > 0
        {
            duration = std::time::Duration::from_secs_f64(n_frames as f64 / rate as f64);
            if !duration.is_zero() {
                break;
            }
        }
    }

    let mut title = None;
    let mut artist = None;
    let mut album = None;
    let mut album_artist = None;
    let mut track_number = 0;
    let mut disc_number = 0;
    let mut year = None;
    let mut cover_data = None;

    let mut collect_metadata = |rev: &symphonia::core::meta::MetadataRevision| {
        for tag in rev.tags() {
            let clean_val = |v: &symphonia::core::meta::Value| {
                let s = v.to_string();
                let trimmed = s.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            };
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if title.is_none() => {
                    title = clean_val(&tag.value);
                }
                Some(StandardTagKey::Artist) if artist.is_none() => {
                    artist = clean_val(&tag.value);
                }
                Some(StandardTagKey::Album) if album.is_none() => {
                    album = clean_val(&tag.value);
                }
                Some(StandardTagKey::AlbumArtist) if album_artist.is_none() => {
                    album_artist = clean_val(&tag.value);
                }
                Some(StandardTagKey::TrackNumber) if track_number == 0 => {
                    if let Ok(n) = tag.value.to_string().parse() {
                        track_number = n;
                    }
                }
                Some(StandardTagKey::DiscNumber) if disc_number == 0 => {
                    if let Ok(n) = tag.value.to_string().parse() {
                        disc_number = n;
                    }
                }
                Some(StandardTagKey::Date) if year.is_none() => {
                    let s = tag.value.to_string();
                    if s.len() >= 4 {
                        year = s
                            .get(..4)
                            .and_then(|y| y.parse::<i32>().ok())
                            .filter(|y| *y > 0);
                    }
                }
                _ => {}
            }
        }
        if cover_data.is_none()
            && let Some(visual) = rev.visuals().first()
        {
            cover_data = Some((visual.data.to_vec(), visual.media_type.clone()));
        }
    };

    if let Some(meta) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        collect_metadata(meta);
    }
    if let Some(meta) = probed.format.metadata().current() {
        collect_metadata(meta);
    }

    Some(FallbackProbe {
        duration,
        title,
        artist,
        album,
        album_artist,
        track_number,
        disc_number,
        year,
        cover_data,
    })
}

/// When a file last changed, in whole seconds since the epoch. `None` when the
/// filesystem does not keep the time or it sits outside what an `i64` of seconds holds.
pub fn modified_at(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    match modified.duration_since(std::time::UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).ok(),
        Err(before) => i64::try_from(before.duration().as_secs())
            .ok()
            .map(|seconds| -seconds),
    }
}

/// Reads one file into a track, the album artist its tags name, and the year its tag claims.
/// A file naming an album artist joins that artist's album of the same name. One that names none
/// joins the album of the same name in its own folder, so featured artists never split an album.
/// The year comes out of the same read because it is what an album is dated by, and opening every
/// album's first track again costs a round trip each on a share.
pub fn track_from_file(
    path: &Path,
    artist_hint: Option<&str>,
    album_hint: Option<&str>,
    cache_dir: &Path,
) -> Option<(Track, Vec<ArtistRef>, Option<i32>)> {
    let tagged = Probe::open(path).ok().and_then(|file| file.read().ok());
    let tag = tagged
        .as_ref()
        .and_then(|file| file.primary_tag().or_else(|| file.first_tag()));

    let mut duration = tagged
        .as_ref()
        .map(|file| file.properties().duration())
        .unwrap_or_default();

    let fallback = if duration.is_zero() || tag.is_none() {
        probe_symphonia(path)
    } else {
        None
    };

    if duration.is_zero()
        && let Some(ref fb) = fallback
    {
        duration = fb.duration;
    }

    // Last resort: `lofty` failed to open the tag at all, which also means `probe_symphonia`'s
    // own text-frame reading gave up on it (its id3v2-skip retry above only ever recovers
    // duration, never metadata, since that retry starts past the whole tag on purpose). This
    // hand-rolled reader skips a frame it doesn't understand by its declared size alone, so it
    // can still recover the other, perfectly well-formed frames around it.
    let lenient = tag.is_none().then(|| id3::read(path)).flatten();

    let (inferred_title, inferred_artist) = infer_from_stem(&file_stem(path));

    let name = clean(tag.and_then(Accessor::title))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.title.clone()))
        .or_else(|| lenient.as_ref().and_then(|l| l.title.clone()))
        .or(inferred_title)
        .unwrap_or_else(|| file_stem(path));

    let artist_names = one_or_many(tag, Field::TrackArtist);
    let artist = (!artist_names.is_empty())
        .then(|| artist_names.join(", "))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.artist.clone()))
        .or_else(|| lenient.as_ref().and_then(|l| l.artist.clone()))
        .or_else(|| artist_hint.map(str::to_owned))
        .or(inferred_artist)
        .unwrap_or_else(|| "Unknown Artist".to_owned());
    let track_artist_refs = artist_refs(&artist, artist_names);

    let album_artist_names = one_or_many(tag, Field::AlbumArtist);
    let album_artist = (!album_artist_names.is_empty())
        .then(|| album_artist_names.join(", "))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.album_artist.clone()))
        .or_else(|| lenient.as_ref().and_then(|l| l.album_artist.clone()));
    let album_artist_refs = album_artist
        .as_deref()
        .map_or_else(Vec::new, |name| artist_refs(name, album_artist_names));

    let album_name = clean(tag.and_then(Accessor::album))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.album.clone()))
        .or_else(|| lenient.as_ref().and_then(|l| l.album.clone()))
        .or_else(|| album_hint.map(str::to_owned))
        .unwrap_or_default();

    let track_number = tag
        .and_then(Accessor::track)
        .or_else(|| {
            fallback
                .as_ref()
                .map(|fb| fb.track_number)
                .filter(|n| *n > 0)
        })
        .or_else(|| lenient.as_ref().and_then(|l| l.track_number))
        .unwrap_or(0);

    let disc_number = tag
        .and_then(Accessor::disk)
        .or_else(|| {
            fallback
                .as_ref()
                .map(|fb| fb.disc_number)
                .filter(|n| *n > 0)
        })
        .or_else(|| lenient.as_ref().and_then(|l| l.disc_number))
        .unwrap_or(0);

    let mut cover = extract_cover(tag, path, cache_dir);
    if cover.is_none()
        && let Some(ref fb) = fallback
        && let Some((ref data, ref mime)) = fb.cover_data
    {
        cover = cache_image_data(data, mime, cache_dir);
    }
    if cover.is_none()
        && let Some(ref l) = lenient
        && let Some((ref data, ref mime)) = l.cover
    {
        cover = cache_image_data(data, mime, cache_dir);
    }

    let album_id = (!album_name.is_empty()).then(|| {
        let owner = album_artist
            .clone()
            .unwrap_or_else(|| album_folder(path).to_string_lossy().into_owned());
        album_id(&owner, &album_name)
    });
    let year = tag
        .and_then(|tag| tag.date())
        .map(|date| date.year as i32)
        .filter(|year| *year > 0)
        .or_else(|| fallback.as_ref().and_then(|fb| fb.year))
        .or_else(|| lenient.as_ref().and_then(|l| l.year));

    Some((
        Track {
            id: Some(track_id(path)),
            name,
            playable: is_playable(path),
            artists: artist,
            artist_refs: track_artist_refs,
            album: album_name,
            album_id,
            cover,
            duration,
            added_at: modified_at(path),
            added_by: None,
            playcount: None,
            popularity: 0,
            explicit: false,
            track_number,
            disc_number,
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        },
        album_artist_refs,
        year,
    ))
}

/// Builds the album `tracks` were grouped under. `id` is the one the tracks carry, since it is
/// keyed by folder rather than by `artist` when the files name no album artist.
pub fn album_from_tracks(
    id: &str,
    name: &str,
    artist_refs: &[ArtistRef],
    tracks: &[Track],
    year: i32,
) -> Album {
    let artists = artist_refs
        .iter()
        .map(|artist| artist.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let cover = tracks.iter().find_map(|track| track.cover.clone());
    Album {
        id: id.to_owned(),
        name: name.to_owned(),
        artists,
        artist_refs: artist_refs.to_vec(),
        cover: cover.clone(),
        cover_large: cover,
        release_type: ReleaseType::Album,
        year,
        track_count: tracks.len() as u32,
        release_date: String::new(),
        label: String::new(),
        copyrights: Vec::new(),
        added_at: tracks.iter().filter_map(|track| track.added_at).max(),
    }
}

/// The artists every track credits, in the order the first track lists them, for an album whose
/// files name no album artist. Credits split on commas, semicolons and featuring marks, and
/// tracks that share nobody are credited to various artists.
pub fn shared_artists(tracks: &[Track]) -> String {
    let Some((first, rest)) = tracks.split_first() else {
        return VARIOUS_ARTISTS.to_owned();
    };
    let others: Vec<HashSet<String>> = rest
        .iter()
        .map(|track| {
            credited(&track.artists)
                .iter()
                .map(|name| normalize(name))
                .collect()
        })
        .collect();
    let shared: Vec<String> = credited(&first.artists)
        .into_iter()
        .filter(|name| {
            others
                .iter()
                .all(|credits| credits.contains(&normalize(name)))
        })
        .collect();
    match shared.is_empty() {
        true => VARIOUS_ARTISTS.to_owned(),
        false => shared.join(", "),
    }
}

/// Splits one track's artist credit into the names it lists.
fn credited(artists: &str) -> Vec<String> {
    let mut text = format!(" {} ", artists.replace(['(', ')', '[', ']'], " "));
    for mark in CREDIT_MARKS {
        while let Some(at) = text.to_ascii_lowercase().find(mark) {
            text.replace_range(at..at + mark.len(), "\n");
        }
    }
    text.split('\n')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The folder an untagged track's album is keyed by. A disc folder such as `CD1` or `Disc 2`
/// stands for the album folder above it, so a multi-disc rip still makes one album.
fn album_folder(path: &Path) -> &Path {
    let Some(dir) = path.parent() else {
        return path;
    };
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let disc = ["cd", "disc", "disk"].iter().any(|prefix| {
        name.strip_prefix(prefix).is_some_and(|rest| {
            let rest = rest.trim_start_matches([' ', '_', '-', '.']);
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
        })
    });
    match disc {
        true => dir.parent().unwrap_or(dir),
        false => dir,
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

pub fn folder_cover(dir: &Path) -> Option<String> {
    beside(dir, COVER_NAMES)
}

pub fn artist_cover(dir: &Path) -> Option<String> {
    beside(dir, ARTIST_NAMES)
}

fn beside(dir: &Path, names: &[&str]) -> Option<String> {
    names
        .iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
        .map(|candidate| format!("file://{}", candidate.display()))
}

fn extract_cover(tag: Option<&Tag>, path: &Path, cache_dir: &Path) -> Option<String> {
    let picture = tag.and_then(|tag| {
        tag.pictures()
            .iter()
            .find(|picture| picture.pic_type() == PictureType::CoverFront)
            .or_else(|| tag.pictures().first())
    });

    if let Some(picture) = picture
        && let Some(cached) = cache_picture(picture, cache_dir)
    {
        return Some(cached);
    }

    path.parent().and_then(folder_cover)
}

fn cache_picture(picture: &Picture, cache_dir: &Path) -> Option<String> {
    let extension = match picture.mime_type() {
        Some(MimeType::Png) => "png",
        Some(MimeType::Gif) => "gif",
        Some(MimeType::Bmp) => "bmp",
        Some(MimeType::Tiff) => "tiff",
        _ => "jpg",
    };

    cache_image_data(picture.data(), extension, cache_dir)
}

fn cache_image_data(data: &[u8], media_type_or_ext: &str, cache_dir: &Path) -> Option<String> {
    if data.is_empty() {
        return None;
    }

    let extension = if media_type_or_ext.contains("png") {
        "png"
    } else if media_type_or_ext.contains("gif") {
        "gif"
    } else if media_type_or_ext.contains("bmp") {
        "bmp"
    } else if media_type_or_ext.contains("tiff") {
        "tiff"
    } else {
        "jpg"
    };

    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    let hash = hasher.finish();

    let dir = cache_dir.join("local-covers");
    std::fs::create_dir_all(&dir).ok()?;
    let dest = dir.join(format!("{hash:016x}.{extension}"));
    if !dest.exists() {
        std::fs::write(&dest, data).ok()?;
    }
    Some(format!("file://{}", dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::tag::{ItemValue, TagItem, TagType};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn infer_stem_with_title_and_artist() {
        let (title, artist) = infer_from_stem("Chann Vi Gawah - Madhav Mahajan");
        assert_eq!(title.as_deref(), Some("Chann Vi Gawah"));
        assert_eq!(artist.as_deref(), Some("Madhav Mahajan"));
    }

    #[test]
    fn infer_stem_with_leading_track_number() {
        let (title, artist) = infer_from_stem("01 - Bohemian Rhapsody");
        assert_eq!(title.as_deref(), Some("Bohemian Rhapsody"));
        assert_eq!(artist, None);
    }

    #[test]
    fn infer_stem_with_trailing_track_number() {
        let (title, artist) = infer_from_stem("Bohemian Rhapsody - 01");
        assert_eq!(title.as_deref(), Some("Bohemian Rhapsody"));
        assert_eq!(artist, None);
    }

    #[test]
    fn infer_stem_multiple_hyphens() {
        let (title, artist) =
            infer_from_stem("Aasa Kooda - From Think Indie - Sai Abhyankkar, Sai Smriti");
        assert_eq!(title.as_deref(), Some("Aasa Kooda - From Think Indie"));
        assert_eq!(artist.as_deref(), Some("Sai Abhyankkar, Sai Smriti"));
    }

    #[test]
    fn infer_stem_no_hyphen() {
        let (title, artist) = infer_from_stem("SingleTitle");
        assert_eq!(title, None);
        assert_eq!(artist, None);
    }

    fn tagged(items: &[(ItemKey, &str)]) -> Tag {
        let mut tag = Tag::new(TagType::VorbisComments);
        for (key, value) in items {
            let item = TagItem::new(*key, ItemValue::Text((*value).to_owned()));
            assert!(
                tag.push(item),
                "{key:?} field is not supported by this format"
            );
        }
        tag
    }

    #[test]
    fn a_repeated_field_is_several_artists() {
        let tag = tagged(&[
            (ItemKey::TrackArtist, "First"),
            (ItemKey::TrackArtist, " Second"),
            (ItemKey::TrackArtist, "Third  "),
        ]);

        let names = one_or_many(Some(&tag), Field::TrackArtist);
        let refs = artist_refs(&names.join(", "), names);

        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].name, "First");
        assert_eq!(refs[1].name, "Second");
        assert_eq!(refs[2].name, "Third");
    }

    #[test]
    fn the_credited_list_wins_over_the_single_field() {
        let tag = tagged(&[
            (ItemKey::TrackArtist, "First & Second"),
            (ItemKey::TrackArtists, "First"),
            (ItemKey::TrackArtists, "Second"),
        ]);

        let names = one_or_many(Some(&tag), Field::TrackArtist);
        let refs = artist_refs(&names.join(", "), names);

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "First");
        assert_eq!(refs[1].name, "Second");
    }

    #[test]
    fn an_artist_id_ignores_capitalization() {
        assert_eq!(artist_id("Artist"), artist_id("artist"));
    }

    #[test]
    fn cache_image_data_deduplication_and_empty() {
        let temp = std::env::temp_dir().join(format!(
            "sonora-img-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        assert!(cache_image_data(&[], "jpg", &temp).is_none());

        let pic1 = cache_image_data(&[1, 2, 3], "image/jpeg", &temp);
        let pic2 = cache_image_data(&[1, 2, 3], "jpg", &temp);
        let pic3 = cache_image_data(&[4, 5, 6], "png", &temp);

        assert_eq!(pic1, pic2);
        assert_ne!(pic1, pic3);

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn parses_webm_opus_track() {
        let path = Path::new(r"D:\Songs\Chann Vi Gawah - Madhav Mahajan.opus");
        if !path.exists() {
            return;
        }
        let temp = std::env::temp_dir().join(format!(
            "sonora-webm-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (track, ..) = track_from_file(path, None, None, &temp).expect("track parsed");
        assert!(track.playable);
        assert_eq!(track.name, "Chann Vi Gawah");
        assert_eq!(track.artists, "Madhav Mahajan");
        assert_eq!(track.duration.as_secs(), 252);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn decodes_webm_opus_playback() {
        use rodio::Source;
        let path = Path::new(r"D:\Songs\Chann Vi Gawah - Madhav Mahajan.opus");
        if !path.exists() {
            return;
        }
        let file = std::fs::File::open(path).unwrap();
        let reader = std::io::BufReader::new(file);
        let decoder = rodio::Decoder::builder().with_data(reader).build().unwrap();
        let duration = decoder.total_duration().unwrap();
        assert_eq!(duration.as_secs(), 252);
    }

    #[test]
    fn a_track_is_dated_by_the_file_it_came_from() {
        let dir = scratch("sonora-wire-test-dated");
        let path = dir.join("song.mp3");
        std::fs::write(&path, []).unwrap();

        let stamped = modified_at(&path).expect("a file just written has a modified time");
        let (track, ..) = track_from_file(&path, None, None, &dir).expect("a track");

        assert_eq!(track.added_at, Some(stamped));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_album_is_dated_by_its_newest_track() {
        let dir = scratch("sonora-wire-test-album-dated");
        let path = dir.join("song.mp3");
        std::fs::write(&path, []).unwrap();

        let (mut older, ..) = track_from_file(&path, None, None, &dir).expect("a track");
        let mut newer = older.clone();
        older.added_at = Some(1_000);
        newer.added_at = Some(2_000);

        let album = album_from_tracks(
            "id",
            "Album",
            &artist_refs("Artist", Vec::new()),
            &[older, newer],
            2026,
        );

        assert_eq!(album.added_at, Some(2_000));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_has_no_date() {
        let dir = scratch("sonora-wire-test-missing");
        assert_eq!(modified_at(&dir.join("gone.mp3")), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(windows)]
    #[test]
    fn a_tagged_file_whose_name_is_not_unicode_still_scans() {
        use std::os::windows::ffi::OsStringExt;

        log::set_max_level(log::LevelFilter::Warn);
        let dir = scratch("sonora-wire-test-not-unicode");
        let name = std::ffi::OsString::from_wide(&[0xd800, 0x2e, 0x6d, 0x70, 0x33]);
        let path = dir.join(name);
        std::fs::write(&path, b"ID3\x04\x00\x00\x00\x00\x00\x00").unwrap();

        assert!(track_from_file(&path, None, None, &dir).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }
}
