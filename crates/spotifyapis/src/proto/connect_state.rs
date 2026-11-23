//! Connect device state, playback contexts, and transfers.

use std::collections::HashMap;

use anyhow::Result;
use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};
use bytes::Bytes;
use prost::Message as _;
use reqwest::{
    Method,
    header::{self, HeaderValue},
};
use serde::{Deserialize, Deserializer};
use serde_json::json;

use super::{between_devices, component, dealer_headers, null_default};
use crate::{Session, http};

const VOLUME: &str = "/connect-state/v1/connect/volume";
const TRANSFER: &str = "/connect-state/v1/connect/transfer";
pub const CLUSTER_TOPIC: &str = "hm://connect-state/v1/cluster";
pub const VOLUME_TOPIC: &str = "hm://connect-state/v1/connect/volume";
const DEVICES: &str = "/connect-state/v1/devices";

#[derive(prost::Message)]
pub struct ClusterUpdate {
    #[prost(message, optional, tag = "1")]
    pub cluster: Option<Cluster>,
    // #[prost(int32, tag = "2")]
    // pub update_reason: i32,
    // #[prost(string, tag = "3")]
    // pub ack_id: String,
    // #[prost(string, repeated, tag = "4")]
    // pub devices_that_changed: Vec<String>,
}

#[derive(prost::Message)]
pub struct Device {
    #[prost(message, optional, tag = "1")]
    pub device_info: Option<DeviceInfo>,
    #[prost(message, optional, tag = "2")]
    pub player_state: Option<PlayerState>,
    // #[prost(message, optional, tag = "3")]
    // pub private_device_info: Option<PrivateDeviceInfo>,
    // #[prost(bytes, tag = "4")]
    // pub transfer_data: Vec<u8>,
}

#[derive(prost::Message)]
pub struct Cluster {
    // #[prost(int64, tag = "1")]
    // pub changed_timestamp_ms: i64,
    #[prost(string, tag = "2")]
    pub active_device_id: String,
    #[prost(message, optional, tag = "3")]
    pub player_state: Option<PlayerState>,
    #[prost(map = "string, message", tag = "4")]
    pub device: HashMap<String, DeviceInfo>,
    // #[prost(bytes, tag = "5")]
    // pub transfer_data: Vec<u8>,
    // #[prost(uint64, tag = "6")]
    // pub transfer_data_timestamp: u64,
    // #[prost(bool, tag = "8")]
    // pub need_full_player_state: bool,
    #[prost(int64, tag = "9")]
    pub server_timestamp_ms: i64,
    // #[prost(bool, optional, tag = "10")]
    // pub needs_state_updates: Option<bool>,
    // #[prost(uint64, optional, tag = "11")]
    // pub started_playing_at_timestamp: Option<u64>,
    // #[prost(bool, optional, tag = "13")]
    // pub needs_local_devices: Option<bool>,
}

#[derive(prost::Message)]
pub struct PutStateRequest {
    // #[prost(string, tag = "1")]
    // pub callback_url: String,
    #[prost(message, optional, tag = "2")]
    pub device: Option<Device>,
    #[prost(int32, tag = "3")]
    pub member_type: i32,
    #[prost(bool, tag = "4")]
    pub is_active: bool,
    #[prost(enumeration = "PutStateReason", tag = "5")]
    pub put_state_reason: i32,
    #[prost(uint32, tag = "6")]
    pub message_id: u32,
    #[prost(string, tag = "7")]
    pub last_command_sent_by_device_id: String,
    #[prost(uint32, tag = "8")]
    pub last_command_message_id: u32,
    #[prost(uint64, tag = "9")]
    pub started_playing_at: u64,
    // #[prost(uint64, tag = "11")]
    // pub has_been_playing_for_ms: u64,
    #[prost(uint64, tag = "12")]
    pub client_side_timestamp_ms: u64,
    // #[prost(bool, tag = "13")]
    // pub only_write_player_state: bool,
}

