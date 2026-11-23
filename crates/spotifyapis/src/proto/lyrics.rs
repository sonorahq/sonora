//! Spotify's color-lyrics v3 protobuf response.

pub(crate) const ENDPOINT: &str = "/color-lyrics/v3/track";

/// The optional lyric sheet returned for a track.
#[derive(prost::Message)]
pub struct Answer {
    #[prost(message, optional, tag = "1")]
    pub lyrics: Option<Sheet>,
    // #[prost(bool, tag = "2")]
    // pub has_vocal_removal: bool,
}

/// A lyric sheet in recording order.
#[derive(prost::Message)]
pub struct Sheet {
    #[prost(enumeration = "SyncType", tag = "1")]
    pub sync_type: i32,
    #[prost(message, repeated, tag = "2")]
    pub lines: Vec<Line>,
    // #[prost(message, repeated, tag = "3")]
    // pub preview_lines: Vec<Line>,
    // #[prost(message, optional, tag = "4")]
    // pub provider: Option<Provider>,
    // #[prost(message, optional, tag = "5")]
    // pub translation: Option<Translation>,
    // #[prost(message, optional, tag = "6")]
    // pub language: Option<Language>,
    // #[prost(int32, tag = "7")]
    // pub cap_status: i32,
}

// #[derive(prost::Message)]
// pub struct Provider {
//     #[prost(string, tag = "1")]
//     pub name: String,
//     #[prost(string, tag = "2")]
//     pub lyrics_id: String,
//     #[prost(string, tag = "3")]
//     pub display_name: String,
// }
//
// #[derive(prost::Message)]
// pub struct Language {
//     #[prost(string, tag = "1")]
//     pub code: String,
//     #[prost(bool, tag = "2")]
//     pub is_rtl: bool,
// }
//
// #[derive(prost::Message)]
// pub struct Translation {
//     #[prost(message, optional, tag = "1")]
//     pub language: Option<Language>,
//     #[prost(message, repeated, tag = "2")]
//     pub lines: Vec<Line>,
// }

/// The timing precision of the lyric sheet.
#[derive(Debug, PartialEq, prost::Enumeration)]
pub enum SyncType {
    Unsynced = 0,
    LineSynced = 1,
    SyllableSynced = 2,
}

/// A line and any timed segments of its text.
#[derive(prost::Message)]
pub struct Line {
    #[prost(int64, tag = "1")]
    pub start_time_ms: i64,
    #[prost(int64, tag = "2")]
    pub end_time_ms: i64,
    #[prost(string, tag = "3")]
    pub words: String,
    #[prost(message, repeated, tag = "4")]
    pub sync_data: Vec<SyncData>,
}

/// Timing and length of the next segment in the line's text.
#[derive(prost::Message)]
pub struct SyncData {
    #[prost(int64, tag = "1")]
    pub start_time_ms: i64,
    #[prost(int64, tag = "2")]
    pub end_time_ms: i64,
    #[prost(int64, tag = "3")]
    pub num_chars: i64,
}
