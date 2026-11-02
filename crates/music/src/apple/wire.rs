//! Apple Music's JSON turned into Sonora's models.
//!
//! Every resource arrives in the same envelope: an `id`, a `type`, an `attributes` object and
//! sometimes `relationships`. A library resource carries its own id (`i.`, `l.`, `r.`, `p.`)
//! and, when it was asked for with `include=catalog`, the catalog resource it came from. Only
//! the catalog id can be played or opened, so a library row that has none is dropped.
//!
//! An answer is read straight from its text into the shapes below, which keep only the fields a
//! reader here looks at and borrow their strings from that text. A field Apple sends in some
//! other shape than the one expected reads as absent, the way a missing one does.
//!
//! Nothing here fails the caller: a row that cannot be read is left out, which is what keeps one
//! odd entry in a library of thousands from emptying a page.

use std::borrow::Cow;
use std::fmt::{self, Write as _};
use std::marker::PhantomData;
use std::time::Duration;

use serde::Deserialize;
use serde::de::value::MapAccessDeserializer;
use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

use crate::{
    Album, Artist, ArtistProfile, ArtistRef, PinTarget, PinTargetKind, Playlist, ReleaseType,
    SavedArtist, Track, calendar,
};

/// The size artwork is asked to render at, and the larger one for a page header.
pub const ART: u32 = 600;
pub const HERO: u32 = 1200;
/// The widest an artist banner is asked for, since it spans the whole page.
const BANNER: u64 = 2400;

/// The artist views the page shows first, before every edition arrives from the artist's albums.
/// Apple keeps live albums and compilations out of `full-albums`, so each needs its own view.
pub const DISCOGRAPHY: [&str; 4] = [
    "full-albums",
    "live-albums",
    "compilation-albums",
    "singles",
];

/// One field of an answer as whatever JSON Apple put at its place. A leaf is never looked
/// inside, so an object or an array there is skipped and only noted as present.
#[derive(Default)]
pub enum Leaf<'a> {
    /// Nothing was sent under this name.
    #[default]
    Absent,
    Text(Cow<'a, str>),
    Unsigned(u64),
    Signed(i64),
    Real(f64),
    Flag(bool),
    /// Present, but null, an object or an array.
    Other,
}

/// The top level of an answer. The rows sit under `data`, and `results` holds whatever else an
/// endpoint answers with, read as `R`.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a, R: Loose<'de>"))]
pub struct Answer<'a, R = IgnoredAny> {
    #[serde(deserialize_with = "loose")]
    pub data: Vec<Resource<'a>>,
    pub next: Leaf<'a>,
    #[serde(deserialize_with = "loose")]
    pub meta: Meta<'a>,
    #[serde(deserialize_with = "loose")]
    pub results: R,
    /// The resources of an answer asked for with `format[resources]=map`, by type and then id.
    #[serde(deserialize_with = "loose")]
    pub resources: Members<'a, Members<'a, Resource<'a>>>,
}

/// A page of a listing with every row left as the text it arrived as, so a row is only parsed
/// while a reader looks at it.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Outline<'a> {
    #[serde(deserialize_with = "loose")]
    pub data: Vec<Raw<'a>>,
    pub next: Leaf<'a>,
    #[serde(deserialize_with = "loose")]
    pub meta: Meta<'a>,
}

/// One row of a page, kept as the text it arrived as.
pub struct Raw<'a>(&'a RawValue);

#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Meta<'a> {
    pub total: Leaf<'a>,
}

/// The `results` of a search, one relation per type of hit.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Search<'a> {
    #[serde(deserialize_with = "loose")]
    pub songs: Relation<'a>,
    #[serde(deserialize_with = "loose")]
    pub albums: Relation<'a>,
    #[serde(deserialize_with = "loose")]
    pub playlists: Relation<'a>,
    #[serde(rename = "library-artists", deserialize_with = "loose")]
    pub library_artists: Relation<'a>,
}

/// The `results` of the charts, a list of charts per type.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Charts<'a> {
    #[serde(deserialize_with = "loose")]
    pub songs: Vec<Relation<'a>>,
    #[serde(deserialize_with = "loose")]
    pub albums: Vec<Relation<'a>>,
    #[serde(deserialize_with = "loose")]
    pub playlists: Vec<Relation<'a>>,
}

/// The `results` of a station built from a track: the station and its first tracks.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Seeded<'a> {
    #[serde(deserialize_with = "loose")]
    pub station: Station<'a>,
    #[serde(deserialize_with = "loose")]
    pub tracks: Vec<Resource<'a>>,
}

#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Station<'a> {
    pub id: Leaf<'a>,
}

/// One resource, with only the parts a reader here looks at.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Resource<'a> {
    pub id: Leaf<'a>,
    #[serde(rename = "type")]
    pub kind: Leaf<'a>,
    /// Present whenever the row names `attributes` at all, even as something other than an
    /// object.
    #[serde(deserialize_with = "loose")]
    pub attributes: Option<Attributes<'a>>,
    #[serde(deserialize_with = "loose")]
    pub relationships: Relationships<'a>,
    #[serde(deserialize_with = "loose")]
    pub views: Views<'a>,
}

