//! Extended metadata requests, responses, and catalogue payloads.

use std::collections::HashMap;

use anyhow::Result;
use bytes::Bytes;

use crate::{Session, http};

pub(crate) const ENDPOINT: &str = "/extended-metadata/v0/extended-metadata";

#[derive(prost::Message)]
pub struct Artist {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub gid: Option<Bytes>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    // #[prost(sint32, optional, tag = "3")]
    // pub popularity: Option<i32>,
    #[prost(message, repeated, tag = "4")]
    pub top_tracks: Vec<TopTracks>,
    #[prost(message, repeated, tag = "5")]
    pub album_group: Vec<AlbumGroup>,
    #[prost(message, repeated, tag = "6")]
    pub single_group: Vec<AlbumGroup>,
    #[prost(message, repeated, tag = "7")]
    pub compilation_group: Vec<AlbumGroup>,
    #[prost(message, repeated, tag = "8")]
    pub appears_on_group: Vec<AlbumGroup>,
    // #[prost(string, repeated, tag = "9")]
    // pub genre: Vec<String>,
    // #[prost(message, repeated, tag = "10")]
    // pub external_id: Vec<ExternalId>,
    #[prost(message, repeated, tag = "11")]
    pub portrait: Vec<Image>,
    #[prost(message, repeated, tag = "12")]
    pub biography: Vec<Biography>,
    // #[prost(message, repeated, tag = "13")]
    // pub activity_period: Vec<ActivityPeriod>,
    // #[prost(message, repeated, tag = "14")]
    // pub restriction: Vec<Restriction>,
    // #[prost(message, repeated, tag = "15")]
    // pub related: Vec<Artist>,
    // #[prost(bool, optional, tag = "16")]
    // pub is_portrait_album_cover: Option<bool>,
    #[prost(message, optional, tag = "17")]
    pub portrait_group: Option<ImageGroup>,
    // #[prost(message, repeated, tag = "18")]
    // pub sale_period: Vec<SalePeriod>,
    // #[prost(message, repeated, tag = "19")]
    // pub localized_name: Vec<LocalizedName>,
    // #[prost(message, repeated, tag = "20")]
    // pub availability: Vec<Availability>,
    // #[prost(int64, optional, tag = "21")]
    // pub index_version: Option<i64>,
}

/// Artist tracks in Spotify's ranking order for a market.
#[derive(prost::Message)]
pub struct TopTracks {
    // #[prost(string, optional, tag = "1")]
    // pub country: Option<String>,
    #[prost(message, repeated, tag = "2")]
    pub tracks: Vec<TrackReference>,
}