#[derive(PartialEq, prost::Message)]
pub struct DeviceInfo {
    #[prost(bool, tag = "1")]
    pub can_play: bool,
    #[prost(uint32, tag = "2")]
    pub volume: u32,
    #[prost(string, tag = "3")]
    pub name: String,
    #[prost(message, optional, tag = "4")]
    pub capabilities: Option<Capabilities>,
    #[prost(string, tag = "6")]
    pub device_software_version: String,
    #[prost(enumeration = "DeviceType", tag = "7")]
    pub device_type: i32,
    #[prost(string, tag = "9")]
    pub spirc_version: String,
    #[prost(string, tag = "10")]
    pub device_id: String,
    // #[prost(bool, tag = "11")]
    // pub is_private_session: bool,
    // #[prost(bool, tag = "12")]
    // pub is_social_connect: bool,
    #[prost(string, tag = "13")]
    pub client_id: String,
    #[prost(string, tag = "14")]
    pub brand: String,
    #[prost(string, tag = "15")]
    pub model: String,
    // #[prost(map = "string, string", tag = "16")]
    // pub metadata_map: std::collections::HashMap<String, String>,
    // #[prost(string, tag = "17")]
    // pub product_id: String,
    // #[prost(string, tag = "18")]
    // pub deduplication_id: String,
    // #[prost(uint32, tag = "19")]
    // pub selected_alias_id: u32,
    // #[prost(map = "string, message", tag = "20")]
    // pub device_aliases: HashMap<String, DeviceAlias>,
    // #[prost(bool, tag = "21")]
    // pub is_offline: bool,
    // #[prost(string, tag = "22")]
    // pub public_ip: String,
    #[prost(string, tag = "23")]
    pub license: String,
    // #[prost(message, optional, tag = "24")]
    // pub audio_output_device_info: Option<AudioOutputDeviceInfo>,
    // #[prost(bool, tag = "25")]
    // pub is_group: bool,
    // #[prost(bool, tag = "26")]
    // pub is_dynamic_device: bool,
    // #[prost(string, repeated, tag = "27")]
    // pub disallow_playback_reasons: Vec<String>,
    // #[prost(string, repeated, tag = "28")]
    // pub disallow_transfer_reasons: Vec<String>,
    // #[prost(bool, tag = "29")]
    // pub is_locally_discovered: bool,
    // #[prost(string, optional, tag = "31")]
    // pub group_instance: Option<String>,
    #[prost(bool, tag = "32")]
    pub is_public: bool,
    // #[prost(bool, optional, tag = "34")]
    // pub is_wol_capable: Option<bool>,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "Spotify assigns distinct wire fields"
)]
#[derive(PartialEq, prost::Message)]
pub struct Capabilities {
    #[prost(bool, tag = "2")]
    pub can_be_player: bool,
    // #[prost(bool, tag = "3")]
    // pub restrict_to_local: bool,
    #[prost(bool, tag = "5")]
    pub gaia_eq_connect_id: bool,
    // #[prost(bool, tag = "6")]
    // pub supports_logout: bool,
    #[prost(bool, tag = "7")]
    pub is_observable: bool,
    #[prost(int32, tag = "8")]
    pub volume_steps: i32,
    #[prost(string, repeated, tag = "9")]
    pub supported_types: Vec<String>,
    #[prost(bool, tag = "10")]
    pub command_acks: bool,
    // #[prost(bool, tag = "11")]
    // pub supports_rename: bool,
    #[prost(bool, tag = "12")]
    pub hidden: bool,
    // #[prost(bool, tag = "13")]
    // pub disable_volume: bool,
    // #[prost(bool, tag = "14")]
    // pub connect_disabled: bool,
    #[prost(bool, tag = "15")]
    pub supports_playlist_v2: bool,
    #[prost(bool, tag = "16")]
    pub is_controllable: bool,
    // #[prost(bool, tag = "17")]
    // pub supports_external_episodes: bool,
    // #[prost(bool, tag = "18")]
    // pub supports_set_backend_metadata: bool,
    #[prost(bool, tag = "19")]
    pub supports_transfer_command: bool,
    #[prost(bool, tag = "20")]
    pub supports_command_request: bool,
    // #[prost(bool, tag = "21")]
    // pub is_voice_enabled: bool,
    #[prost(bool, tag = "22")]
    pub needs_full_player_state: bool,
    #[prost(bool, tag = "23")]
    pub supports_gzip_pushes: bool,
    #[prost(bool, tag = "25")]
    pub supports_set_options_command: bool,
    // #[prost(message, optional, tag = "26")]
    // pub supports_hifi: Option<CapabilitySupportDetails>,
    // #[prost(string, tag = "27")]
    // pub connect_capabilities: String,
    // #[prost(bool, tag = "28")]
    // pub supports_rooms: bool,
    // #[prost(bool, tag = "29")]
    // pub supports_dj: bool,
    #[prost(int32, tag = "30")]
    pub supported_audio_quality: i32,
    // #[prost(bool, tag = "33")]
    // pub supports_smart_shuffle_mode: bool,
    // #[prost(bool, tag = "34")]
    // pub supports_remote_sleep_timer: bool,
    // #[prost(bool, tag = "35")]
    // pub supports_ping_request: bool,
    // #[prost(bool, tag = "36")]
    // pub supports_playlist_mixing: bool,
    // #[prost(bool, tag = "37")]
    // pub supports_remote_audio_quality_control: bool,
    // #[prost(bool, tag = "38")]
    // pub supports_zephyr: bool,
    // #[prost(bool, tag = "39")]
    // pub supports_gapless: bool,
    // #[prost(bool, tag = "40")]
    // pub supports_crossfade: bool,
}