/// The attributes of every kind of resource read here, in one shape.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", bound(deserialize = "'de: 'a"))]
pub struct Attributes<'a> {
    pub name: Leaf<'a>,
    pub artist_name: Leaf<'a>,
    pub album_name: Leaf<'a>,
    pub curator_name: Leaf<'a>,
    pub content_rating: Leaf<'a>,
    pub release_date: Leaf<'a>,
    pub record_label: Leaf<'a>,
    pub copyright: Leaf<'a>,
    pub artist_bio: Leaf<'a>,
    pub date_added: Leaf<'a>,
    pub last_modified_date: Leaf<'a>,
    pub duration_in_millis: Leaf<'a>,
    pub track_number: Leaf<'a>,
    pub disc_number: Leaf<'a>,
    pub track_count: Leaf<'a>,
    pub is_single: Leaf<'a>,
    pub is_compilation: Leaf<'a>,
    pub can_edit: Leaf<'a>,
    pub has_collaboration: Leaf<'a>,
    pub is_public: Leaf<'a>,
    /// Only whether it is there matters: every real genre has a parent.
    pub parent_id: Leaf<'a>,
    pub personal_rating: Leaf<'a>,
    /// What a recommendation group is, such as `recently-played`.
    pub kind: Leaf<'a>,
    #[serde(deserialize_with = "loose")]
    pub genre_names: Vec<Leaf<'a>>,
    /// Present whenever Apple sent play parameters at all, which is what makes a song
    /// streamable.
    #[serde(deserialize_with = "loose")]
    pub play_params: Option<PlayParams<'a>>,
    #[serde(deserialize_with = "loose")]
    pub artwork: Artwork<'a>,
    /// The wide pictures Apple keeps for an artist page, sent only when asked for with
    /// `extend=editorialArtwork`.
    #[serde(deserialize_with = "loose")]
    pub editorial_artwork: Editorial<'a>,
    #[serde(deserialize_with = "loose")]
    pub editorial_notes: Notes<'a>,
    #[serde(deserialize_with = "loose")]
    pub title: Title<'a>,
    #[serde(deserialize_with = "loose")]
    pub loudness: Loudness<'a>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", bound(deserialize = "'de: 'a"))]
pub struct PlayParams<'a> {
    pub catalog_id: Leaf<'a>,
    /// The catalog song a library row plays as now. It differs from `catalog_id` when Apple
    /// pulled the old id and delivered the song again under a new one.
    pub reporting_id: Leaf<'a>,
}

#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Artwork<'a> {
    pub url: Leaf<'a>,
    pub width: Leaf<'a>,
    pub height: Leaf<'a>,
}

/// The editorial pictures of an artist, in the order a page banner prefers them.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", bound(deserialize = "'de: 'a"))]
pub struct Editorial<'a> {
    #[serde(deserialize_with = "loose")]
    pub banner_uber: Artwork<'a>,
    #[serde(deserialize_with = "loose")]
    pub super_hero_wide: Artwork<'a>,
    #[serde(deserialize_with = "loose")]
    pub centered_fullscreen_background: Artwork<'a>,
}

/// What Apple writes about an artist. Some answers send it as one string, kept in `text`, and
/// others as an object with a `standard` and a `short` text.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Notes<'a> {
    #[serde(skip)]
    pub text: Leaf<'a>,
    pub standard: Leaf<'a>,
    pub short: Leaf<'a>,
}

/// The title of a recommendation group.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase", bound(deserialize = "'de: 'a"))]
pub struct Title<'a> {
    pub string_for_display: Leaf<'a>,
}

/// The loudness an audio analysis measured, as integrated LUFS and a true peak in dBFS.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Loudness<'a> {
    #[serde(deserialize_with = "loose")]
    pub main: Measure<'a>,
}

#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Measure<'a> {
    pub value: Leaf<'a>,
    pub peak: Leaf<'a>,
}

/// The relationships of a resource that any reader here follows.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Relationships<'a> {
    #[serde(deserialize_with = "loose")]
    pub catalog: Relation<'a>,
    #[serde(deserialize_with = "loose")]
    pub artists: Relation<'a>,
    #[serde(deserialize_with = "loose")]
    pub albums: Relation<'a>,
    #[serde(deserialize_with = "loose")]
    pub tracks: Relation<'a>,
    #[serde(deserialize_with = "loose")]
    pub contents: Relation<'a>,
    #[serde(rename = "audio-analysis", deserialize_with = "loose")]
    pub audio_analysis: Relation<'a>,
}

/// One relationship or view: the resources it holds.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Relation<'a> {
    /// Present whenever the relationship names `data` at all. An included catalog relationship
    /// that came back without one names a song Apple has since pulled.
    #[serde(deserialize_with = "loose")]
    pub data: Option<Vec<Resource<'a>>>,
}

/// The named views an artist or an album page is asked for.
#[derive(Default, Deserialize)]
#[serde(default, bound(deserialize = "'de: 'a"))]
pub struct Views<'a> {
    #[serde(rename = "top-songs", deserialize_with = "loose")]
    top_songs: Relation<'a>,
    #[serde(rename = "full-albums", deserialize_with = "loose")]
    full_albums: Relation<'a>,
    #[serde(rename = "live-albums", deserialize_with = "loose")]
    live_albums: Relation<'a>,
    #[serde(rename = "compilation-albums", deserialize_with = "loose")]
    compilation_albums: Relation<'a>,
    #[serde(rename = "singles", deserialize_with = "loose")]
    singles: Relation<'a>,
    #[serde(rename = "appears-on-albums", deserialize_with = "loose")]
    appears_on_albums: Relation<'a>,
    #[serde(rename = "related-albums", deserialize_with = "loose")]
    related_albums: Relation<'a>,
    #[serde(rename = "similar-artists", deserialize_with = "loose")]
    similar_artists: Relation<'a>,
}

/// The members of an object in the order Apple sent them, each read as `V`.
pub struct Members<'a, V>(Vec<(Cow<'a, str>, V)>);