#[derive(prost::Message)]
pub struct Album {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub gid: Option<Bytes>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    #[prost(message, repeated, tag = "3")]
    pub artist: Vec<Artist>,
    #[prost(enumeration = "AlbumType", optional, tag = "4")]
    pub album_type: Option<i32>,
    #[prost(string, optional, tag = "5")]
    pub label: Option<String>,
    #[prost(message, optional, tag = "6")]
    pub date: Option<Date>,
    // #[prost(sint32, optional, tag = "7")]
    // pub popularity: Option<i32>,
    // #[prost(string, repeated, tag = "8")]
    // pub genre: Vec<String>,
    #[prost(message, repeated, tag = "9")]
    pub cover: Vec<Image>,
    // #[prost(message, repeated, tag = "10")]
    // pub external_id: Vec<ExternalId>,
    #[prost(message, repeated, tag = "11")]
    pub disc: Vec<Disc>,
    // #[prost(string, repeated, tag = "12")]
    // pub review: Vec<String>,
    #[prost(message, repeated, tag = "13")]
    pub copyright: Vec<Copyright>,
    // #[prost(message, repeated, tag = "14")]
    // pub restriction: Vec<Restriction>,
    // #[prost(message, repeated, tag = "15")]
    // pub related: Vec<Album>,
    // #[prost(message, repeated, tag = "16")]
    // pub sale_period: Vec<SalePeriod>,
    #[prost(message, optional, tag = "17")]
    pub cover_group: Option<ImageGroup>,
    // #[prost(string, optional, tag = "18")]
    // pub original_title: Option<String>,
    // #[prost(string, optional, tag = "19")]
    // pub version_title: Option<String>,
    // #[prost(string, optional, tag = "20")]
    // pub type_str: Option<String>,
    // #[prost(message, repeated, tag = "21")]
    // pub visibility_block: Vec<VisibilityBlock>,
    // #[prost(int64, optional, tag = "22")]
    // pub earliest_live_timestamp: Option<i64>,
    // #[prost(message, repeated, tag = "23")]
    // pub availability: Vec<Availability>,
    // #[prost(message, repeated, tag = "24")]
    // pub windowed_track: Vec<WindowedTrack>,
    // #[prost(message, optional, tag = "25")]
    // pub licensor: Option<Licensor>,
    // #[prost(int64, optional, tag = "26")]
    // pub version: Option<i64>,
    // #[prost(string, optional, tag = "27")]
    // pub feed_gid: Option<String>,
    // #[prost(string, optional, tag = "28")]
    // pub delivery_id: Option<String>,
    // #[prost(message, repeated, tag = "29")]
    // pub localized_name: Vec<LocalizedName>,
    // #[prost(int64, optional, tag = "30")]
    // pub index_version: Option<i64>,
    // #[prost(int64, optional, tag = "42")]
    // pub prerelease_end_date: Option<i64>,
    // #[prost(message, optional, tag = "44")]
    // pub content_access_status: Option<ContentAccessStatus>,
}

#[derive(Debug, prost::Enumeration)]
pub enum AlbumType {
    Album = 1,
    Single = 2,
    Compilation = 3,
    Ep = 4,
    Audiobook = 5,
    Podcast = 6,
}