/// An outbound volume command, including an explicit zero when muting.
#[derive(prost::Message)]
pub struct SetVolumeRequest {
    #[prost(int32, optional, tag = "1")]
    pub volume: Option<i32>,
}

#[derive(prost::Message)]
pub struct SetVolumeCommand {
    #[prost(int32, tag = "1")]
    pub volume: i32,
    // #[prost(message, optional, tag = "2")]
    // pub command_options: Option<ConnectCommandOptions>,
    // #[prost(message, optional, tag = "3")]
    // pub logging_params: Option<ConnectLoggingParams>,
    // #[prost(string, tag = "4")]
    // pub connection_type: String,
    #[prost(string, tag = "5")]
    pub sent_by_device_id: String,
    // #[prost(string, optional, tag = "6")]
    // pub group_device_identifier: Option<String>,
    // #[prost(string, optional, tag = "7")]
    // pub group_instance: Option<String>,
}

/// Track providers suppressed for this player.
#[derive(prost::Message)]
pub struct Suppressions {
    #[prost(string, repeated, tag = "1")]
    pub providers: Vec<String>,
}

/// Reported track, queue, playback flags, and timing for a Connect session.
#[expect(
    clippy::struct_excessive_bools,
    reason = "Spotify assigns distinct wire fields"
)]
#[derive(prost::Message)]
pub struct PlayerState {
    #[prost(int64, tag = "1")]
    pub timestamp_ms: i64,
    #[prost(string, tag = "2")]
    pub context_uri: String,
    #[prost(string, tag = "3")]
    pub context_url: String,
    // #[prost(message, optional, tag = "4")]
    // pub context_restrictions: Option<Restrictions>,
    #[prost(message, optional, tag = "5")]
    pub play_origin: Option<PlayOrigin>,
    #[prost(message, optional, tag = "6")]
    pub index: Option<ContextIndex>,
    #[prost(message, optional, tag = "7")]
    pub track: Option<ProvidedTrack>,
    #[prost(string, tag = "8")]
    pub playback_id: String,
    #[prost(double, tag = "9")]
    pub playback_speed: f64,
    #[prost(int64, tag = "10")]
    pub position_at_timestamp_ms: i64,
    #[prost(int64, tag = "11")]
    pub duration_ms: i64,
    #[prost(bool, tag = "12")]
    pub is_playing: bool,
    #[prost(bool, tag = "13")]
    pub is_paused: bool,
    #[prost(bool, tag = "14")]
    pub is_buffering: bool,
    #[prost(bool, tag = "15")]
    pub is_system_initiated: bool,
    #[prost(message, optional, tag = "16")]
    pub options: Option<ContextPlayerOptions>,
    // #[prost(message, optional, tag = "17")]
    // pub restrictions: Option<Restrictions>,
    #[prost(message, optional, tag = "18")]
    pub suppressions: Option<Suppressions>,
    #[prost(message, repeated, tag = "19")]
    pub prev_tracks: Vec<ProvidedTrack>,
    #[prost(message, repeated, tag = "20")]
    pub next_tracks: Vec<ProvidedTrack>,
    // #[prost(map = "string, string", tag = "21")]
    // pub context_metadata: std::collections::HashMap<String, String>,
    // #[prost(map = "string, string", tag = "22")]
    // pub page_metadata: std::collections::HashMap<String, String>,
    #[prost(string, tag = "23")]
    pub session_id: String,
    #[prost(string, tag = "24")]
    pub queue_revision: String,
    #[prost(int64, tag = "25")]
    pub position: i64,
    // #[prost(message, optional, tag = "32")]
    // pub playback_quality: Option<PlaybackQuality>,
    #[prost(string, repeated, tag = "33")]
    pub signals: Vec<String>,
    // #[prost(string, tag = "35")]
    // pub session_command_id: String,
    // #[prost(message, optional, tag = "37")]
    // pub ad_break_context: Option<AdBreakContext>,
    // #[prost(message, optional, tag = "38")]
    // pub sleep_timer: Option<SleepTimer>,
}