/// A part of an answer that reads whatever JSON sits at its place. Anything that is not its
/// own shape reads as its default and is skipped without being kept.
pub trait Loose<'de>: Default {
    fn loose<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Loosely(PhantomData))
    }

    /// Reads an object at this place.
    fn object<A: MapAccess<'de>>(mut map: A) -> Result<Self, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Self::default())
    }

    /// Reads an array at this place.
    fn array<A: SeqAccess<'de>>(mut seq: A) -> Result<Self, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Self::default())
    }

    /// Reads a string at this place.
    fn text(_text: Cow<'de, str>) -> Self {
        Self::default()
    }
}

/// The visitor behind [`Loose::loose`]. It hands an object, an array or a string to the reader
/// and makes the default of anything else.
struct Loosely<T>(PhantomData<T>);

/// The visitor behind a [`Leaf`].
struct Seen<'a>(PhantomData<Leaf<'a>>);

/// Reads one element or member through [`Loose`].
struct Seed<T>(PhantomData<T>);

/// Reads each listed shape from an object, and as its default from anything else.
macro_rules! objects {
    ($($shape:ident),* $(,)?) => {$(
        impl<'de: 'a, 'a> Loose<'de> for $shape<'a> {
            fn object<A: MapAccess<'de>>(map: A) -> Result<Self, A::Error> {
                Self::deserialize(MapAccessDeserializer::new(map))
            }
        }
    )*};
}

objects!(
    Outline,
    Meta,
    Search,
    Charts,
    Seeded,
    Station,
    Resource,
    Attributes,
    PlayParams,
    Artwork,
    Editorial,
    Title,
    Loudness,
    Measure,
    Relationships,
    Relation,
    Views,
);

impl<'a> Leaf<'a> {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Leaf::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Leaf::Unsigned(number) => Some(number),
            Leaf::Signed(number) => u64::try_from(number).ok(),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Leaf::Unsigned(number) => i64::try_from(number).ok(),
            Leaf::Signed(number) => Some(number),
            _ => None,
        }
    }

    /// Any number as a float, the way a whole number reads as one too.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Leaf::Unsigned(number) => Some(number as f64),
            Leaf::Signed(number) => Some(number as f64),
            Leaf::Real(number) => Some(number),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Leaf::Flag(flag) => Some(flag),
            _ => None,
        }
    }

    /// Whether Apple sent anything under this name, null included.
    pub fn is_present(&self) -> bool {
        !matches!(self, Leaf::Absent)
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Leaf<'a> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(Seen(PhantomData))
    }
}

impl<'de: 'a, 'a> Loose<'de> for Leaf<'a> {
    fn loose<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::deserialize(deserializer)
    }
}

impl<'de: 'a, 'a> Visitor<'de> for Seen<'a> {
    type Value = Leaf<'a>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, flag: bool) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Flag(flag))
    }

    fn visit_i64<E>(self, number: i64) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Signed(number))
    }

    fn visit_u64<E>(self, number: u64) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Unsigned(number))
    }

    fn visit_f64<E>(self, number: f64) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Real(number))
    }

    fn visit_borrowed_str<E>(self, text: &'de str) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Text(Cow::Borrowed(text)))
    }

    fn visit_str<E>(self, text: &str) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Text(Cow::Owned(text.to_owned())))
    }

    fn visit_string<E>(self, text: String) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Text(Cow::Owned(text)))
    }

    fn visit_unit<E>(self) -> Result<Leaf<'a>, E> {
        Ok(Leaf::Other)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Leaf<'a>, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Leaf::Other)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Leaf<'a>, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Leaf::Other)
    }
}

impl<'de, T: Loose<'de>> Visitor<'de> for Loosely<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<T, E> {
        Ok(T::default())
    }

    fn visit_i64<E>(self, _: i64) -> Result<T, E> {
        Ok(T::default())
    }

    fn visit_u64<E>(self, _: u64) -> Result<T, E> {
        Ok(T::default())
    }

    fn visit_f64<E>(self, _: f64) -> Result<T, E> {
        Ok(T::default())
    }

    fn visit_borrowed_str<E>(self, text: &'de str) -> Result<T, E> {
        Ok(T::text(Cow::Borrowed(text)))
    }

    fn visit_str<E>(self, text: &str) -> Result<T, E> {
        Ok(T::text(Cow::Owned(text.to_owned())))
    }

    fn visit_string<E>(self, text: String) -> Result<T, E> {
        Ok(T::text(Cow::Owned(text)))
    }

    fn visit_unit<E>(self) -> Result<T, E> {
        Ok(T::default())
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        T::object(map)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<T, A::Error> {
        T::array(seq)
    }
}

impl<'de, T: Loose<'de>> DeserializeSeed<'de> for Seed<T> {
    type Value = T;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<T, D::Error> {
        T::loose(deserializer)
    }
}

impl<'de, T: Loose<'de>> Loose<'de> for Vec<T> {
    fn array<A: SeqAccess<'de>>(mut seq: A) -> Result<Self, A::Error> {
        let mut items = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(item) = seq.next_element_seed(Seed(PhantomData))? {
            items.push(item);
        }
        Ok(items)
    }
}

/// Present whenever anything at all was sent, null included, and absent only when the name was
/// missing.
impl<'de, T: Loose<'de>> Loose<'de> for Option<T> {
    fn loose<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        T::loose(deserializer).map(Some)
    }
}

impl<'de> Loose<'de> for IgnoredAny {}

impl Default for Raw<'_> {
    fn default() -> Self {
        Raw(RawValue::NULL)
    }
}

impl<'de: 'a, 'a> Loose<'de> for Raw<'a> {
    fn loose<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <&'a RawValue>::deserialize(deserializer).map(Raw)
    }
}

