use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::Accessor;
use lofty::probe::Probe;
use lofty::tag::Tag;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, StandardTagKey};
use symphonia::core::probe::Hint;

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

const PLAYABLE_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "wav", "opus", "webm", "mka", "wv", "ape",
];

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

pub fn album_id(dir: &Path) -> String {
    format!("{LOCAL_ALBUM_PREFIX}{}", dir.display())
}

pub fn path_from_album_id(id: &str) -> Option<&Path> {
    id.strip_prefix(LOCAL_ALBUM_PREFIX).map(Path::new)
}

pub fn artist_id(name: &str) -> String {
    format!("{LOCAL_ARTIST_PREFIX}{name}")
}

pub fn artist_name_from_id(id: &str) -> Option<&str> {
    id.strip_prefix(LOCAL_ARTIST_PREFIX)
}

fn artist_ref(name: &str) -> ArtistRef {
    ArtistRef {
        name: name.to_owned(),
        id: Some(artist_id(name)),
    }
}

fn clean(value: Option<std::borrow::Cow<'_, str>>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn infer_from_stem(stem: &str) -> (Option<String>, Option<String>) {
    let split = stem
        .rsplit_once(" - ")
        .or_else(|| stem.rsplit_once(" \u{2013} "))
        .or_else(|| stem.rsplit_once(" \u{2014} "));

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
            return (Some(left.to_owned()), Some(right.to_owned()));
        }
    }
    (None, None)
}

struct FallbackProbe {
    duration: std::time::Duration,
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    track_number: u32,
    disc_number: u32,
    year: Option<i32>,
    cover_data: Option<(Vec<u8>, String)>,
}

fn probe_symphonia(path: &Path) -> Option<FallbackProbe> {
    let file = std::fs::File::open(path).ok()?;
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
                        year = s[..4].parse::<i32>().ok().filter(|y| *y > 0);
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

pub fn track_from_file(
    path: &Path,
    artist_hint: Option<&str>,
    album: Option<(&str, &Path)>,
    cache_dir: &Path,
) -> Option<Track> {
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

    let (inferred_title, inferred_artist) = infer_from_stem(&file_stem(path));

    let name = clean(tag.and_then(Accessor::title))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.title.clone()))
        .or(inferred_title)
        .unwrap_or_else(|| file_stem(path));

    let artist = clean(tag.and_then(Accessor::artist))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.artist.clone()))
        .or_else(|| artist_hint.map(str::to_owned))
        .or(inferred_artist)
        .unwrap_or_else(|| "Unknown Artist".to_owned());

    let album_name = clean(tag.and_then(Accessor::album))
        .or_else(|| fallback.as_ref().and_then(|fb| fb.album.clone()))
        .or_else(|| album.map(|(name, _)| name.to_owned()))
        .unwrap_or_default();

    let album_dir = album.map(|(_, dir)| dir);

    let track_number = tag
        .and_then(Accessor::track)
        .or_else(|| {
            fallback
                .as_ref()
                .map(|fb| fb.track_number)
                .filter(|n| *n > 0)
        })
        .unwrap_or(0);

    let disc_number = tag
        .and_then(Accessor::disk)
        .or_else(|| {
            fallback
                .as_ref()
                .map(|fb| fb.disc_number)
                .filter(|n| *n > 0)
        })
        .unwrap_or(0);

    let mut cover = extract_cover(tag, path, album_dir, cache_dir);
    if cover.is_none()
        && let Some(ref fb) = fallback
        && let Some((ref data, ref mime)) = fb.cover_data
    {
        cover = cache_image_data(data, mime, cache_dir);
    }

    Some(Track {
        id: Some(track_id(path)),
        name,
        playable: is_playable(path),
        artists: artist.clone(),
        artist_refs: vec![artist_ref(&artist)],
        album: album_name,
        album_id: album_dir.map(album_id),
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
    })
}

pub fn tag_year(path: &Path) -> Option<i32> {
    if let Some(tagged) = Probe::open(path).ok().and_then(|probe| probe.read().ok())
        && let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag())
        && let Some(year) = tag
            .date()
            .map(|date| date.year as i32)
            .filter(|year| *year > 0)
    {
        return Some(year);
    }
    probe_symphonia(path).and_then(|fb| fb.year)
}

pub fn album_from_tracks(
    name: &str,
    artist: &str,
    dir: &Path,
    tracks: &[Track],
    year: i32,
) -> Album {
    let cover = folder_cover(dir).or_else(|| tracks.iter().find_map(|track| track.cover.clone()));
    Album {
        id: album_id(dir),
        name: name.to_owned(),
        artists: artist.to_owned(),
        artist_refs: vec![artist_ref(artist)],
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

fn extract_cover(
    tag: Option<&Tag>,
    path: &Path,
    album_dir: Option<&Path>,
    cache_dir: &Path,
) -> Option<String> {
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

    path.parent()
        .and_then(folder_cover)
        .or_else(|| album_dir.and_then(folder_cover))
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
        let track = track_from_file(path, None, None, &temp).expect("track parsed");
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
        let track = track_from_file(&path, None, None, &dir).expect("a track");

        assert_eq!(track.added_at, Some(stamped));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_album_is_dated_by_its_newest_track() {
        let dir = scratch("sonora-wire-test-album-dated");
        let path = dir.join("song.mp3");
        std::fs::write(&path, []).unwrap();

        let mut older = track_from_file(&path, None, None, &dir).expect("a track");
        let mut newer = older.clone();
        older.added_at = Some(1_000);
        newer.added_at = Some(2_000);

        let album = album_from_tracks("Album", "Artist", &dir, &[older, newer], 2026);

        assert_eq!(album.added_at, Some(2_000));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_has_no_date() {
        let dir = scratch("sonora-wire-test-missing");
        assert_eq!(modified_at(&dir.join("gone.mp3")), None);
        std::fs::remove_dir_all(&dir).ok();
    }
}
