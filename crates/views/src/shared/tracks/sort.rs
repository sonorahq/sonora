use std::cmp::Ordering;

use gpui::SharedString;
use music::{Arrival, Track};

use super::TrackField;
use crate::shared::text::{folded, holds};

pub(super) fn compare(tracks: &[Track], field: TrackField, a: usize, b: usize) -> Ordering {
    let text =
        |index: usize, pick: fn(&Track) -> &str| tracks.get(index).map(pick).unwrap_or_default();
    let folded = |pick: fn(&Track) -> &str| folded(text(a, pick), text(b, pick));

    match field {
        TrackField::Title => folded(|track| &track.name),
        TrackField::Artists => folded(|track| &track.artists),
        TrackField::Album => folded(|track| &track.album),
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

/// Orders two rows of a list that comes in the order it was added, oldest first, so a
/// descending sort puts the latest addition on top whichever end the provider lists it at.
pub(super) fn by_arrival(arrival: Arrival, a: usize, b: usize) -> Ordering {
    match arrival {
        Arrival::OldestFirst => a.cmp(&b),
        Arrival::NewestFirst => b.cmp(&a),
    }
}

pub(super) fn group(tracks: &[Track], field: TrackField, row: usize) -> Option<SharedString> {
    let track = tracks.get(row)?;

    match field {
        TrackField::Title => Some(initial(&track.name)),
        TrackField::Artists => Some(initial(&track.artists)),
        TrackField::Album => Some(initial(&track.album)),
        TrackField::AddedBy => Some(initial(contributor(track))),
        _ => None,
    }
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
