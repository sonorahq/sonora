//! SearchView v3 catalogue projections.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;

use super::{component, linux_headers};
use crate::{Session, http};

const ENDPOINT: &str = "/searchview/v3/search";

/// The catalogue type to request from SearchView.
#[derive(Clone, Copy)]
pub enum SearchKind {
    Track,
    Album,
    Artist,
    Playlist,
}

/// Ranked hits from a SearchView main-view request.
#[derive(prost::Message)]
pub struct MainViewResponse {
    #[prost(message, repeated, tag = "1")]
    pub hits: Vec<Entity>,
    // #[prost(message, optional, tag = "2")]
    // pub recommendations: Option<Recommendations>,
    // #[prost(bytes, optional, tag = "3")]
    // pub unknown_bytes_3: Option<Vec<u8>>,
    // #[prost(string, tag = "4")]
    // pub next_page_token: String,
    // #[prost(message, optional, tag = "5")]
    // pub banner_content: Option<BannerContent>,
    // #[prost(message, optional, tag = "10")]
    // pub flat_filters: Option<Filters>,
    // #[prost(message, optional, tag = "12")]
    // pub filter_catalog: Option<FilterCatalog>,
}

/// A search hit with common fields and catalogue projections.
#[derive(prost::Message)]
pub struct Entity {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(string, tag = "2")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub image_uri: String,
    // #[prost(message, optional, tag = "4")]
    // pub artist: Option<Artist>,
    #[prost(message, optional, tag = "5")]
    pub track: Option<Track>,
    #[prost(message, optional, tag = "6")]
    pub album: Option<Album>,
    #[prost(message, optional, tag = "7")]
    pub playlist: Option<Playlist>,
    // #[prost(message, optional, tag = "8")]
    // pub genre: Option<Genre>,
    // #[prost(message, optional, tag = "9")]
    // pub audio_show: Option<AudioShow>,
    // #[prost(message, optional, tag = "10")]
    // pub audio_episode: Option<AudioEpisode>,
    // #[prost(message, optional, tag = "11")]
    // pub profile: Option<Profile>,
    // #[prost(message, optional, tag = "13")]
    // pub audiobook: Option<Audiobook>,
    // #[prost(message, optional, tag = "14")]
    // pub section: Option<Section>,
    // #[prost(message, optional, tag = "16")]
    // pub new_release: Option<HighlightTypeNewRelease>,
    // #[prost(message, optional, tag = "17")]
    // pub author: Option<Author>,
    // #[prost(message, optional, tag = "18")]
    // pub recent_search: Option<HighlightTypeRecentSearch>,
    // #[prost(message, optional, tag = "19")]
    // pub concert: Option<Concert>,
    // #[prost(message, optional, tag = "20")]
    // pub best_match: Option<HighlightTypeBestMatch>,
    // #[prost(message, optional, tag = "21")]
    // pub venue: Option<Venue>,
    // #[prost(message, optional, tag = "22")]
    // pub kallax: Option<Kallax>,
    // #[prost(message, optional, tag = "23")]
    // pub podcast_chapter: Option<PodcastChapter>,
    // #[prost(string, tag = "24")]
    // pub signifier_text: String,
    // #[prost(message, optional, tag = "25")]
    // pub gating_decision: Option<GatingDecision>,
    // #[prost(message, optional, tag = "26")]
    // pub work: Option<Work>,
    // #[prost(message, optional, tag = "2046")]
    // pub autocomplete: Option<AutocompleteQuery>,
    // #[prost(string, tag = "2047")]
    // pub serp_metadata: String,
}