impl<'de: 'a, 'a, R: Loose<'de>> Loose<'de> for Answer<'a, R> {
    fn object<A: MapAccess<'de>>(map: A) -> Result<Self, A::Error> {
        Self::deserialize(MapAccessDeserializer::new(map))
    }
}

impl<'de: 'a, 'a> Loose<'de> for Notes<'a> {
    fn object<A: MapAccess<'de>>(map: A) -> Result<Self, A::Error> {
        Self::deserialize(MapAccessDeserializer::new(map))
    }

    fn text(text: Cow<'de, str>) -> Self {
        Notes {
            text: Leaf::Text(text),
            ..Notes::default()
        }
    }
}

impl<'a, V> Default for Members<'a, V> {
    fn default() -> Self {
        Members(Vec::new())
    }
}

impl<'a, V> Members<'a, V> {
    /// The member under `name`, the last one when Apple sent the name twice.
    pub fn get(&self, name: &str) -> Option<&V> {
        self.0
            .iter()
            .rev()
            .find(|(key, _)| key.as_ref() == name)
            .map(|(_, value)| value)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().map(|(_, value)| value)
    }
}

impl<'de: 'a, 'a, V: Loose<'de>> Loose<'de> for Members<'a, V> {
    fn object<A: MapAccess<'de>>(mut map: A) -> Result<Self, A::Error> {
        let mut members = Vec::new();
        while let Some(key) = map.next_key::<Leaf<'a>>()? {
            let value = map.next_value_seed(Seed(PhantomData))?;
            if let Leaf::Text(key) = key {
                members.push((key, value));
            }
        }
        Ok(Members(members))
    }
}

impl<'a> Relation<'a> {
    /// The resources of the relationship, none when it has no `data` array.
    pub fn rows(&self) -> &[Resource<'a>] {
        self.data.as_deref().unwrap_or_default()
    }
}

impl<'a> Views<'a> {
    /// The rows of one named view, none for a name not asked for here.
    pub fn get(&self, name: &str) -> &[Resource<'a>] {
        let view = match name {
            "top-songs" => &self.top_songs,
            "full-albums" => &self.full_albums,
            "live-albums" => &self.live_albums,
            "compilation-albums" => &self.compilation_albums,
            "singles" => &self.singles,
            "appears-on-albums" => &self.appears_on_albums,
            "related-albums" => &self.related_albums,
            "similar-artists" => &self.similar_artists,
            _ => return &[],
        };
        view.rows()
    }
}

impl<'a> Outline<'a> {
    /// The rows of the page in order, each parsed only when the iterator reaches it. A row
    /// that is not JSON is skipped.
    pub fn rows(&self) -> impl Iterator<Item = Resource<'a>> + '_ {
        self.data.iter().filter_map(|raw| row(raw.0.get()))
    }

