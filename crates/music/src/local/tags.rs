use std::path::Path;

use anyhow::{Context as _, Result};
use lofty::config::WriteOptions;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::{Accessor, ItemKey};
use lofty::probe::Probe;
use lofty::tag::Tag;
use lofty::tag::items::Timestamp;

use crate::TrackTags;

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
        artist: text(tag.artist()),
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

pub fn write(path: &Path, tags: &TrackTags) -> Result<()> {
    update(path, |tag| {
        set(tag, ItemKey::TrackTitle, &tags.title);
        set(tag, ItemKey::TrackArtist, &tags.artist);
        set(tag, ItemKey::AlbumTitle, &tags.album);
        set(tag, ItemKey::AlbumArtist, &tags.album_artist);
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

fn update(path: &Path, change: impl FnOnce(&mut Tag)) -> Result<()> {
    let mut tagged = Probe::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .read()
        .with_context(|| format!("cannot read the tags in {}", path.display()))?;
    if tagged.primary_tag().is_none() && tagged.first_tag().is_none() {
        let kind = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(kind));
    }
    let held = tagged.primary_tag_mut().is_some();
    let tag = match held {
        true => tagged.primary_tag_mut(),
        false => tagged.first_tag_mut(),
    };
    let Some(tag) = tag else {
        anyhow::bail!("{} cannot hold tags", path.display());
    };
    change(tag);

    tagged
        .save_to_path(path, WriteOptions::default())
        .with_context(|| format!("cannot save the tags in {}", path.display()))?;
    Ok(())
}

/// Writes the year, leaving a date that already falls in it alone so its month and day survive.
fn set_year(tag: &mut Tag, value: &str) {
    let wanted = year(value);
    if wanted.is_some() && wanted == tag.date().map(|date| date.year) {
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

fn number(value: Option<u32>) -> String {
    value
        .filter(|value| *value > 0)
        .map(|value| value.to_string())
        .unwrap_or_default()
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

fn counted(tag: &mut Tag, key: ItemKey, value: &str) {
    match value.trim().parse::<u32>().ok().filter(|value| *value > 0) {
        Some(value) => set(tag, key, &value.to_string()),
        None => set(tag, key, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flac(path: &Path, comments: &[&str]) {
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
