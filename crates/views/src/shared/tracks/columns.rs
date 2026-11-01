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
    AddedAt,
    AddedBy,
    PlayedAt,
    Plays,
    Duration,
}

const LENGTH: Pixels = px(84.);
const CREDITED: Pixels = px(144.);
const CONTRIBUTOR: Pixels = px(168.);
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

pub(crate) const LIBRARY_COLUMNS: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, ADDED_AT, DURATION];

pub(crate) const PLAYLIST_COLUMNS_SHARED: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, CREDITED_AT, DURATION];

pub(crate) const PLAYLIST_COLUMNS_BLEND: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, ADDED_BY, DURATION];

pub(crate) const HISTORY_COLUMNS: &[ColumnSpec<TrackField>] =
    &[INDEX, COVER, TITLE, ARTISTS, ALBUM, PLAYED_AT, DURATION];

pub(crate) const ARTIST_COLUMNS: &[ColumnSpec<TrackField>] = &[
    INDEX,
    COVER,
    TITLE,
    ARTISTS.ranked(SPARE),
    ALBUM.ranked(HANDY),
    PLAYS,
    DURATION,
];

pub(crate) const ARTIST_COLUMNS_LEAN: &[ColumnSpec<TrackField>] = &[
    INDEX,
    COVER,
    TITLE,
    ARTISTS.ranked(SPARE),
    ALBUM.ranked(HANDY),
    DURATION,
];

pub(crate) const ALBUM_COLUMNS: &[ColumnSpec<TrackField>] = &[
    INDEX,
    ALBUM_TITLE,
    ALBUM_ARTISTS,
    ALBUM_PLAYCOUNT,
    ALBUM_DURATION,
];

pub(crate) const ALBUM_COLUMNS_LEAN: &[ColumnSpec<TrackField>] =
    &[INDEX, ALBUM_TITLE, ALBUM_ARTISTS, ALBUM_DURATION];

pub(crate) fn playlist_columns(blend: bool, shared: bool) -> &'static [ColumnSpec<TrackField>] {
    match (blend, shared) {
        (true, _) => PLAYLIST_COLUMNS_BLEND,
        (false, true) => PLAYLIST_COLUMNS_SHARED,
        (false, false) => LIBRARY_COLUMNS,
    }
}

pub(crate) fn artist_columns(playcounts: bool) -> &'static [ColumnSpec<TrackField>] {
    match playcounts {
        true => ARTIST_COLUMNS,
        false => ARTIST_COLUMNS_LEAN,
    }
}

pub(crate) fn album_columns(playcounts: bool) -> &'static [ColumnSpec<TrackField>] {
    match playcounts {
        true => ALBUM_COLUMNS,
        false => ALBUM_COLUMNS_LEAN,
    }
}