    /// Every row of the page read through `read`, in order. Each row is dropped as soon as
    /// `read` is done with it, so a page never exists as one whole tree.
    pub fn read<T, F>(&self, read: &F) -> Vec<T>
    where
        F: Fn(&Resource<'_>) -> Option<T> + ?Sized,
    {
        self.rows().filter_map(|row| read(&row)).collect()
    }
}

/// An answer body as an [`Answer`]. A body that is empty, is not JSON or is not an object reads
/// as an answer with nothing in it.
pub fn answer<'a, R: Loose<'a>>(body: &'a str) -> Answer<'a, R> {
    whole(body).unwrap_or_default()
}

/// A page body as an [`Outline`], empty when it is not the JSON a listing answers with.
pub fn outline(body: &str) -> Outline<'_> {
    whole(body).unwrap_or_default()
}

/// The catalog resource behind a library row, when it was included.
pub fn catalog<'r, 'a>(value: &'r Resource<'a>) -> Option<&'r Resource<'a>> {
    value.relationships.catalog.rows().first()
}

/// A non-empty string field.
pub fn text(field: &Leaf) -> Option<String> {
    field
        .as_str()
        .filter(|found| !found.is_empty())
        .map(str::to_owned)
}

fn number(field: &Leaf) -> u32 {
    field.as_u64().unwrap_or(0).min(u64::from(u32::MAX)) as u32
}

/// Apple hands out artwork as a template with the size left open. The query string of a
/// generated cover carries a signature, so only the placeholders are touched.
pub fn artwork(attributes: &Attributes, size: u32) -> Option<String> {
    let template = attributes.artwork.url.as_str()?;
    Some(sized(template, size, size))
}

/// The first editorial picture of an artist that Apple sent, at no more than `BANNER` across
/// and its own aspect.
pub fn banner(attributes: &Attributes) -> Option<String> {
    let editorial = &attributes.editorial_artwork;
    [
        &editorial.banner_uber,
        &editorial.super_hero_wide,
        &editorial.centered_fullscreen_background,
    ]
    .into_iter()
    .find_map(|art| {
        let template = art.url.as_str()?;
        let width = art.width.as_u64().filter(|width| *width > 0)?;
        let height = art.height.as_u64()?;
        let across = BANNER.min(width);
        let down = height * across / width;
        Some(sized(template, across as u32, down as u32))
    })
}

/// An ISO-8601 stamp as seconds since the epoch. Apple writes them two ways, `2026-09-15` and
/// `2026-09-15T12:03:39Z`, and only the date part matters for sorting a library.
pub fn moment(stamp: Option<&str>) -> Option<i64> {
    calendar::seconds(stamp?)
}

/// The artists of a resource. Apple only names them in one string unless the relationship was
/// asked for, and without an id an artist is not somewhere the app can go.
pub fn artist_refs(value: &Resource, attributes: &Attributes) -> Vec<ArtistRef> {
    let listed: Vec<ArtistRef> = value
        .relationships
        .artists
        .rows()
        .iter()
        .filter_map(|artist| {
            Some(ArtistRef {
                name: text(&artist.attributes.as_ref()?.name)?,
                id: artist.id.as_str().map(str::to_owned),
            })
        })
        .collect();
    match listed.is_empty() {
        true => vec![ArtistRef {
            name: text(&attributes.artist_name).unwrap_or_default(),
            id: None,
        }],
        false => listed,
    }
}

/// One catalog song.
pub fn song(value: &Resource) -> Option<Track> {
    let id = value.id.as_str()?;
    song_as(value, id.to_owned())
}

/// One row of a playlist: a library song on the listener's own playlists, a catalog song on
/// everyone else's.
pub fn playlist_track(row: &Resource) -> Option<Track> {
    match row.kind.as_str() {
        Some("library-songs") => library_song(row),
        _ => song(row),
    }
}

/// One library song, which plays through the catalog id its play parameters name. A library row
/// without one is an upload, and this path has no way to play it. A row whose catalog
/// relationship was asked for and came back empty names an id Apple has pulled, which web
/// playback refuses. It plays through the reporting id when Apple delivered the song again
/// under a new one, and is listed unplayable when it did not.
pub fn library_song(value: &Resource) -> Option<Track> {
    let attributes = value.attributes.as_ref();
    let params = attributes.and_then(|attributes| attributes.play_params.as_ref());
    let id = params
        .and_then(|params| params.catalog_id.as_str())
        .or_else(|| catalog(value)?.id.as_str())?
        .to_owned();
    // The catalog copy is richer when it came along. Apple puts no date on a library song row,
    // so the date is the library album's when that was included: the day the first song of
    // that album joined the library, which is also the order Apple itself sorts songs by.
    let mut track = match catalog(value) {
        Some(found) => song_as(found, id)?,
        None if value.relationships.catalog.data.is_none() => song_as(value, id)?,
        None => match params
            .and_then(|params| params.reporting_id.as_str())
            .filter(|replacement| *replacement != id)
        {
            Some(replacement) => song_as(value, replacement.to_owned())?,
            None => {
                let mut track = song_as(value, id)?;
                track.playable = false;
                track
            }
        },
    };
    track.added_at = attributes
        .and_then(|attributes| moment(attributes.date_added.as_str()))
        .or_else(|| {
            value
                .relationships
                .albums
                .rows()
                .first()?
                .attributes
                .as_ref()
                .and_then(|attributes| moment(attributes.date_added.as_str()))
        })
        .or(track.added_at);
    Some(track)
}

/// One catalog album.
pub fn album(value: &Resource) -> Option<Album> {
    let id = value.id.as_str()?;
    let attributes = value.attributes.as_ref()?;
    let release = text(&attributes.release_date).unwrap_or_default();
    Some(Album {
        id: id.to_owned(),
        name: text(&attributes.name)?,
        artists: text(&attributes.artist_name).unwrap_or_default(),
        artist_refs: artist_refs(value, attributes),
        cover: artwork(attributes, ART),
        cover_large: artwork(attributes, HERO),
        release_type: release_type(attributes),
        year: release
            .get(..4)
            .and_then(|year| year.parse().ok())
            .unwrap_or(0),
        track_count: number(&attributes.track_count),
        release_date: release,
        label: text(&attributes.record_label).unwrap_or_default(),
        copyrights: text(&attributes.copyright).into_iter().collect(),
        added_at: moment(attributes.date_added.as_str()),
    })
}

/// One library album, which opens through the catalog release it came from.
pub fn library_album(value: &Resource) -> Option<Album> {
    let found = catalog(value)?;
    let id = found.id.as_str()?;
    let mut album = match album(found) {
        Some(album) => album,
        None => Album {
            id: id.to_owned(),
            ..album(value)?
        },
    };
    album.added_at = value
        .attributes
        .as_ref()
        .and_then(|attributes| moment(attributes.date_added.as_str()))
        .or(album.added_at);
    Some(album)
}

/// What Apple calls the release, as far as the model has a name for it.
fn release_type(attributes: &Attributes) -> ReleaseType {
    let single = attributes.is_single.as_bool().unwrap_or(false);
    let compilation = attributes.is_compilation.as_bool().unwrap_or(false);
    let tracks = number(&attributes.track_count);
    match (single, compilation, tracks) {
        (true, _, _) => ReleaseType::Single,
        (_, true, _) => ReleaseType::Compilation,
        (_, _, 2..=6) => ReleaseType::Ep,
        _ => ReleaseType::Album,
    }
}

/// One catalog playlist, which is Apple's or a curator's rather than the listener's.
pub fn playlist(value: &Resource) -> Option<Playlist> {
    let id = value.id.as_str()?;
    let attributes = value.attributes.as_ref()?;
    let curator = text(&attributes.curator_name).unwrap_or_else(|| "Apple Music".to_owned());
    Some(Playlist {
        id: id.to_owned(),
        name: text(&attributes.name)?,
        owner: curator.clone(),
        owner_id: curator,
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover: artwork(attributes, ART),
        track_count: value.relationships.tracks.rows().len() as u32,
        modified_at: moment(attributes.last_modified_date.as_str()),
    })
}

/// One of the listener's own playlists.
pub fn library_playlist(value: &Resource, owner: &str) -> Option<Playlist> {
    let id = value.id.as_str()?;
    let attributes = value.attributes.as_ref()?;
    Some(Playlist {
        id: id.to_owned(),
        name: text(&attributes.name)?,
        owner: owner.to_owned(),
        owner_id: owner.to_owned(),
        owned: attributes.can_edit.as_bool().unwrap_or(false),
        collaborative: attributes.has_collaboration.as_bool().unwrap_or(false),
        blend: false,
        public: attributes.is_public.as_bool().unwrap_or(false),
        cover: artwork(attributes, ART),
        track_count: 0,
        modified_at: moment(attributes.last_modified_date.as_str())
            .or(moment(attributes.date_added.as_str())),
    })
}

/// One library artist. Only the catalog counterpart has a picture, and only its id can open a
/// page, so an artist without one is left out.
pub fn saved_artist(value: &Resource) -> Option<SavedArtist> {
    let found = catalog(value)?;
    let attributes = found.attributes.as_ref()?;
    Some(SavedArtist {
        id: found.id.as_str()?.to_owned(),
        name: text(&attributes.name).or_else(|| text(&value.attributes.as_ref()?.name))?,
        cover: artwork(attributes, ART),
        added_at: value
            .attributes
            .as_ref()
            .and_then(|attributes| moment(attributes.date_added.as_str())),
    })
}

/// A catalog artist page, with whatever its views brought along.
pub fn artist(value: &Resource) -> Option<Artist> {
    let attributes = value.attributes.as_ref()?;
    let notes = &attributes.editorial_notes;
    Some(Artist {
        name: text(&attributes.name)?,
        cover_large: artwork(attributes, HERO),
        banner: banner(attributes),
        biography: text(&notes.text)
            .or_else(|| text(&notes.standard))
            .or_else(|| text(&attributes.artist_bio)),
        monthly_listeners: None,
        top_tracks: view(value, "top-songs").iter().filter_map(song).collect(),
        albums: newest_first(
            DISCOGRAPHY
                .iter()
                .flat_map(|name| view(value, name))
                .filter_map(album)
                .collect(),
        ),
    })
}

/// Orders releases by release date, newest first, keeping the order Apple gave for releases on
/// the same day.
pub fn newest_first(mut albums: Vec<Album>) -> Vec<Album> {
    albums.sort_by(|a, b| b.release_date.cmp(&a.release_date));
    albums
}

pub fn artist_profile(value: &Resource) -> Option<ArtistProfile> {
    let attributes = value.attributes.as_ref()?;
    let notes = &attributes.editorial_notes;
    Some(ArtistProfile {
        name: text(&attributes.name)?,
        cover_large: artwork(attributes, HERO),
        biography: text(&notes.standard)
            .or_else(|| text(&notes.short))
            .or_else(|| text(&attributes.artist_bio)),
    })
}

/// One artist out of a similar-artists view: an id to open and a face for the rail's
/// artists tab.
pub fn similar_artist(value: &Resource) -> Option<SavedArtist> {
    let attributes = value.attributes.as_ref()?;
    Some(SavedArtist {
        id: value.id.as_str()?.to_owned(),
        name: text(&attributes.name)?,
        cover: artwork(attributes, ART),
        added_at: None,
    })
}

/// The rows of one named view of a resource, which is how an artist page carries its songs and
/// releases.
pub fn view<'r, 'a>(value: &'r Resource<'a>, name: &str) -> &'r [Resource<'a>] {
    value.views.get(name)
}

