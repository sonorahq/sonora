use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{Account, Link, Play, Secret, Service, host, root};

/// Where the native API lives under a Maloja server's root.
const API: &str = "/apis/mlj_1";

/// Maloja has no now-playing concept, so it keeps the default and only submits finished listens,
/// one request each.
pub struct Maloja;

#[async_trait]
impl Service for Maloja {
    fn id(&self) -> &'static str {
        "maloja"
    }

    fn link(&self) -> Link {
        Link::Server
    }

    async fn connect(&self, secret: Secret) -> Result<Account> {
        let Secret::Server { url, key } = secret else {
            bail!("maloja needs a server url and an api key");
        };
        let (url, key) = (root(&url), key.trim().to_owned());
        if url.is_empty() || key.is_empty() {
            bail!("maloja needs a server url and an api key");
        }

        let answer = super::http()
            .get(format!("{url}{API}/test"))
            .query(&[("key", &key)])
            .send()
            .await
            .context("cannot reach the maloja server")?;
        if !answer.status().is_success() {
            bail!(
                "the maloja server refused the api key ({})",
                answer.status()
            );
        }

        Ok(Account {
            name: name(&url).await.unwrap_or_else(|| host(&url)),
            server: url,
            session: key,
            enabled: true,
            ..Account::default()
        })
    }

    async fn scrobble(&self, account: &Account, plays: &[Play]) -> Result<()> {
        for play in plays {
            let answer: Answer = super::http()
                .post(format!("{}{API}/newscrobble", account.server))
                .json(&New {
                    key: &account.session,
                    artists: std::slice::from_ref(&play.artist),
                    title: &play.title,
                    album: play.release(),
                    length: play.duration.as_secs(),
                    time: play.timestamp(),
                })
                .send()
                .await
                .context("cannot reach the maloja server")?
                .json()
                .await
                .context("cannot read the maloja answer")?;

            if answer.status != "success" && answer.status != "ok" {
                let reason = answer
                    .error
                    .map(|error| error.desc)
                    .unwrap_or_else(|| answer.status.clone());
                bail!("the maloja server refused the scrobble: {reason}");
            }
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct New<'a> {
    key: &'a str,
    artists: &'a [String],
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    album: Option<&'a str>,
    length: u64,
    time: i64,
}

#[derive(Deserialize)]
struct Answer {
    status: String,
    error: Option<Refusal>,
}

#[derive(Deserialize)]
struct Refusal {
    desc: String,
}

#[derive(Deserialize)]
struct Info {
    name: Option<String>,
}

/// What the instance calls itself, which is friendlier in the settings row than its host.
async fn name(url: &str) -> Option<String> {
    let info: Info = super::http()
        .get(format!("{url}{API}/serverinfo"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    info.name.filter(|name| !name.is_empty())
}
