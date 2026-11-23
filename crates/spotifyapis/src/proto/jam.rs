//! Social Connect session messages and Dealer updates.

use serde::{Deserialize, Deserializer};

use super::null_default;

pub const JAM_TOPIC: &str = "social-connect/v2/session_update";
pub(crate) const JAM_JOIN: &str = "/social-connect/v2/sessions/join";
pub(crate) const JAM_CURRENT: &str = "/social-connect/v2/sessions/current";
pub(crate) const JAM_START: &str = "/social-connect/v2/sessions/current_or_new";
pub(crate) const JAM_INFO: &str = "/social-connect/v2/sessions/info";
pub(crate) const JAM_ACTIONS: &str = "/social-connect/v3/sessions";

#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SocialSession {
    #[prost(int64, tag = "1")]
    #[serde(skip)]
    pub timestamp: i64,
    #[prost(string, tag = "2")]
    #[serde(alias = "session_id", deserialize_with = "null_default")]
    pub session_id: String,
    #[prost(string, tag = "3")]
    #[serde(skip)]
    pub join_session_token: String,
    #[prost(string, tag = "4")]
    #[serde(alias = "join_session_url", deserialize_with = "null_default")]
    pub join_session_url: String,
    #[prost(string, tag = "5")]
    #[serde(alias = "session_owner_id", deserialize_with = "null_default")]
    pub session_owner_id: String,
    #[prost(message, repeated, tag = "6")]
    #[serde(alias = "session_members", deserialize_with = "null_default")]
    pub session_members: Vec<SessionMember>,
    #[prost(string, tag = "7")]
    #[serde(skip)]
    pub join_session_uri: String,
    #[prost(bool, tag = "9")]
    #[serde(alias = "is_session_owner", deserialize_with = "null_default")]
    pub is_session_owner: bool,
    #[prost(bool, tag = "10")]
    #[serde(skip)]
    pub is_listening: bool,
    #[prost(bool, tag = "11")]
    #[serde(skip)]
    pub is_controlling: bool,
    #[prost(bool, tag = "12")]
    #[serde(skip)]
    pub is_discoverable: bool,
    #[prost(int32, tag = "13")]
    #[serde(skip)]
    pub initial_session_type: i32,
    #[prost(string, optional, tag = "14")]
    #[serde(skip)]
    pub host_active_device_id: Option<String>,
    #[prost(bool, tag = "17")]
    #[serde(skip)]
    pub active: bool,
    #[prost(bool, tag = "25")]
    #[serde(skip)]
    pub redirect_commands: bool,
}

#[derive(prost::Message, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionMember {
    #[prost(int64, tag = "1")]
    #[serde(skip)]
    pub timestamp: i64,
    #[prost(string, tag = "2")]
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[prost(string, tag = "3")]
    #[serde(deserialize_with = "null_default")]
    pub username: String,
    #[prost(string, tag = "4")]
    #[serde(alias = "display_name", deserialize_with = "null_default")]
    pub display_name: String,
    #[prost(string, tag = "5")]
    #[serde(alias = "image_url", deserialize_with = "null_default")]
    pub image_url: String,
    #[prost(string, tag = "6")]
    #[serde(alias = "large_image_url", deserialize_with = "null_default")]
    pub large_image_url: String,
    #[prost(bool, tag = "7")]
    #[serde(skip)]
    pub is_listening: bool,
    #[prost(bool, tag = "8")]
    #[serde(skip)]
    pub is_controlling: bool,
    #[prost(bool, tag = "10")]
    #[serde(skip)]
    pub is_current_user: bool,
}

#[derive(prost::Message, Deserialize)]
#[serde(default)]
pub struct SessionUpdate {
    #[prost(message, optional, tag = "1")]
    pub session: Option<SocialSession>,
    #[prost(enumeration = "SessionUpdateReason", tag = "2")]
    #[serde(deserialize_with = "update_reason")]
    pub reason: i32,
    #[prost(message, repeated, tag = "3")]
    #[serde(skip)]
    pub updated_session_members: Vec<SessionMember>,
}

#[derive(Debug, prost::Enumeration, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionUpdateReason {
    Unknown = 0,
    NewSession = 1,
    UserJoined = 2,
    UserLeft = 3,
    SessionDeleted = 4,
    YouLeft = 5,
    YouWereKicked = 6,
    YouJoined = 7,
    ParticipantPromotedToHost = 8,
    DiscoverabilityChanged = 9,
    UserKicked = 10,
}

fn update_reason<'de, D: Deserializer<'de>>(input: D) -> Result<i32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Reason {
        Number(i32),
        Name(SessionUpdateReason),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::deserialize(input)? {
        Some(Reason::Number(value)) => value,
        Some(Reason::Name(value)) => value as i32,
        _ => 0,
    })
}