// #[derive(prost::Message)]
// pub struct DeviceAlias {
//     #[prost(uint32, tag = "1")]
//     pub id: u32,
//     #[prost(string, tag = "2")]
//     pub display_name: String,
//     #[prost(bool, tag = "3")]
//     pub is_group: bool,
// }
//
// #[derive(prost::Message)]
// pub struct AdBreakContext {
//     #[prost(int64, tag = "1")]
//     pub max_seconds_remaining: i64,
//     #[prost(int64, tag = "2")]
//     pub position_in_current_ad_break: i64,
//     #[prost(int64, tag = "3")]
//     pub total_ads_in_break_estimate: i64,
// }

/// Origin of a playback session and the feature or device that started it.
#[derive(prost::Message)]
pub struct PlayOrigin {
    // #[prost(string, tag = "1")]
    // pub feature_identifier: String,
    // #[prost(string, tag = "2")]
    // pub feature_version: String,
    // #[prost(string, tag = "3")]
    // pub view_uri: String,
    // #[prost(string, tag = "4")]
    // pub external_referrer: String,
    // #[prost(string, tag = "5")]
    // pub referrer_identifier: String,
    // #[prost(string, tag = "6")]
    // pub device_identifier: String,
    // #[prost(string, repeated, tag = "7")]
    // pub feature_classes: Vec<String>,
    // #[prost(string, tag = "8")]
    // pub restriction_identifier: String,
}

