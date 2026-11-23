//! API identity and Spotify service discovery for one account.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use reqwest::Url;
use tokio::sync::OnceCell;

use crate::{
    auth::OAuth,
    http::{self, Endpoints},
};

/// A signed-in account and its discovered Spotify servers.
pub struct Session {
    oauth: Arc<OAuth>,
    endpoints: OnceCell<Endpoints>,
}

impl Session {
    pub fn new(oauth: Arc<OAuth>) -> Self {
        Self {
            oauth,
            endpoints: OnceCell::new(),
        }
    }

    pub fn username(&self) -> &str {
        self.oauth.username()
    }

    pub fn client_id(&self) -> &str {
        self.oauth.client_id()
    }

    pub fn has_current_grant(&self) -> bool {
        self.oauth.has_current_grant()
    }

    /// Gets a valid bearer for Spotify requests.
    pub async fn access_token(&self) -> Result<String> {
        self.oauth.token().await
    }

    /// Discovered access-point addresses for native Spotify audio connections.
    pub async fn access_points(&self) -> Result<&[String]> {
        Ok(&self.endpoints().await?.accesspoint)
    }

    /// Picks a regional host or the global host supplied in the discovery response.
    pub(crate) async fn api_url(&self) -> Result<Url> {
        let hosts = &self.endpoints().await?.spclient;
        hosts
            .regional
            .iter()
            .chain(&hosts.global)
            .find_map(|address| http::regional_url("https", address))
            .context("Spotify discovery returned no API host")
    }

    /// Returns the Dealer hosts supplied in the discovery response.
    pub(crate) async fn dealer_urls(&self) -> Result<Vec<Url>> {
        Ok(self
            .endpoints()
            .await?
            .dealer
            .iter()
            .filter_map(|address| http::regional_url("wss", address))
            .collect())
    }

    async fn endpoints(&self) -> Result<&Endpoints> {
        self.endpoints
            .get_or_try_init(http::discover)
            .await
            .context("Spotify endpoint discovery failed")
    }
}