/// Recording details SearchView includes with a track hit.
#[derive(prost::Message)]
pub struct Track {
    // #[prost(bool, tag = "1")]
    // pub explicit: bool,
    // #[prost(bool, tag = "2")]
    // pub windowed: bool,
    #[prost(message, optional, tag = "3")]
    pub track_album: Option<RelatedEntity>,
    #[prost(message, repeated, tag = "4")]
    pub track_artists: Vec<RelatedEntity>,
    // #[prost(bool, tag = "6")]
    // pub mogef19: bool,
    // #[prost(bool, tag = "7")]
    // pub lyrics_match: bool,
    // #[prost(message, optional, tag = "8")]
    // pub on_demand: Option<OnDemand>,
    // #[prost(uint64, optional, tag = "9")]
    // pub unknown_varint_9: Option<u64>,
    // #[prost(bool, tag = "10")]
    // pub premium_only: bool,
    // #[prost(string, tag = "11")]
    // pub alternate_image_uri: String,
    // #[prost(string, tag = "12")]
    // pub release_signifier_text: String,
    // #[prost(string, tag = "13")]
    // pub alternate_uri: String,
    // #[prost(int32, tag = "14")]
    // pub r#type: i32,
    // #[prost(bool, tag = "15")]
    // pub age_assured: bool,
    // #[prost(message, optional, tag = "16")]
    // pub lyrics: Option<Snippet>,
    // #[prost(int32, tag = "17")]
    // pub classification: i32,
    #[prost(message, optional, tag = "18")]
    pub duration: Option<prost_types::Duration>,
    // #[prost(int32, tag = "19")]
    // pub content_access: i32,
    // #[prost(bool, tag = "20")]
    // pub is_chaptered: bool,
}

/// A release card and its artist credits.
#[derive(prost::Message)]
pub struct Album {
    #[prost(string, tag = "1")]
    pub artist_names: String,
    #[prost(enumeration = "AlbumType", tag = "2")]
    pub r#type: i32,
    #[prost(int32, tag = "3")]
    pub release_year: i32,
    // #[prost(int32, tag = "4")]
    // pub state: i32,
    #[prost(message, optional, tag = "5")]
    pub release_time: Option<prost_types::Timestamp>,
    #[prost(string, tag = "6")]
    pub user_country_release_iso_time: String,
    #[prost(message, repeated, tag = "7")]
    pub album_artists: Vec<RelatedEntity>,
    // #[prost(int64, tag = "8")]
    // pub followerscount: i64,
    // #[prost(bool, tag = "9")]
    // pub premium_only: bool,
    // #[prost(bool, tag = "10")]
    // pub explicit: bool,
    // #[prost(string, tag = "11")]
    // pub release_signifier_text: String,
    // #[prost(bool, tag = "12")]
    // pub mogef19: bool,
}

/// SearchView's release classification.
#[derive(Debug, prost::Enumeration)]
pub enum AlbumType {
    Undefined = 0,
    Album = 1,
    Single = 2,
    Compilation = 3,
    Ep = 4,
    Audiobook = 5,
    Podcast = 6,
}

/// A playlist card and its owner label.
#[derive(prost::Message)]
pub struct Playlist {
    // #[prost(bool, tag = "1")]
    // pub personalized: bool,
    // #[prost(bool, tag = "2")]
    // pub owned_by_spotify: bool,
    #[prost(int32, tag = "3")]
    pub number_of_tracks: i32,
    #[prost(int64, tag = "4")]
    pub followers_count: i64,
    #[prost(string, tag = "5")]
    pub owner_name: String,
    // #[prost(string, repeated, tag = "6")]
    // pub tags: Vec<String>,
    // #[prost(bool, tag = "7")]
    // pub mats: bool,
    // #[prost(bool, tag = "8")]
    // pub prompted: bool,
}

/// A named catalogue reference, used for album artists.
#[derive(prost::Message)]
pub struct RelatedEntity {
    #[prost(string, tag = "1")]
    pub uri: String,
    #[prost(string, tag = "2")]
    pub name: String,
}

impl Session {
    /// Searches one catalogue type with its own result limit.
    pub async fn search(
        &self,
        query: &str,
        kind: SearchKind,
        limit: usize,
    ) -> Result<MainViewResponse> {
        let kind = match kind {
            SearchKind::Track => "track",
            SearchKind::Album => "album",
            SearchKind::Artist => "artist",
            SearchKind::Playlist => "playlist",
        };
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let id = (fastrand::u128(..) & !0x00000000_0000_f000_c000_000000000000)
            | 0x00000000_0000_4000_8000_000000000000;
        let request_id = format!(
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            id >> 96,
            (id >> 80) as u16,
            (id >> 64) as u16,
            (id >> 48) as u16,
            id & 0xffff_ffff_ffff,
        );
        let query = component(query);
        let path = format!(
            "{}?query={query}&entity_types={kind}&limit={limit}&timestamp={timestamp}&request_id={request_id}&show_type=podcast",
            ENDPOINT
        );
        http::get(self, &path, linux_headers()).await
    }
}
