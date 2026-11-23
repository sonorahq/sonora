//! Social Connect operations.

use anyhow::Result;
use bytes::Bytes;
use prost::Message as _;
use reqwest::Method;

use super::{component, dealer_headers};
use crate::{
    Session, http,
    proto::jam::{self, SocialSession},
};

impl Session {
    /// Reads the current Jam.
    pub async fn current_jam(&self, connection: &str) -> Result<Option<SocialSession>> {
        self.read_jam(Method::GET, jam::JAM_CURRENT, connection)
            .await
    }

    /// Looks up an invite without joining.
    pub async fn jam_invite(&self, token: &str, connection: &str) -> Result<Option<SocialSession>> {
        self.read_jam(
            Method::GET,
            &format!("{}/{}", jam::JAM_INFO, component(token)),
            connection,
        )
        .await
    }

    /// Starts or resumes a Jam.
    pub async fn start_jam(&self, device: &str, connection: &str) -> Result<Option<SocialSession>> {
        let path = format!(
            "{}?activate=true&local_device_id={}&type=REMOTE",
            jam::JAM_START,
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
            jam::JAM_JOIN,
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
            jam::JAM_ACTIONS,
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
            jam::JAM_ACTIONS,
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
