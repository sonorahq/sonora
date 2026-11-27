use gpui::{Pixels, TextAlign, px};
use ui::rank::{ESSENTIAL, HANDY, NICE, SPARE, USEFUL};
use ui::{ColumnSpec, Sort, Width};

use crate::shared::cells::{DATE, NUMBER};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrackField {
    Index,
    Cover,
    Title,
    Artists,
    Album,
    Genre,
    Mood,
    Bpm,
    Tags,
    Year,
    Popularity,
    AddedAt,
    LikedAt,
    AddedBy,
    PlayedAt,
    Plays,
    Duration,
}

const LENGTH: Pixels = px(84.);
const CREDITED: Pixels = px(144.);
const CONTRIBUTOR: Pixels = px(168.);
const FILED: Pixels = px(132.);
const ALBUM_LENGTH: Pixels = px(120.);
const ALBUM_PLAYS: Pixels = px(108.);

const COLUMN: ColumnSpec<TrackField> = ColumnSpec::filling(TrackField::Index);

const INDEX: ColumnSpec<TrackField> = ColumnSpec::numbering(TrackField::Index, NUMBER);

const COVER: ColumnSpec<TrackField> = ColumnSpec::artwork(TrackField::Cover);

const TITLE: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Title,
    key: "title",
    header: "column-title",
    width: Width::Fill(0.515),
    rank: ESSENTIAL,
    ..COLUMN
};

const ARTISTS: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Artists,
    key: "artists",
    header: "column-artist",
    width: Width::Fill(0.212),
    rank: HANDY,
    ..COLUMN
};

const ALBUM: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Album,
    key: "album",
    header: "column-album",
    width: Width::Fill(0.273),
    rank: SPARE,
    ..COLUMN
};

/// The genres a track is filed under, shown once the listener turns the column on. It ranks with
/// the album and sits after it, so a narrow table drops the genres first.
const GENRE: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Genre,
    key: "genre",
    header: "column-genre",
    width: Width::Fixed(FILED),
    rank: SPARE,
    optional: true,
    ..COLUMN
};

const MOOD: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Mood,
    key: "mood",
    header: "common-mood",
    ..GENRE
};

const BPM: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Bpm,
    key: "bpm",
    header: "common-bpm",
    width: Width::Fixed(LENGTH),
    align: TextAlign::Right,
    ..GENRE
};

const TAGS: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Tags,
    key: "tags",
    header: "column-tags",
    ..GENRE
};

const YEAR: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Year,
    key: "year",
    header: "column-year",
    width: Width::Fixed(LENGTH),
    align: TextAlign::Right,
    first: Sort::Descending,
    ..GENRE
};

const POPULARITY: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Popularity,
    key: "popularity",
    header: "column-popularity",
    first: Sort::Descending,
    ..YEAR
};

const ADDED_AT: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::AddedAt,
    key: "added-at",
    header: "column-date-added",
    width: Width::Fixed(DATE),
    rank: NICE,
    first: Sort::Descending,
    ..COLUMN
};

/// The day a favorite was made, for a shelf that lists its whole catalog and so dates each row by
/// when it joined. It stays off until the listener turns it on, and sorts from the sort menu.
const LIKED_AT: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::LikedAt,
    key: "liked-at",
    header: "column-date-liked",
    optional: true,
    ..ADDED_AT
};

const ADDED_BY: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::AddedBy,
    key: "added-by",
    header: "column-added-by",
    width: Width::Fixed(CONTRIBUTOR),
    rank: NICE,
    ..COLUMN
};

const CREDITED_AT: ColumnSpec<TrackField> = ColumnSpec {
    width: Width::Fixed(CREDITED),
    ..ADDED_AT
};

const PLAYS: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Plays,
    key: "plays",
    header: "column-plays",
    width: Width::Fixed(DATE),
    rank: NICE,
    ..COLUMN
};

const DURATION: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::Duration,
    key: "duration",
    header: "column-length",
    align: TextAlign::Right,
    width: Width::Fixed(LENGTH),
    rank: USEFUL,
    ..COLUMN
};

const PLAYED_AT: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::PlayedAt,
    key: "played-at",
    header: "column-played-at",
    width: Width::Fixed(px(168.)),
    rank: NICE,
    ..COLUMN
};

const ALBUM_TITLE: ColumnSpec<TrackField> = ColumnSpec {
    width: Width::Fill(0.665),
    ..TITLE
};