#[derive(prost::Message, Deserialize)]
#[serde(default)]
pub struct ProvidedTrack {
    #[prost(string, tag = "1")]
    #[serde(deserialize_with = "null_default")]
    pub uri: String,
    #[prost(string, tag = "2")]
    #[serde(deserialize_with = "null_default")]
    pub uid: String,
    #[prost(map = "string, string", tag = "3")]
    #[serde(deserialize_with = "null_default")]
    pub metadata: HashMap<String, String>,
    #[prost(string, repeated, tag = "4")]
    #[serde(skip)]
    pub removed: Vec<String>,
    #[prost(string, repeated, tag = "5")]
    #[serde(skip)]
    pub blocked: Vec<String>,
    #[prost(string, tag = "6")]
    #[serde(deserialize_with = "null_default")]
    pub provider: String,
    // #[prost(message, optional, tag = "7")]
    // pub restrictions: Option<Restrictions>,
    #[prost(string, tag = "8")]
    #[serde(skip)]
    pub album_uri: String,
    #[prost(string, repeated, tag = "9")]
    #[serde(skip)]
    pub disallow_reasons: Vec<String>,
    #[prost(string, tag = "10")]
    #[serde(skip)]
    pub artist_uri: String,
}

#[derive(prost::Message)]
pub struct ContextIndex {
    #[prost(uint32, tag = "1")]
    pub page: u32,
    #[prost(uint32, tag = "2")]
    pub track: u32,
}

#[derive(prost::Message)]
pub struct ContextPlayerOptions {
    #[prost(bool, tag = "1")]
    pub shuffling_context: bool,
    #[prost(bool, tag = "2")]
    pub repeating_context: bool,
    #[prost(bool, tag = "3")]
    pub repeating_track: bool,
    #[prost(float, optional, tag = "4")]
    pub playback_speed: Option<f32>,
    // #[prost(map = "string, string", tag = "5")]
    // pub modes: std::collections::HashMap<String, String>,
}

#[derive(Debug, prost::Enumeration)]
pub enum DeviceType {
    Unknown = 0,
    Computer = 1,
    Tablet = 2,
    Smartphone = 3,
    Speaker = 4,
    Tv = 5,
    Avr = 6,
    Stb = 7,
    AudioDongle = 8,
    GameConsole = 9,
    CastVideo = 10,
    CastAudio = 11,
    Automobile = 12,
    Smartwatch = 13,
    Chromebook = 14,
    UnknownSpotify = 100,
    CarThing = 101,
    Observer = 102,
    HomeThing = 103,
}

#[derive(Clone, Copy, Debug, prost::Enumeration)]
pub enum PutStateReason {
    Unknown = 0,
    SpircHello = 1,
    SpircNotify = 2,
    NewDevice = 3,
    PlayerStateChanged = 4,
    VolumeChanged = 5,
    PickerOpened = 6,
    BecameInactive = 7,
    AliasChanged = 8,
    NewConnection = 9,
    PullPlayback = 10,
    AudioDriverInfoChanged = 11,
    PutStateRateLimited = 12,
    BackendMetadataApplied = 13,
}

#[derive(prost::Message, Deserialize)]
#[serde(default)]
pub struct Context {
    #[prost(string, optional, tag = "1")]
    pub uri: Option<String>,
    #[prost(string, optional, tag = "2")]
    #[serde(skip)]
    pub url: Option<String>,
    #[prost(map = "string, string", tag = "3")]
    #[serde(skip)]
    pub metadata: std::collections::HashMap<String, String>,
    // #[prost(message, optional, tag = "4")]
    // pub restrictions: Option<Restrictions>,
    #[prost(message, repeated, tag = "5")]
    #[serde(deserialize_with = "null_default")]
    pub pages: Vec<ContextPage>,
    #[prost(bool, optional, tag = "6")]
    #[serde(skip)]
    pub loading: Option<bool>,
}

#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ContextPage {
    #[prost(string, optional, tag = "1")]
    #[serde(alias = "page_url")]
    pub page_url: Option<String>,
    #[prost(string, optional, tag = "2")]
    #[serde(alias = "next_page_url")]
    pub next_page_url: Option<String>,
    #[prost(map = "string, string", tag = "3")]
    #[serde(skip)]
    pub metadata: std::collections::HashMap<String, String>,
    #[prost(message, repeated, tag = "4")]
    #[serde(deserialize_with = "null_default")]
    pub tracks: Vec<ContextTrack>,
    #[prost(bool, optional, tag = "5")]
    #[serde(skip)]
    pub loading: Option<bool>,
}