#[derive(prost::Message)]
pub struct Track {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub gid: Option<Bytes>,
    #[prost(string, optional, tag = "2")]
    pub name: Option<String>,
    #[prost(message, optional, tag = "3")]
    pub album: Option<Album>,
    #[prost(message, repeated, tag = "4")]
    pub artist: Vec<Artist>,
    #[prost(sint32, tag = "5")]
    pub track_number: i32,
    #[prost(sint32, tag = "6")]
    pub disc_number: i32,
    #[prost(sint32, optional, tag = "7")]
    pub duration_ms: Option<i32>,
    #[prost(sint32, tag = "8")]
    pub popularity: i32,
    #[prost(bool, optional, tag = "9")]
    pub explicit: Option<bool>,
    #[prost(message, repeated, tag = "10")]
    pub external_id: Vec<ExternalId>,
    // #[prost(message, repeated, tag = "11")]
    // pub restriction: Vec<Restriction>,
    #[prost(message, repeated, tag = "12")]
    pub file: Vec<AudioFile>,
    #[prost(message, repeated, tag = "13")]
    pub alternative: Vec<TrackReference>,
    // #[prost(message, repeated, tag = "14")]
    // pub sale_period: Vec<SalePeriod>,
    // #[prost(message, repeated, tag = "15")]
    // pub preview: Vec<AudioFile>,
    // #[prost(string, repeated, tag = "16")]
    // pub tags: Vec<String>,
    // #[prost(int64, optional, tag = "17")]
    // pub earliest_live_timestamp: Option<i64>,
    // #[prost(bool, optional, tag = "18")]
    // pub has_lyrics: Option<bool>,
    // #[prost(message, repeated, tag = "19")]
    // pub availability: Vec<Availability>,
    // #[prost(string, repeated, tag = "20")]
    // pub lyrics_country: Vec<String>,
    // #[prost(message, optional, tag = "21")]
    // pub licensor: Option<Licensor>,
    #[prost(string, repeated, tag = "22")]
    pub language_of_performance: Vec<String>,
    // #[prost(message, repeated, tag = "23")]
    // pub localized_name: Vec<LocalizedName>,
    // #[prost(message, optional, tag = "24")]
    // pub original_audio: Option<Audio>,
    // #[prost(message, repeated, tag = "25")]
    // pub content_rating: Vec<ContentRating>,
    // #[prost(int64, optional, tag = "26")]
    // pub index_version: Option<i64>,
    // #[prost(string, optional, tag = "27")]
    // pub original_title: Option<String>,
    // #[prost(string, optional, tag = "28")]
    // pub version_title: Option<String>,
    #[prost(message, repeated, tag = "32")]
    pub artist_with_role: Vec<ArtistWithRole>,
    // #[prost(string, optional, tag = "36")]
    // pub canonical_uri: Option<String>,
    // TrackV4 responses contain tags 37, 39, 43, and 44, but the local APK
    // descriptors give them no names. Tags 40, 42, and 45–50 are unverified.
    // Observed a nested message at inner tag 1 with varints at tags 1–4.
    // #[prost(bytes, optional, tag = "37")]
    // pub field_37: Option<Vec<u8>>,
    // #[prost(message, repeated, tag = "38")]
    // pub original_video: Vec<Video>,
    // Observed a nested message at inner tag 1 with a varint at tag 3 and length-delimited data at tag 10.
    // #[prost(bytes, optional, tag = "39")]
    // pub field_39: Option<Vec<u8>>,
    // #[prost(message, repeated, tag = "41")]
    // pub audio_formats: Vec<TrackAudioFormat>,
    // Observed varint values 0 and 1. The semantic type is unverified.
    // #[prost(uint64, optional, tag = "43")]
    // pub field_43: Option<u64>,
    // Observed a nested message at inner tag 2 with varints at tags 1 and 2.
    // #[prost(bytes, optional, tag = "44")]
    // pub field_44: Option<Vec<u8>>,
    // #[prost(message, optional, tag = "51")]
    // pub content_access_status: Option<ContentAccessStatus>,
}

/// The GID from a nested Track whose full metadata is fetched separately.
#[derive(prost::Message)]
pub struct TrackReference {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub gid: Option<Bytes>,
}

/// An identifier assigned by a catalogue outside Spotify.
#[derive(prost::Message)]
pub struct ExternalId {
    #[prost(string, optional, tag = "1")]
    pub kind: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub id: Option<String>,
}

#[derive(prost::Message)]
pub struct ArtistWithRole {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub artist_gid: Option<Bytes>,
    #[prost(string, optional, tag = "2")]
    pub artist_name: Option<String>,
    #[prost(int32, optional, tag = "3")]
    pub role: Option<i32>,
    // #[prost(message, repeated, tag = "4")]
    // pub localized_name: Vec<LocalizedName>,
}

#[derive(prost::Message)]
pub struct AlbumGroup {
    #[prost(message, repeated, tag = "1")]
    pub album: Vec<Album>,
}

#[derive(prost::Message)]
pub struct Date {
    #[prost(sint32, optional, tag = "1")]
    pub year: Option<i32>,
    #[prost(sint32, optional, tag = "2")]
    pub month: Option<i32>,
    #[prost(sint32, optional, tag = "3")]
    pub day: Option<i32>,
    // #[prost(sint32, optional, tag = "4")]
    // pub hour: Option<i32>,
    // #[prost(sint32, optional, tag = "5")]
    // pub minute: Option<i32>,
}

#[derive(prost::Message)]
pub struct Image {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub file_id: Option<Bytes>,
    #[prost(enumeration = "ImageSize", optional, tag = "2")]
    pub size: Option<i32>,
    #[prost(sint32, optional, tag = "3")]
    pub width: Option<i32>,
    // #[prost(sint32, optional, tag = "4")]
    // pub height: Option<i32>,
}

