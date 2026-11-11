use std::cmp::Ordering;

use std::collections::HashMap;

use gpui::SharedString;
use music::{Descriptors, Track};

use super::TrackField;
use crate::shared::text::{folded, holds};

/// What the provider says of the tracks by id, where it says anything.
pub(super) type Found<'a> = Option<&'a HashMap<String, Descriptors>>;

/// The provider's extra metadata for a track, when available.
pub(super) fn described<'a>(track: &Track, found: Found<'a>) -> Option<&'a Descriptors> {
    found?.get(track.id.as_deref()?)
}

/// The genres a track is filed under. Tags the provider put on the track win, and a track with
/// none takes the genres the provider described it with, which arrive after the list does.
pub(super) fn genres_of<'a>(track: &'a Track, found: Found<'a>) -> &'a [String] {
    if !track.tags.is_empty() {
        return &track.tags;
    }
    described(track, found)
        .map(|found| found.genres.as_slice())
        .unwrap_or_default()
}

pub(super) fn compare(
    tracks: &[Track],
    found: Found,
    field: TrackField,
    a: usize,
    b: usize,
) -> Ordering {
    let text =
        |index: usize, pick: fn(&Track) -> &str| tracks.get(index).map(pick).unwrap_or_default();
    let folded = |pick: fn(&Track) -> &str| folded(text(a, pick), text(b, pick));

    match field {
        TrackField::Title => folded(|track| &track.name),
        TrackField::Artists => folded(|track| &track.artists),
        TrackField::Album => folded(|track| &track.album),
        TrackField::Genre => genres(genres_at(tracks, found, a), genres_at(tracks, found, b)),
        TrackField::AddedAt | TrackField::PlayedAt => tracks
            .get(a)
            .and_then(|track| track.added_at)
            .cmp(&tracks.get(b).and_then(|track| track.added_at)),
        TrackField::AddedBy => folded(contributor),
        TrackField::Plays => tracks
            .get(a)
            .and_then(|track| track.playcount)
            .cmp(&tracks.get(b).and_then(|track| track.playcount)),
        TrackField::Duration => tracks
            .get(a)
            .map(|track| track.duration)
            .cmp(&tracks.get(b).map(|track| track.duration)),
        TrackField::Index | TrackField::Cover => a.cmp(&b),
    }
}

pub(super) fn group(
    tracks: &[Track],
    found: Found,
    field: TrackField,
    row: usize,
) -> Option<SharedString> {
    let track = tracks.get(row)?;

    match field {
        TrackField::Title => Some(initial(&track.name)),
        TrackField::Artists => Some(initial(&track.artists)),
        TrackField::Album => Some(initial(&track.album)),
        TrackField::Genre => Some(match genres_of(track, found).first() {
            Some(genre) => SharedString::from(genre.clone()),
            None => initial(""),
        }),
        TrackField::AddedBy => Some(initial(contributor(track))),
        _ => None,
    }
}

fn genres_at<'a>(tracks: &'a [Track], found: Found<'a>, index: usize) -> &'a [String] {
    tracks
        .get(index)
        .map(|track| genres_of(track, found))
        .unwrap_or_default()
}

/// Orders two genre lists by their first genre that differs, ignoring case. A list that runs out
/// first sorts ahead, so a track with no genre comes before every tagged one.
fn genres(left: &[String], right: &[String]) -> Ordering {
    left.iter()
        .zip(right)
        .map(|(left, right)| folded(left, right))
        .find(|order| order.is_ne())
        .unwrap_or_else(|| left.len().cmp(&right.len()))
}

fn contributor(track: &Track) -> &str {
    track
        .added_by
        .as_ref()
        .map(|added| added.name.as_str())
        .unwrap_or_default()
}

pub(crate) fn initial(text: &str) -> SharedString {
    text.chars()
        .next()
        .filter(|first| first.is_alphabetic())
        .map(|first| SharedString::from(first.to_uppercase().collect::<String>()))
        .unwrap_or_else(|| SharedString::from("#"))
}

pub(super) fn hits(track: &Track, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    [&track.name, &track.artists, &track.album]
        .into_iter()
        .map(String::as_str)
        .chain(std::iter::once(contributor(track)))
        .any(|field| holds(field, query))
}

#[cfg(test)]
mod tests {
    use super::initial;

    #[test]
    fn letters_bucket_under_their_uppercase_form() {
        assert_eq!(initial("bark at the moon"), "B");
        assert_eq!(initial("Bark at the Moon"), "B");
    }

    #[test]
    fn cyrillic_keeps_its_own_letter() {
        assert_eq!(initial("прощай"), "П");
        assert_eq!(initial("Ялта"), "Я");
    }

    #[test]
    fn digits_punctuation_and_emptiness_share_one_bucket() {
        assert_eq!(initial("99 Luftballons"), "#");
        assert_eq!(initial("!!!"), "#");
        assert_eq!(initial(" leading space"), "#");
        assert_eq!(initial(""), "#");
    }

    #[test]
    fn multi_char_uppercase_is_kept_whole() {
        assert_eq!(initial("ßeta"), "SS");
    }
}