#[derive(prost::Message, Deserialize)]
#[serde(default)]
pub struct ContextTrack {
    #[prost(string, optional, tag = "1")]
    pub uri: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub uid: Option<String>,
    #[prost(bytes = "bytes", optional, tag = "3")]
    #[serde(deserialize_with = "track_gid")]
    pub gid: Option<Bytes>,
    #[prost(map = "string, string", tag = "4")]
    #[serde(skip)]
    pub metadata: std::collections::HashMap<String, String>,
}

#[derive(prost::Message)]
pub struct TransferState {
    #[prost(message, optional, tag = "1")]
    pub options: Option<TransferOptions>,
    #[prost(message, optional, tag = "2")]
    pub playback: Option<Playback>,
    #[prost(message, optional, tag = "3")]
    pub current_session: Option<TransferSession>,
    #[prost(message, optional, tag = "4")]
    pub queue: Option<Queue>,
    #[prost(message, optional, tag = "5")]
    pub play_history: Option<PlayHistory>,
    // The entry schema at tag 6 is unverified.
    // #[prost(bytes, repeated, tag = "6")]
    // pub play_history_entries: Vec<Bytes>,
    // #[prost(message, optional, tag = "7")]
    // pub sleep_timer: Option<SleepTimer>,
}

// #[derive(prost::Message)]
// pub struct SleepTimer {
//     #[prost(oneof = "SleepTimerType", tags = "1, 2, 3")]
//     pub timer_type: Option<SleepTimerType>,
// }
//
// #[derive(prost::Oneof)]
// pub enum SleepTimerType {
//     #[prost(message, tag = "1")]
//     None(()),
//     #[prost(message, tag = "2")]
//     Timestamp(SleepTimerTimestamp),
//     #[prost(message, tag = "3")]
//     EndOfTrack(()),
// }
//
// #[derive(prost::Message)]
// pub struct SleepTimerTimestamp {
//     #[prost(int64, tag = "1")]
//     pub timestamp: i64,
// }

#[derive(prost::Message)]
pub struct TransferSession {
    // #[prost(message, optional, tag = "1")]
    // pub play_origin: Option<TransferPlayOrigin>,
    #[prost(message, optional, tag = "2")]
    pub context: Option<Context>,
    #[prost(string, optional, tag = "3")]
    pub current_uid: Option<String>,
    // #[prost(message, optional, tag = "4")]
    // pub option_overrides: Option<ContextPlayerOptionOverrides>,
    // #[prost(message, optional, tag = "5")]
    // pub suppressions: Option<TransferSuppressions>,
    // #[prost(message, optional, tag = "6")]
    // pub instrumentation_params: Option<InstrumentationParams>,
    // #[prost(string, optional, tag = "7")]
    // pub shuffle_seed: Option<String>,
    // #[prost(message, optional, tag = "8")]
    // pub main_context: Option<Context>,
    // #[prost(string, optional, tag = "9")]
    // pub original_session_id: Option<String>,
    // #[prost(message, optional, tag = "10")]
    // pub playlist_play_request: Option<PlaylistPlayRequest>,
}

#[derive(prost::Message)]
pub struct Queue {
    #[prost(message, repeated, tag = "1")]
    pub tracks: Vec<ContextTrack>,
    #[prost(bool, optional, tag = "2")]
    pub is_playing_queue: Option<bool>,
}

