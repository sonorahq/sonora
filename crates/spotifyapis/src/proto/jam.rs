//! Social Connect session messages and Dealer updates.

use anyhow::Result;
use bytes::Bytes;
use prost::Message as _;
use reqwest::Method;
use serde::{Deserialize, Deserializer};

use super::{component, dealer_headers, null_default};
use crate::{Session, http};

pub const JAM_TOPIC: &str = "social-connect/v2/session_update";
const JAM_JOIN: &str = "/social-connect/v2/sessions/join";
const JAM_CURRENT: &str = "/social-connect/v2/sessions/current";
const JAM_START: &str = "/social-connect/v2/sessions/current_or_new";
const JAM_INFO: &str = "/social-connect/v2/sessions/info";
const JAM_ACTIONS: &str = "/social-connect/v3/sessions";

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

/// Dealer's JSON notification for a change to a Jam.
#[derive(Default, Deserialize)]
#[serde(default)]
pub struct SessionUpdate {
    // #[prost(message, optional, tag = "1")]
    pub session: Option<SocialSession>,
    // #[prost(enumeration = "SessionUpdateReason", tag = "2")]
    #[serde(deserialize_with = "update_reason")]
    pub reason: SessionUpdateReason,
    // #[prost(message, repeated, tag = "3")]
    // pub updated_session_members: Vec<SessionMember>,
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

fn update_reason<'de, D: Deserializer<'de>>(input: D) -> Result<SessionUpdateReason, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Reason {
        Number(i32),
        Name(SessionUpdateReason),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Option::deserialize(input)? {
        Some(Reason::Number(value)) => SessionUpdateReason::try_from(value).unwrap_or_default(),
        Some(Reason::Name(value)) => value,
        _ => SessionUpdateReason::Unknown,
    })
}

impl Session {
    /// Reads the current Jam.
    pub async fn current_jam(&self, connection: &str) -> Result<Option<SocialSession>> {
        self.read_jam(Method::GET, JAM_CURRENT, connection).await
    }

    /// Looks up an invite without joining.
    pub async fn jam_invite(&self, token: &str, connection: &str) -> Result<Option<SocialSession>> {
        self.read_jam(
            Method::GET,
            &format!("{}/{}", JAM_INFO, component(token)),
            connection,
        )
        .await
    }

    /// Starts or resumes a Jam.
    pub async fn start_jam(&self, device: &str, connection: &str) -> Result<Option<SocialSession>> {
        let path = format!(
            "{}?activate=true&local_device_id={}&type=REMOTE",
            JAM_START,
            component(device)
        );
        self.read_jam(Method::GET, &path, connection).await
    }

    /// Joins the named Jam as a remote participant.
    pub async fn join_jam(
        &self,
        token: &str,
        device: &str,
        connection: &str,
    ) -> Result<Option<SocialSession>> {
        let path = format!(
            "{}/{}?playback_control=listen_and_control&join_type=deeplinking&local_device_id={}",
            JAM_JOIN,
            component(token),
            component(device)
        );
        self.read_jam(Method::POST, &path, connection).await
    }

    /// Ends a hosted Jam or leaves it as a guest.
    pub async fn leave_jam(
        &self,
        id: &str,
        owner: bool,
        device: &str,
        connection: &str,
    ) -> Result<()> {
        let (method, suffix) = if owner {
            (Method::DELETE, "")
        } else {
            (Method::POST, "/leave")
        };
        let path = format!(
            "{}/{}{suffix}?local_device_id={}",
            JAM_ACTIONS,
            component(id),
            component(device)
        );
        self.jam_request(method, &path, connection).await?;
        Ok(())
    }

    /// Removes a participant from a hosted Jam.
    pub async fn kick_jam_member(
        &self,
        id: &str,
        member: &str,
        device: &str,
        connection: &str,
    ) -> Result<()> {
        let path = format!(
            "{}/{}/member/{}/kick?local_device_id={}",
            JAM_ACTIONS,
            component(id),
            component(member),
            component(device)
        );
        self.jam_request(Method::POST, &path, connection).await?;
        Ok(())
    }

    async fn read_jam(
        &self,
        method: Method,
        path: &str,
        connection: &str,
    ) -> Result<Option<SocialSession>> {
        let body = self.jam_request(method, path, connection).await?;
        let session = SocialSession::decode(body)?;
        Ok((!session.session_id.is_empty()).then_some(session))
    }

    async fn jam_request(&self, method: Method, path: &str, connection: &str) -> Result<Bytes> {
        let separator = if path.contains('?') { '&' } else { '?' };
        let mut headers = dealer_headers(connection)?;
        headers.extend(http::proto_headers(http::PROTOBUF));
        if let Some(id) = headers.get("x-spotify-connection-id").cloned() {
            headers.insert("spotify-connection-id", id);
        }
        http::send(
            self,
            method,
            &format!("{path}{separator}alt=protobuf"),
            headers,
            Bytes::new(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dealer_jam_reasons_accept_observed_json_forms() {
        for (input, expected) in [
            (
                r#"{"reason":"SESSION_DELETED","session":null}"#,
                SessionUpdateReason::SessionDeleted,
            ),
            (r#"{"reason":6}"#, SessionUpdateReason::YouWereKicked),
            (r#"{"reason":"FUTURE_VALUE"}"#, SessionUpdateReason::Unknown),
        ] {
            let update: SessionUpdate = serde_json::from_str(input).unwrap();
            assert_eq!(update.reason as i32, expected as i32);
            assert!(update.session.is_none());
        }
    }
}