/// The listener's pins in pin order, each with its library id, read from an answer that sent
/// its resources as a map.
///
/// Every pin in `data` is only a reference, looked up in `resources`. Its catalog reference is
/// looked up there too and read in its place, since a library artist has no artwork of its own.
/// A pinned song or video has no sidebar row and is left out.
pub fn pins(answer: &Answer, owner: &str) -> Vec<(String, PinTarget)> {
    let resource = |reference: &Resource| {
        answer
            .resources
            .get(reference.kind.as_str()?)?
            .get(reference.id.as_str()?)
    };
    answer
        .data
        .iter()
        .filter_map(|reference| {
            let pin = resource(reference)?;
            let found = catalog(pin).map(|found| resource(found).unwrap_or(found));
            let mut target = pin_target(pin, found, owner)?;
            target.pinned = true;
            Some((pin.id.as_str()?.to_owned(), target))
        })
        .collect()
}

/// The library id of the artist in a library search answer whose catalog artist is `id`. The
/// rows are read whether the answer lists them inline or sends them as a resource map.
pub fn library_artist(answer: &Answer<Search>, id: &str) -> Option<String> {
    let inline = answer.results.library_artists.rows().iter();
    let mapped = answer
        .resources
        .get("library-artists")
        .into_iter()
        .flat_map(Members::values);
    inline
        .chain(mapped)
        .find(|row| catalog(row).and_then(|found| found.id.as_str()) == Some(id))
        .and_then(|row| row.id.as_str().map(str::to_owned))
}

/// The uri Sonora knows an Apple item by in the sidebar. It is Spotify-shaped on purpose: a
/// sidebar pin is built by taking what follows the last colon.
pub fn pin_uri(kind: PinTargetKind, id: &str) -> Option<String> {
    let part = match kind {
        PinTargetKind::Playlist => "playlist",
        PinTargetKind::Album => "album",
        PinTargetKind::Artist => "artist",
        _ => return None,
    };
    Some(format!("apple:{part}:{id}"))
}