#[derive(prost::Message)]
pub struct Playback {
    #[prost(int64, optional, tag = "1")]
    pub timestamp_ms: Option<i64>,
    #[prost(int32, optional, tag = "2")]
    pub position_at_timestamp_ms: Option<i32>,
    #[prost(double, optional, tag = "3")]
    pub playback_speed: Option<f64>,
    #[prost(bool, optional, tag = "4")]
    pub is_paused: Option<bool>,
    #[prost(message, optional, tag = "5")]
    pub current_track: Option<ContextTrack>,
    // #[prost(message, optional, tag = "6")]
    // pub associated_current_track: Option<ContextTrack>,
    // #[prost(int32, optional, tag = "7")]
    // pub associated_position_as_of_timestamp: Option<i32>,
}

#[derive(prost::Message)]
pub struct PlayHistory {
    #[prost(message, repeated, tag = "1")]
    pub backward_items: Vec<HistoryItem>,
    // #[prost(message, repeated, tag = "2")]
    // pub forward_items: Vec<HistoryItem>,
}

#[derive(prost::Message)]
pub struct HistoryItem {
    #[prost(string, optional, tag = "1")]
    pub context_id: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub uid: Option<String>,
    // #[prost(bool, optional, tag = "3")]
    // pub disliked: Option<bool>,
    #[prost(message, repeated, tag = "4")]
    pub children: Vec<HistoryItem>,
}

#[derive(prost::Message, Deserialize)]
pub struct TransferOptions {
    #[prost(bool, optional, tag = "1")]
    pub shuffling_context: Option<bool>,
    #[prost(bool, optional, tag = "2")]
    pub repeating_context: Option<bool>,
    #[prost(bool, optional, tag = "3")]
    pub repeating_track: Option<bool>,
    #[prost(float, optional, tag = "4")]
    pub playback_speed: Option<f32>,
    // #[prost(map = "string, string", tag = "5")]
    // pub modes: std::collections::HashMap<String, String>,
}

fn track_gid<'de, D: Deserializer<'de>>(input: D) -> Result<Option<Bytes>, D::Error> {
    Option::<String>::deserialize(input)?
        .map(|text| {
            let alphabet = if text.contains(['-', '_']) {
                &alphabet::URL_SAFE
            } else {
                &alphabet::STANDARD
            };
            GeneralPurpose::new(
                alphabet,
                GeneralPurposeConfig::new()
                    .with_decode_padding_mode(DecodePaddingMode::Indifferent),
            )
            .decode(text)
            .map(Bytes::from)
            .map_err(serde::de::Error::custom)
        })
        .transpose()
}

impl Session {
    /// Publishes device state and returns the current account cluster.
    pub async fn put_device_state(
        &self,
        device: &str,
        connection: &str,
        request: &PutStateRequest,
    ) -> Result<Cluster> {
        let body = self
            .device_state(
                device,
                connection,
                Method::PUT,
                "",
                request.encode_to_vec().into(),
            )
            .await?;
        Ok(Cluster::decode(body)?)
    }

    /// Marks a registered device inactive without notifying its peers.
    pub async fn deactivate_device(&self, device: &str, connection: &str) -> Result<()> {
        self.device_state(
            device,
            connection,
            Method::PUT,
            "/inactive?notify=false",
            Bytes::new(),
        )
        .await?;
        Ok(())
    }

    /// Unregisters a device from Spotify Connect.
    pub async fn unregister_device(&self, device: &str) -> Result<()> {
        self.device_state(device, "", Method::DELETE, "", Bytes::new())
            .await?;
        Ok(())
    }

    /// Sets the volume of another Spotify Connect device.
    pub async fn set_volume(
        &self,
        from: &str,
        to: &str,
        connection: &str,
        request: &SetVolumeRequest,
    ) -> Result<()> {
        self.device_command(
            Method::PUT,
            &between_devices(VOLUME, from, to),
            connection,
            http::PROTOBUF,
            request.encode_to_vec().into(),
        )
        .await
    }