#[derive(Debug, prost::Enumeration)]
pub enum ImageSize {
    Default = 0,
    Small = 1,
    Large = 2,
    Xlarge = 3,
}

#[derive(prost::Message)]
pub struct ImageGroup {
    #[prost(message, repeated, tag = "1")]
    pub image: Vec<Image>,
}

#[derive(prost::Message)]
pub struct Biography {
    #[prost(string, optional, tag = "1")]
    pub text: Option<String>,
    // #[prost(message, repeated, tag = "2")]
    // pub portrait: Vec<Image>,
    // #[prost(message, repeated, tag = "3")]
    // pub portrait_group: Vec<ImageGroup>,
}

#[derive(prost::Message)]
pub struct Disc {
    // #[prost(sint32, optional, tag = "1")]
    // pub number: Option<i32>,
    // #[prost(string, optional, tag = "2")]
    // pub name: Option<String>,
    #[prost(message, repeated, tag = "3")]
    pub track: Vec<TrackReference>,
}

#[derive(prost::Message)]
pub struct Copyright {
    #[prost(enumeration = "CopyrightType", optional, tag = "1")]
    pub r#type: Option<i32>,
    #[prost(string, optional, tag = "2")]
    pub text: Option<String>,
}

#[derive(Debug, PartialEq, prost::Enumeration)]
pub enum CopyrightType {
    P = 0,
    C = 1,
}

#[derive(prost::Message)]
pub struct AudioFile {
    #[prost(bytes = "bytes", optional, tag = "1")]
    pub file_id: Option<Bytes>,
    #[prost(int32, optional, tag = "2")]
    pub format: Option<i32>,
}

/// Display fields from Spotify's compact identity profile.
#[derive(prost::Message)]
pub struct IdentityProfile {
    #[prost(message, optional, tag = "2")]
    pub name: Option<IdentityName>,
    #[prost(message, repeated, tag = "3")]
    pub images: Vec<IdentityImage>,
}

/// A wrapped display name.
#[derive(prost::Message)]
pub struct IdentityName {
    #[prost(string, tag = "1")]
    pub value: String,
}

/// One size of a profile image.
#[derive(prost::Message)]
pub struct IdentityImage {
    #[prost(int32, tag = "1")]
    pub width: i32,
    #[prost(string, tag = "3")]
    pub url: String,
}

/// Counts reported for tracks and artists by the platform reputation extension.
#[derive(prost::Message)]
pub struct Reputation {
    #[prost(int64, optional, tag = "1")]
    pub total_established_user_relationships: Option<i64>,
    // #[prost(bool, optional, tag = "2")]
    // pub is_verified: Option<bool>,
    #[prost(int64, optional, tag = "3")]
    pub total_plays: Option<i64>,
    #[prost(int64, optional, tag = "4")]
    pub monthly_listeners: Option<i64>,
    // #[prost(message, optional, tag = "7")]
    // pub chart_position: Option<ChartPosition>,
    // #[prost(message, optional, tag = "8")]
    // pub rating: Option<Rating>,
    // #[prost(message, optional, tag = "9")]
    // pub verification: Option<Verification>,
}

/// Catalogue artwork in the aspect ratios supplied by the visual identity extension.
#[derive(prost::Message)]
pub struct VisualIdentity {
    // #[prost(message, optional, tag = "1")]
    // pub square_cover_image: Option<VisualImage>,
    // #[prost(message, optional, tag = "2")]
    // pub sixteen_by_nine_cover_image: Option<VisualImage>,
    // #[prost(message, optional, tag = "3")]
    // pub nine_by_sixteen_cover_image: Option<VisualImage>,
    // #[prost(message, optional, tag = "4")]
    // pub nine_by_sixteen_cover_video: Option<VisualVideo>,
    // #[prost(message, optional, tag = "5")]
    // pub square_full_bleed_image: Option<VisualImage>,
    #[prost(message, optional, tag = "6")]
    pub wide_full_bleed_image: Option<VisualImage>,
    // #[prost(message, optional, tag = "7")]
    // pub curated_color: Option<CuratedColor>,
}