/// One library album, artist or playlist as an unpinned pin target, read with `catalog` as the
/// catalog item behind it. An album or artist needs that item, whose id is the one Sonora opens.
fn pin_target(value: &Resource, catalog: Option<&Resource>, owner: &str) -> Option<PinTarget> {
    let kind = value.kind.as_str()?;
    let attributes = value.attributes.as_ref()?;
    let (kind, id, subtitle) = match kind {
        "library-albums" | "albums" => (
            PinTargetKind::Album,
            catalog.and_then(|found| found.id.as_str()),
            text(&attributes.artist_name).unwrap_or_default(),
        ),
        "library-playlists" | "playlists" => (
            PinTargetKind::Playlist,
            value.id.as_str(),
            text(&attributes.curator_name).unwrap_or_else(|| owner.to_owned()),
        ),
        "library-artists" | "artists" => (
            PinTargetKind::Artist,
            catalog.and_then(|found| found.id.as_str()),
            String::new(),
        ),
        _ => return None,
    };
    Some(PinTarget {
        uri: pin_uri(kind, id?)?,
        name: text(&attributes.name)?,
        subtitle,
        cover: artwork(attributes, ART).or_else(|| artwork(catalog?.attributes.as_ref()?, ART)),
        kind,
        pinned: false,
    })
}

/// One song listed under `id`, which a library row sets to the catalog id it plays through. The
/// row still needs an id of its own, as every song has.
fn song_as(value: &Resource, id: String) -> Option<Track> {
    value.id.as_str()?;
    let attributes = value.attributes.as_ref()?;
    Some(Track {
        id: Some(id),
        name: text(&attributes.name)?,
        // Without play parameters a song is listed but not streamable here.
        playable: attributes.play_params.is_some(),
        artists: text(&attributes.artist_name).unwrap_or_default(),
        artist_refs: artist_refs(value, attributes),
        album: text(&attributes.album_name).unwrap_or_default(),
        album_id: value
            .relationships
            .albums
            .rows()
            .first()
            .and_then(|album| album.id.as_str())
            .map(str::to_owned),
        cover: artwork(attributes, ART),
        duration: Duration::from_millis(attributes.duration_in_millis.as_u64().unwrap_or(0)),
        added_at: moment(attributes.date_added.as_str()),
        added_by: None,
        playcount: None,
        popularity: 0,
        explicit: attributes.content_rating.as_str() == Some("explicit"),
        track_number: number(&attributes.track_number),
        disc_number: number(&attributes.disc_number),
        tags: attributes
            .genre_names
            .iter()
            .filter_map(Leaf::as_str)
            .map(str::to_owned)
            .collect(),
        languages: Vec::new(),
        credits: Vec::new(),
    })
}

/// One row of a listing, or nothing when its text is not JSON.
fn row(text: &str) -> Option<Resource<'_>> {
    whole(text)
}

/// A whole text read as `T`, or nothing when it is not one JSON value with only whitespace
/// around it.
fn whole<'a, T: Loose<'a>>(text: &'a str) -> Option<T> {
    let mut json = serde_json::Deserializer::from_str(text);
    let read = T::loose(&mut json).ok()?;
    json.end().ok()?;
    Some(read)
}

/// Reads a field through [`Loose`], for `deserialize_with`.
fn loose<'de, D: Deserializer<'de>, T: Loose<'de>>(deserializer: D) -> Result<T, D::Error> {
    T::loose(deserializer)
}

