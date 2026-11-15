//! Where the Sonora Sessions server is: the official one, or one the listener runs, whose address
//! is read leniently and checked before it is trusted.

use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use reqwest::Url;

/// The official server.
pub(crate) const OFFICIAL: &str = "wss://sessions.sonorahq.org/ws";
/// Points the app at another server whatever the settings say, as one run locally while working
/// on it.
pub(crate) const VARIABLE: &str = "SONORA_SESSION_SERVER";
/// How long a health check waits for an answer.
const PATIENCE: Duration = Duration::from_secs(6);

/// The WebSocket address `typed` stands for. A bare host or an http address is taken to mean the
/// server's WebSocket on the same host, and an address without a path gets the server's `/ws`.
pub(crate) fn socket(typed: &str) -> Option<Url> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    let written = match typed.split_once("://") {
        Some(("ws" | "wss", _)) => typed.to_owned(),
        Some(("http", rest)) => format!("ws://{rest}"),
        Some(("https", rest)) => format!("wss://{rest}"),
        Some(_) => return None,
        None => format!("wss://{typed}"),
    };
    let mut url = Url::parse(&written).ok()?;
    url.host_str()?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/ws");
    }
    Some(url)
}

/// Asks the server behind `socket` whether it is up, by its `/health` over plain HTTP.
pub(crate) async fn check(http: reqwest::Client, socket: Url) -> Result<()> {
    let mut health = socket;
    let scheme = match health.scheme() {
        "ws" => "http",
        _ => "https",
    };
    health
        .set_scheme(scheme)
        .ok()
        .context("cannot make a health address")?;
    health.set_path("/health");
    health.set_query(None);
    let answer = http
        .get(health)
        .timeout(PATIENCE)
        .send()
        .await
        .context("cannot reach the server")?;
    if !answer.status().is_success() {
        bail!("the server answered {}", answer.status());
    }
    let body = answer.text().await.context("cannot read the answer")?;
    if body.trim() != "ok" {
        bail!("the server does not answer as a Sonora Sessions server");
    }
    Ok(())
}
