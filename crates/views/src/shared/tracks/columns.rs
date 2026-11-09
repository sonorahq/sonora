//! The track table's columns: which fields a list shows, how wide, how they rank when the
//! table narrows and which order a first click sorts them in.

use ui::rank::{ESSENTIAL, HANDY, NICE, SPARE, USEFUL};
use ui::{ColumnSpec, Sort, TextAlign, Width};

use crate::shared::cells::{DATE, NUMBER};

/// What one column of a track table shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TrackField {
    Index,
    Cover,
    Title,
    Artists,
    Album,
    AddedAt,
    AddedBy,
    PlayedAt,
    Plays,
    Duration,
}

const LENGTH: f32 = 84.;
const CREDITED: f32 = 144.;
const CONTRIBUTOR: f32 = 168.;
const ALBUM_LENGTH: f32 = 120.;
const ALBUM_PLAYS: f32 = 108.;
const PLAYED: f32 = 168.;

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

const ADDED_AT: ColumnSpec<TrackField> = ColumnSpec {
    field: TrackField::AddedAt,
    key: "added-at",
    header: "column-date-added",
    width: Width::Fixed(DATE),
    rank: NICE,
    first: Sort::Descending,
    ..COLUMN
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
    width: Width::Fixed(PLAYED),
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

/// The library's songs, and a playlist that is neither shared nor a blend.
pub(crate) const LIBRARY_COLUMNS: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, ADDED_AT, DURATION];

/// A playlist others add to: the date column shows who added each song.
pub(crate) const PLAYLIST_COLUMNS_SHARED: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, CREDITED_AT, DURATION];

/// A blend, whose songs each come from one member.
pub(crate) const PLAYLIST_COLUMNS_BLEND: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, ADDED_BY, DURATION];

/// The listening history, dated by when each song played.
pub(crate) const HISTORY_COLUMNS: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, PLAYED_AT, DURATION];

/// An artist's songs, with play counts.
pub(crate) const ARTIST_COLUMNS: &[ColumnSpec<TrackField>] = &[
    INDEX,
    COVER,
    TITLE,
    ARTISTS.ranked(SPARE),
    ALBUM.ranked(HANDY),
    PLAYS,
    DURATION,
];

/// An artist's songs where the provider has no play counts.
pub(crate) const ARTIST_COLUMNS_LEAN: &[ColumnSpec<TrackField>] = &[
    INDEX,
    COVER,
    TITLE,
    ARTISTS.ranked(SPARE),
    ALBUM.ranked(HANDY),
    DURATION,
];

/// An album's tracks, with play counts and without covers.
pub(crate) const ALBUM_COLUMNS: &[ColumnSpec<TrackField>] = &[
    INDEX,
    ALBUM_TITLE,
    ALBUM_ARTISTS,
    ALBUM_PLAYCOUNT,
    ALBUM_DURATION,
];

/// An album's tracks where the provider has no play counts.
pub(crate) const ALBUM_COLUMNS_LEAN: &[ColumnSpec<TrackField>] =
    &[INDEX, ALBUM_TITLE, ALBUM_ARTISTS, ALBUM_DURATION];

/// The columns of a playlist page.
pub(crate) fn playlist_columns(blend: bool, shared: bool) -> &'static [ColumnSpec<TrackField>] {
    match (blend, shared) {
        (true, _) => PLAYLIST_COLUMNS_BLEND,
        (false, true) => PLAYLIST_COLUMNS_SHARED,
        (false, false) => LIBRARY_COLUMNS,
    }
}

/// The columns of an artist page's song list.
pub(crate) fn artist_columns(playcounts: bool) -> &'static [ColumnSpec<TrackField>] {
    match playcounts {
        true => ARTIST_COLUMNS,
        false => ARTIST_COLUMNS_LEAN,
    }
}

/// The columns of an album page.
pub(crate) fn album_columns(playcounts: bool) -> &'static [ColumnSpec<TrackField>] {
    match playcounts {
        true => ALBUM_COLUMNS,
        false => ALBUM_COLUMNS_LEAN,
    }
}