/// An artwork template with its placeholders filled in one pass: the sides with `width` and
/// `height`, the format with `jpg` and the crop with nothing.
fn sized(template: &str, width: u32, height: u32) -> String {
    let mut filled = String::with_capacity(template.len() + 8);
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        filled.push_str(&rest[..at]);
        rest = &rest[at..];
        match rest.get(..3) {
            Some("{w}") => {
                let _ = write!(filled, "{width}");
                rest = &rest[3..];
            }
            Some("{h}") => {
                let _ = write!(filled, "{height}");
                rest = &rest[3..];
            }
            Some("{f}") => {
                filled.push_str("jpg");
                rest = &rest[3..];
            }
            Some("{c}") => rest = &rest[3..],
            _ => {
                filled.push('{');
                rest = &rest[1..];
            }
        }
    }
    filled.push_str(rest);
    filled
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn read(value: &Value) -> Resource<'_> {
        Resource::loose(value).unwrap()
    }

    fn catalog_song() -> Value {
        serde_json::json!({
            "id": "1440857781",
            "type": "songs",
            "attributes": {
                "name": "Blue Monday",
                "artistName": "New Order",
                "albumName": "Substance",
                "durationInMillis": 447_000u64,
                "trackNumber": 3,
                "discNumber": 1,
                "contentRating": "explicit",
                "genreNames": ["Alternative", "Music"],
                "playParams": { "id": "1440857781", "kind": "song" },
                "artwork": { "url": "https://is1.mzstatic.com/a/{w}x{h}bb.{f}" }
            },
            "relationships": {
                "artists": { "data": [{ "id": "555", "attributes": { "name": "New Order" } }] },
                "albums": { "data": [{ "id": "888" }] }
            }
        })
    }

    #[test]
    fn reads_a_catalog_song() {
        let track = song(&read(&catalog_song())).unwrap();
        assert_eq!(track.id.as_deref(), Some("1440857781"));
        assert_eq!(track.name, "Blue Monday");
        assert_eq!(track.album_id.as_deref(), Some("888"));
        assert_eq!(track.duration, Duration::from_millis(447_000));
        assert!(track.explicit);
        assert!(track.playable);
        assert_eq!(track.tags, vec!["Alternative", "Music"]);
        assert_eq!(
            track.cover.as_deref(),
            Some("https://is1.mzstatic.com/a/600x600bb.jpg")
        );
        // The relationship is what makes an artist clickable.
        assert_eq!(track.artist_refs[0].id.as_deref(), Some("555"));
    }

    #[test]
    fn a_song_with_no_play_parameters_is_not_playable() {
        let mut row = catalog_song();
        row["attributes"]["playParams"] = Value::Null;
        row["attributes"]
            .as_object_mut()
            .unwrap()
            .remove("playParams");
        assert!(!song(&read(&row)).unwrap().playable);
    }

    #[test]
    fn a_library_song_plays_through_its_catalog_id() {
        let row = serde_json::json!({
            "id": "i.abc123",
            "type": "library-songs",
            "attributes": {
                "name": "Blue Monday",
                "artistName": "New Order",
                "dateAdded": "2025-02-13T10:04:40Z",
                "playParams": { "catalogId": "1440857781", "isLibrary": true }
            }
        });
        let track = library_song(&read(&row)).unwrap();
        assert_eq!(track.id.as_deref(), Some("1440857781"));
        assert_eq!(track.added_at, Some(1_739_441_080));
    }

    #[test]
    fn an_upload_with_no_catalog_id_is_dropped() {
        let row = serde_json::json!({
            "id": "i.abc123",
            "type": "library-songs",
            "attributes": { "name": "Home Recording" }
        });
        assert!(library_song(&read(&row)).is_none());
    }

    /// The library row carries the date, the catalog row everything else.
    #[test]
    fn a_library_album_takes_the_catalog_id_and_keeps_its_date() {
        let row = serde_json::json!({
            "id": "l.xyz",
            "type": "library-albums",
            "attributes": { "name": "Substance", "artistName": "New Order", "dateAdded": "2025-10-14" },
            "relationships": { "catalog": { "data": [{
                "id": "617154241",
                "attributes": { "name": "Substance", "artistName": "New Order", "trackCount": 24, "releaseDate": "1987-08-17" }
            }] } }
        });
        let album = library_album(&read(&row)).unwrap();
        assert_eq!(album.id, "617154241");
        assert_eq!(album.year, 1987);
        assert_eq!(album.track_count, 24);
        assert_eq!(album.added_at, Some(1_760_400_000));
    }

    #[test]
    fn dates_come_back_as_seconds() {
        assert_eq!(moment(Some("1970-01-01")), Some(0));
        assert_eq!(moment(Some("2026-09-15")), Some(1_789_430_400));
        assert_eq!(moment(Some("2026-09-15T12:03:39Z")), Some(1_789_473_819));
        assert_eq!(
            moment(Some("2026-09-15T12:03:39.456Z")),
            Some(1_789_473_819)
        );
        assert_eq!(moment(Some("not a date")), None);
        assert_eq!(moment(None), None);
    }

    #[test]
    fn the_track_count_decides_an_ep_from_an_album() {
        let ep = serde_json::json!({ "trackCount": 4 });
        let album = serde_json::json!({ "trackCount": 12 });
        let single = serde_json::json!({ "trackCount": 1, "isSingle": true });
        let read = |value| Attributes::loose(value).unwrap();
        assert_eq!(release_type(&read(&ep)), ReleaseType::Ep);
        assert_eq!(release_type(&read(&album)), ReleaseType::Album);
        assert_eq!(release_type(&read(&single)), ReleaseType::Single);
    }

    #[test]
    fn a_library_playlist_is_the_listeners_own() {
        let row = serde_json::json!({
            "id": "p.abc",
            "attributes": {
                "name": "Classical",
                "canEdit": true,
                "isPublic": false,
                "lastModifiedDate": "2026-05-12T22:57:25Z"
            }
        });
        let playlist = library_playlist(&read(&row), "Me").unwrap();
        assert_eq!(playlist.id, "p.abc");
        assert!(playlist.owned);
        assert!(!playlist.public);
        assert_eq!(playlist.owner, "Me");
    }

    #[test]
    fn an_artist_page_reads_its_views() {
        let row = serde_json::json!({
            "id": "5468295",
            "attributes": {
                "name": "Daft Punk",
                "artwork": { "url": "https://is1.mzstatic.com/a/{w}x{h}bb.jpg" },
                "editorialNotes": { "standard": "Robots." }
            },
            "views": {
                "top-songs": { "data": [catalog_song()] },
                "full-albums": { "data": [{
                    "id": "617154241",
                    "attributes": { "name": "Random Access Memories", "artistName": "Daft Punk", "trackCount": 13 }
                }] }
            }
        });
        let artist = artist(&read(&row)).unwrap();
        assert_eq!(artist.name, "Daft Punk");
        assert_eq!(artist.biography.as_deref(), Some("Robots."));
        assert_eq!(artist.top_tracks.len(), 1);
        assert_eq!(artist.albums.len(), 1);
        assert_eq!(
            artist.cover_large.as_deref(),
            Some("https://is1.mzstatic.com/a/1200x1200bb.jpg")
        );
    }

    #[test]
    fn a_library_row_becomes_a_landing_item() {
        let row = serde_json::json!({
            "id": "l.xyz",
            "type": "library-albums",
            "attributes": { "name": "Omen", "artistName": "Of Virtue",
                            "artwork": { "url": "https://is1.mzstatic.com/a/{w}x{h}bb.jpg" } },
            "relationships": { "catalog": { "data": [{ "id": "1691419979", "attributes": {} }] } }
        });
        let row = read(&row);
        let item = pin_target(&row, catalog(&row), "Me").unwrap();
        assert_eq!(item.uri, "apple:album:1691419979");
        assert_eq!(item.kind, PinTargetKind::Album);
        assert_eq!(item.subtitle, "Of Virtue");
    }

    #[test]
    fn a_kind_the_model_has_no_place_for_is_dropped() {
        let row = serde_json::json!({
            "type": "library-music-videos",
            "attributes": { "name": "A Video" }
        });
        assert!(pin_target(&read(&row), None, "Me").is_none());
    }
}