const ALBUM_ARTISTS: ColumnSpec<TrackField> = ColumnSpec {
    width: Width::Fill(0.335),
    ..ARTISTS
};

const ALBUM_PLAYCOUNT: ColumnSpec<TrackField> = ColumnSpec {
    width: Width::Fixed(ALBUM_PLAYS),
    ..PLAYS
};

const ALBUM_DURATION: ColumnSpec<TrackField> = ColumnSpec {
    width: Width::Fixed(ALBUM_LENGTH),
    ..DURATION
};

type Spec = ColumnSpec<TrackField>;

/// Whether a column is metadata a listener may turn on in any table, rather than part of the
/// frame every table keeps (index, cover, title, artists, album and length).
const fn is_metadata(field: TrackField) -> bool {
    matches!(
        field,
        TrackField::Genre
            | TrackField::Mood
            | TrackField::Bpm
            | TrackField::Tags
            | TrackField::Year
            | TrackField::Popularity
            | TrackField::Plays
            | TrackField::AddedAt
            | TrackField::LikedAt
            | TrackField::AddedBy
            | TrackField::PlayedAt
    )
}

const fn listed(fields: &[TrackField], field: TrackField) -> bool {
    let mut at = 0;
    while at < fields.len() {
        if fields[at] as u8 == field as u8 {
            return true;
        }
        at += 1;
    }
    false
}

/// Builds one table from the frame it opens with. Every table offers the same metadata, in the
/// same order; `shown` names the ones it starts with and `tuned` swaps in a table's own width or
/// rank for a column.
const fn table<const N: usize>(
    frame: [Spec; N],
    tuned: &[Spec],
    shown: &[TrackField],
) -> [Spec; N] {
    let mut specs = frame;
    let mut at = 0;
    while at < N {
        let mut tune = 0;
        while tune < tuned.len() {
            if tuned[tune].field as u8 == specs[at].field as u8 {
                specs[at] = tuned[tune];
            }
            tune += 1;
        }
        if is_metadata(specs[at].field) {
            specs[at].optional = !listed(shown, specs[at].field);
        }
        at += 1;
    }
    specs
}

/// The columns of a table that lists tracks from many albums. `$dated` is the date column it
/// reads `added_at` as: when a song was added, or when it was played in the history.
macro_rules! songs {
    ($dated:expr, $tuned:expr, $shown:expr) => {
        &table(
            [
                INDEX, COVER, TITLE, ARTISTS, ALBUM, GENRE, MOOD, BPM, TAGS, YEAR, POPULARITY,
                PLAYS, $dated, LIKED_AT, ADDED_BY, DURATION,
            ],
            $tuned,
            $shown,
        )
    };
}

/// The columns of a table that lists the tracks of one album, which needs no cover or album.
const ALBUM_FRAME: [Spec; 14] = [
    INDEX,
    ALBUM_TITLE,
    ALBUM_ARTISTS,
    GENRE,
    MOOD,
    BPM,
    TAGS,
    YEAR,
    POPULARITY,
    ALBUM_PLAYCOUNT,
    ADDED_AT,
    LIKED_AT,
    ADDED_BY,
    ALBUM_DURATION,
];

pub(crate) const LIBRARY_COLUMNS: &[Spec] = songs!(ADDED_AT, &[], &[TrackField::AddedAt]);

const PLAYLIST_COLUMNS_SHARED: &[Spec] = songs!(CREDITED_AT, &[], &[TrackField::AddedAt]);

const PLAYLIST_COLUMNS_BLEND: &[Spec] = songs!(ADDED_AT, &[], &[TrackField::AddedBy]);

pub(crate) const HISTORY_COLUMNS: &[Spec] = songs!(PLAYED_AT, &[], &[TrackField::PlayedAt]);

pub(crate) const ARTIST_COLUMNS: &[Spec] = songs!(
    ADDED_AT,
    &[ARTISTS.ranked(SPARE), ALBUM.ranked(HANDY)],
    &[TrackField::Plays]
);

pub(crate) const ALBUM_COLUMNS: &[Spec] = &table(ALBUM_FRAME, &[], &[TrackField::Plays]);

pub(crate) fn playlist_columns(blend: bool, shared: bool) -> &'static [Spec] {
    match (blend, shared) {
        (true, _) => PLAYLIST_COLUMNS_BLEND,
        (false, true) => PLAYLIST_COLUMNS_SHARED,
        (false, false) => LIBRARY_COLUMNS,
    }
}