/// Available sizes and extracted colours for one image.
#[derive(prost::Message)]
pub struct VisualImage {
    #[prost(message, repeated, tag = "1")]
    pub instances: Vec<VisualImageInstance>,
    // #[prost(message, optional, tag = "2")]
    // pub extracted_color_set: Option<ColorSetContrastOptions>,
    // #[prost(int32, optional, tag = "3")]
    // pub mask: Option<i32>,
}

#[derive(prost::Message)]
pub struct VisualImageInstance {
    #[prost(message, optional, tag = "1")]
    pub flat_file: Option<FlatFile>,
    #[prost(int32, tag = "2")]
    pub size: i32,
}

/// An asset available as one CDN object.
#[derive(prost::Message)]
pub struct FlatFile {
    #[prost(string, tag = "1")]
    pub cdn_url: String,
}

/// Spotify's catalogue descriptors for a track.
#[derive(prost::Message)]
pub struct TrackDescriptors {
    #[prost(message, repeated, tag = "1")]
    pub descriptors: Vec<TrackDescriptor>,
}

/// A descriptor label and its categories, including genre (1).
#[derive(prost::Message)]
pub struct TrackDescriptor {
    #[prost(string, tag = "1")]
    pub text: String,
    // #[prost(float, tag = "2")]
    // pub weight: f32,
    #[prost(int32, repeated, tag = "3")]
    pub categories: Vec<i32>,
    // #[prost(string, tag = "4")]
    // pub concept_uri: String,
    #[prost(string, tag = "5")]
    pub localized_term: String,
}

/// Track tempo and musical key from Spotify's audio attributes extension.
#[derive(prost::Message)]
pub struct AudioAttributes {
    #[prost(double, tag = "1")]
    pub bpm: f64,
    // #[prost(bytes = "bytes", optional, tag = "2")]
    // pub key: Option<Bytes>,
}

/// A ranked list of entities returned by the related releases extension.
#[derive(prost::Message)]
pub struct RelatedEntities {
    #[prost(message, repeated, tag = "1")]
    pub entities: Vec<RelatedEntity>,
}

/// A reference to a related catalogue entity.
#[derive(prost::Message)]
pub struct RelatedEntity {
    #[prost(string, tag = "1")]
    pub uri: String,
}

/// Sections of related creators returned for an artist.
#[derive(prost::Message)]
pub struct RelatedCreators {
    #[prost(message, repeated, tag = "1")]
    pub sections: Vec<RelatedEntities>,
}

/// Selects a metadata extension to fetch for an entity.
#[derive(prost::Message)]
struct ExtensionQuery {
    #[prost(enumeration = "ExtensionKind", tag = "1")]
    pub extension_kind: i32,
    // #[prost(string, tag = "2")]
    // pub etag: String,
}

/// Metadata extensions requested for one Spotify URI.
#[derive(prost::Message)]
struct EntityRequest {
    #[prost(string, tag = "1")]
    pub entity_uri: String,
    #[prost(message, repeated, tag = "2")]
    pub query: Vec<ExtensionQuery>,
}

/// Request for metadata on multiple Spotify entities.
#[derive(prost::Message)]
struct BatchedEntityRequest {
    // #[prost(message, optional, tag = "1")]
    // pub header: Option<BatchedEntityRequestHeader>,
    #[prost(message, repeated, tag = "2")]
    pub entity_request: Vec<EntityRequest>,
}

/// Results grouped by metadata extension kind.
#[derive(prost::Message)]
struct EntityExtensionDataArray {
    // #[prost(message, optional, tag = "1")]
    // pub header: Option<EntityExtensionDataArrayHeader>,
    #[prost(enumeration = "ExtensionKind", tag = "2")]
    pub extension_kind: i32,
    #[prost(message, repeated, tag = "3")]
    pub extension_data: Vec<EntityExtensionData>,
}