    /// Transfers playback between devices and restores the paused state.
    pub async fn transfer_playback(&self, from: &str, to: &str, connection: &str) -> Result<()> {
        let body = json!({"transfer_options": {"restore_paused": "restore"}});
        self.device_command(
            Method::POST,
            &between_devices(TRANSFER, from, to),
            connection,
            "application/json",
            serde_json::to_vec(&body)?.into(),
        )
        .await
    }

    async fn device_state(
        &self,
        device: &str,
        connection: &str,
        method: Method,
        suffix: &str,
        body: Bytes,
    ) -> Result<Bytes> {
        let mut headers = http::proto_headers(http::PROTOBUF);
        if method == Method::PUT {
            headers.insert("x-spotify-connection-id", connection.parse()?);
            if !body.is_empty() {
                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(http::PROTOBUF),
                );
            }
        }
        let path = format!("{}/{}{suffix}", DEVICES, component(device));
        http::send(self, method, &path, headers, body).await
    }

    async fn device_command(
        &self,
        method: Method,
        path: &str,
        connection: &str,
        mime: &'static str,
        body: Bytes,
    ) -> Result<()> {
        let mut headers = dealer_headers(connection)?;
        headers.extend(http::proto_headers(mime));
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
        http::send(self, method, path, headers, body).await?;
        Ok(())
    }
}

impl ProvidedTrack {
    /// Encodes the occurrence as it is published in current, previous, and upcoming tracks.
    pub fn for_queue(
        id: &str,
        uid: String,
        context: &str,
        queued: bool,
        suggested: bool,
        added_by: Option<&str>,
    ) -> Self {
        let mut metadata = HashMap::from([
            ("context_uri".to_owned(), context.to_owned()),
            ("entity_uri".to_owned(), context.to_owned()),
        ]);
        if queued {
            metadata.insert("is_queued".into(), "true".into());
        }
        if let Some(adder) = added_by {
            metadata.insert("added_by".into(), adder.into());
        }
        Self {
            uri: crate::id::track_uri(id),
            uid,
            provider: match (queued, suggested) {
                (true, _) => "queue",
                (false, true) => "autoplay",
                (false, false) => "context",
            }
            .into(),
            metadata,
            ..Default::default()
        }
    }

    /// Returns the requested recording when Spotify reports a playable replacement.
    pub fn track_id(&self) -> Option<&str> {
        let reported = crate::id::track_id(&self.uri)?;
        self.metadata
            .get("requested_uri")
            .and_then(|uri| crate::id::track_id(uri))
            .filter(|id| id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
            .or(Some(reported))
    }

    /// Whether the occurrence was added explicitly to the queue.
    pub fn is_queued(&self) -> bool {
        self.provider == "queue"
            || self
                .metadata
                .get("is_queued")
                .is_some_and(|value| value == "true")
    }

    /// The account that added the track, when Spotify supplies it.
    pub fn added_by(&self) -> Option<&str> {
        ["added_by", "added_by_username", "adder", "queued_by"]
            .iter()
            .find_map(|key| self.metadata.get(*key))
            .filter(|name| !name.is_empty())
            .map(String::as_str)
    }
}

impl ContextTrack {
    /// Reads the catalogue ID from a context row, including rows that only carry the binary GID.
    pub fn track_id(&self) -> Option<crate::id::CatalogueId<'_>> {
        use crate::id::{CatalogueId, base62_bytes, track_id};
        if let Some(id) = self.uri.as_deref().and_then(track_id) {
            return Some(CatalogueId::Borrowed(id));
        }
        base62_bytes(self.gid.as_deref()?).map(CatalogueId::Encoded)
    }
}

impl TransferState {
    /// Decodes the transfer state embedded in a Dealer command.
    pub fn from_command_data(data: &str) -> Result<Self> {
        use base64::engine::general_purpose::STANDARD;
        Ok(Self::decode(Bytes::from(STANDARD.decode(data)?))?)
    }
}
