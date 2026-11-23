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
    endpoints: OnceCell<Discovered>,
    market: OnceCell<String>,
}

/// Keeps validated API bases in discovery order alongside the other service addresses.
struct Discovered {
    endpoints: Endpoints,
    api_urls: Vec<Url>,
}

impl Session {
    pub fn new(oauth: Arc<OAuth>) -> Self {
        Self {
            oauth,
            endpoints: OnceCell::new(),
            market: OnceCell::new(),
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

    /// Obtains the account's market when a catalogue reply needs it to disambiguate results.
    pub async fn account_market(&self) -> Result<&str> {
        let market = self
            .market
            .get_or_try_init(|| async {
                // Leave room for a second access point and its country packet.
                tokio::time::timeout(std::time::Duration::from_secs(15), async {
                    let (addresses, token) =
                        tokio::try_join!(self.access_points(), self.access_token())?;
                    crate::access_point::account_market(addresses, &token, self.username()).await
                })
                .await
                .context("Spotify account market lookup timed out")?
            })
            .await?;
        Ok(market)
    }

    /// Gets a valid bearer for Spotify requests.
    pub(crate) async fn access_token(&self) -> Result<String> {
        self.oauth.token().await
    }

    /// Discovered access-point addresses for native Spotify audio connections.
    pub(crate) async fn access_points(&self) -> Result<&[String]> {
        Ok(&self.endpoints().await?.endpoints.accesspoint)
    }

    /// Lists validated API bases, with regional hosts before global hosts.
    pub(crate) async fn api_urls(&self) -> Result<&[Url]> {
        Ok(&self.endpoints().await?.api_urls)
    }

    /// Returns the Dealer hosts supplied in the discovery response.
    pub(crate) async fn dealer_urls(&self) -> Result<Vec<Url>> {
        Ok(self
            .endpoints()
            .await?
            .endpoints
            .dealer
            .iter()
            .filter_map(|address| http::regional_url("wss", address))
            .collect())
    }

    async fn endpoints(&self) -> Result<&Discovered> {
        self.endpoints
            .get_or_try_init(|| async {
                let endpoints = http::discover().await?;
                let mut api_urls = Vec::new();
                for url in endpoints
                    .spclient
                    .regional
                    .iter()
                    .chain(&endpoints.spclient.global)
                    .filter_map(|address| http::regional_url("https", address))
                {
                    if !api_urls.contains(&url) {
                        api_urls.push(url);
                    }
                }
                anyhow::ensure!(
                    !api_urls.is_empty(),
                    "Spotify discovery returned no API host"
                );
                Ok(Discovered {
                    endpoints,
                    api_urls,
                })
            })
            .await
            .context("Spotify endpoint discovery failed")
    }
}