/// Response containing the groups returned for an extended metadata request.
#[derive(prost::Message)]
struct BatchedExtensionResponse {
    // #[prost(message, optional, tag = "1")]
    // pub header: Option<BatchedExtensionResponseHeader>,
    #[prost(message, repeated, tag = "2")]
    pub extended_metadata: Vec<EntityExtensionDataArray>,
}

/// One entity's payload for the requested extension.
#[derive(prost::Message)]
struct EntityExtensionData {
    // #[prost(message, optional, tag = "1")]
    // pub header: Option<EntityExtensionDataHeader>,
    #[prost(string, tag = "2")]
    entity_uri: String,
    #[prost(message, optional, tag = "3")]
    extension_data: Option<Payload>,
}

// /// Status reported for an individual metadata result.
// #[derive(prost::Message)]
// pub struct EntityExtensionDataHeader {
//     #[prost(int32, tag = "1")]
//     pub status_code: i32,
//     #[prost(string, tag = "2")]
//     pub etag: String,
//     #[prost(string, tag = "3")]
//     pub locale: String,
//     #[prost(int64, tag = "4")]
//     pub cache_ttl_in_seconds: i64,
//     // #[prost(int64, tag = "5")]
//     // pub offline_ttl_in_seconds: i64,
// }

/// A protobuf message that can be decoded from an extended metadata payload.
pub trait MetadataMessage: prost::Message + Default {}

impl<T: prost::Message + Default> MetadataMessage for T {}

/// Encoded catalogue metadata that can be decoded on demand.
#[derive(Clone)]
pub struct MetadataPayload(Bytes);

impl MetadataPayload {
    /// Restores the encoded payload from caller-owned storage.
    pub fn from_bytes(bytes: Bytes) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Decodes the message that corresponds to the enclosing extension kind.
    pub fn decode<M: MetadataMessage>(&self) -> std::result::Result<M, prost::DecodeError> {
        M::decode(self.0.clone())
    }
}

/// Catalog bytes held in the value field of a protobuf Any.
#[derive(prost::Message)]
struct Payload {
    // #[prost(string, tag = "1")]
    // pub type_url: String,
    #[prost(bytes = "bytes", tag = "2")]
    value: Bytes,
}

/// Spotify metadata extension IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
pub enum ExtensionKind {
    Unknown = 0,
    TrackDescriptor = 6,
    ArtistV4 = 8,
    AlbumV4 = 9,
    TrackV4 = 10,
    UserProfile = 15,
    RelatedReleases = 153,
    RelatedCreatorsSection = 165,
    VisualIdentity = 179,
    OnPlatformReputation = 185,
    ListMetadataV2 = 205,
    AudioAttributesV2 = 222,
    SearchGenreMetadata = 280,
}

// #[derive(prost::Message)]
// pub struct TopTracks {
//     #[prost(string, optional, tag = "1")]
//     pub country: Option<String>,
//     #[prost(message, repeated, tag = "2")]
//     pub track: Vec<Track>,
// }
//
// #[derive(prost::Message)]
// pub struct ActivityPeriod {
//     #[prost(sint32, optional, tag = "1")]
//     pub start_year: Option<i32>,
//     #[prost(sint32, optional, tag = "2")]
//     pub end_year: Option<i32>,
//     #[prost(sint32, optional, tag = "3")]
//     pub decade: Option<i32>,
// }
//
// #[derive(prost::Message)]
// pub struct Restriction {
//     #[prost(int32, repeated, packed = "false", tag = "1")]
//     pub catalogue: Vec<i32>,
//     #[prost(oneof = "CountryRestriction", tags = "2, 3")]
//     pub country_restriction: Option<CountryRestriction>,
//     #[prost(int32, optional, tag = "4")]
//     pub r#type: Option<i32>,
//     #[prost(string, repeated, tag = "5")]
//     pub catalogue_str: Vec<String>,
// }
//
// #[derive(prost::Oneof)]
// pub enum CountryRestriction {
//     #[prost(string, tag = "2")]
//     CountriesAllowed(String),
//     #[prost(string, tag = "3")]
//     CountriesForbidden(String),
// }
//
// #[derive(prost::Message)]
// pub struct SalePeriod {
//     #[prost(message, repeated, tag = "1")]
//     pub restriction: Vec<Restriction>,
//     #[prost(message, optional, tag = "2")]
//     pub start: Option<Date>,
//     #[prost(message, optional, tag = "3")]
//     pub end: Option<Date>,
// }
//
// #[derive(prost::Message)]
// pub struct Availability {
//     #[prost(string, repeated, tag = "1")]
//     pub catalogue_str: Vec<String>,
//     #[prost(message, optional, tag = "2")]
//     pub start: Option<Date>,
// }
//
// #[derive(prost::Message)]
// pub struct Licensor {
//     #[prost(bytes, optional, tag = "1")]
//     pub uuid: Option<Vec<u8>>,
// }
//
// #[derive(prost::Message)]
// pub struct Audio {
//     #[prost(bytes, optional, tag = "1")]
//     pub uuid: Option<Vec<u8>>,
//     #[prost(int32, optional, tag = "2")]
//     pub format: Option<i32>,
// }
//
// #[derive(prost::Message)]
// pub struct Video {
//     #[prost(bytes = "bytes", optional, tag = "1")]
//     pub gid: Option<Bytes>,
// }
//
// #[derive(prost::Message)]
// pub struct ContentRating {
//     #[prost(string, optional, tag = "1")]
//     pub country: Option<String>,
//     #[prost(string, repeated, tag = "2")]
//     pub tag: Vec<String>,
// }
//
// #[derive(prost::Message)]
// pub struct TrackAudioFormat {
//     #[prost(message, optional, tag = "1")]
//     pub original_audio: Option<Audio>,
// }

#[derive(prost::Message)]
pub struct SearchGenreMetadata {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub image_url: String,
}

/// Identifies an extension result by kind and Spotify URI.
pub type MetadataKey = (ExtensionKind, String);

impl Session {
    /// Loads each requested URI and extension pair once and indexes its payload.
    pub async fn extended_metadata<'a>(
        &self,
        queries: impl IntoIterator<Item = (&'a str, ExtensionKind)>,
    ) -> Result<HashMap<MetadataKey, MetadataPayload>> {
        let mut positions = HashMap::new();
        let mut request = BatchedEntityRequest::default();
        for (uri, kind) in queries {
            let index = *positions.entry(uri).or_insert_with(|| {
                request.entity_request.push(EntityRequest {
                    entity_uri: uri.to_owned(),
                    query: Vec::new(),
                });
                request.entity_request.len() - 1
            });
            let query = &mut request.entity_request[index].query;
            if !query.iter().any(|item| item.extension_kind == kind as i32) {
                query.push(ExtensionQuery {
                    extension_kind: kind as i32,
                });
            }
        }
        if positions.is_empty() {
            return Ok(HashMap::new());
        }

        let response: BatchedExtensionResponse = http::read_rpc(self, ENDPOINT, &request).await?;
        let mut indexed = HashMap::new();
        for group in response.extended_metadata {
            let Ok(kind) = ExtensionKind::try_from(group.extension_kind) else {
                continue;
            };
            for entry in group.extension_data {
                let Some(payload) = entry.extension_data else {
                    continue;
                };
                let Some(&index) = positions.get(entry.entity_uri.as_str()) else {
                    continue;
                };
                let query = &mut request.entity_request[index].query;
                if let Some(slot) = query
                    .iter()
                    .position(|item| item.extension_kind == group.extension_kind)
                {
                    query.swap_remove(slot);
                    indexed.insert((kind, entry.entity_uri), MetadataPayload(payload.value));
                }
            }
        }
        Ok(indexed)
    }
}
