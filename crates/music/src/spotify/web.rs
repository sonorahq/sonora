//! Spotify's cookie exchange and authenticated web playback requests. Secrets stay in memory
//! except for the owner-only cookie file; CDN requests never carry account credentials.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail, ensure};
use reqwest::{Client, Method};
use ring::hmac;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::auth::{self, AuthConfig};
use crate::credentials;

const VERSION: &str = "1.3.6.121.g2b2ea800fe12";
const SECRETS: &str =
    "https://raw.githubusercontent.com/xyloflake/spot-secrets-go/main/secrets/secretDict.json";
const PLAYER: &str = "https://gae2-spclient.spotify.com";
const CEILING: usize = 1 << 20;
const FILE: &str = "web-cookie";

#[derive(Clone)]
pub(super) struct Web(Arc<Inner>);
struct Inner {
    http: Client,
    cookie: String,
    device: String,
    access: Mutex<Option<Access>>,
    material: Mutex<Option<(u32, String)>>,
}
struct Access {
    token: Token,
    client_token: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Token {
    access_token: String,
    access_token_expiration_timestamp_ms: u64,
    client_id: String,
    is_anonymous: bool,
}

impl Web {
    pub async fn open(config: &AuthConfig, input: &str) -> Result<Self> {
        let cookie = cookie(input)?;
        let web = Self(Arc::new(Inner {
            http: Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36")
                .default_headers([(reqwest::header::ACCEPT, reqwest::header::HeaderValue::from_static("application/json"))].into_iter().collect())
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(45))
                .build()?,
            cookie,
            device: auth::device_id(&config.cache_dir),
            access: Mutex::new(None),
            material: Mutex::new(None),
        }));
        drop(web.access().await?);
        Ok(web)
    }

    pub fn stored(config: &AuthConfig) -> bool {
        config.cache_dir.join(FILE).exists()
    }
    pub fn read(config: &AuthConfig) -> Result<String> {
        let path = config.cache_dir.join(FILE);
        credentials::secure(&path);
        std::fs::read_to_string(path).context("cannot read Spotify cookie")
    }
    pub fn save(&self, config: &AuthConfig) -> Result<()> {
        credentials::write(&config.cache_dir.join(FILE), self.0.cookie.as_bytes())
    }
    pub fn forget(config: &AuthConfig) {
        credentials::remove(&config.cache_dir.join(FILE));
    }

    pub async fn session(&self) -> Result<librespot_core::Session> {
        let held = self.access().await?;
        let access = held.as_ref().context("Spotify session is missing")?;
        let session = librespot_core::Session::new(
            librespot_core::SessionConfig {
                // Native AP state supplies account metadata; HTTP uses the cookie bearer.
                device_id: self.0.device.clone(),
                ..Default::default()
            },
            None,
        );
        let token = access.token.access_token.clone();
        drop(held);
        tokio::time::timeout(
            Duration::from_secs(30),
            session.connect(
                librespot_core::authentication::Credentials::with_access_token(&token),
                false,
            ),
        )
        .await
        .context("Spotify sign-in timed out")??;
        Ok(session)
    }

    async fn access(&self) -> Result<tokio::sync::MutexGuard<'_, Option<Access>>> {
        let mut held = self.0.access.lock().await;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
        if held
            .as_ref()
            .is_some_and(|access| access.token.access_token_expiration_timestamp_ms > now + 60_000)
        {
            return Ok(held);
        }
        let (version, secret) = {
            let mut material = self.0.material.lock().await;
            if material.is_none() {
                // Public web-player material; this request never carries account credentials.
                let secrets: BTreeMap<String, Vec<u8>> =
                    serde_json::from_slice(&body(self.0.http.get(SECRETS).send().await?).await?)
                        .context("invalid Spotify TOTP dictionary")?;
                *material = Some(decode_material(&secrets)?);
            }
            material
                .as_ref()
                .context("Spotify TOTP material is missing")?
                .clone()
        };
        let clock: Value = serde_json::from_slice(
            &body(
                self.0
                    .http
                    .get("https://open.spotify.com/api/server-time")
                    .send()
                    .await?,
            )
            .await?,
        )
        .context("invalid Spotify server time response")?;
        let timestamp = clock["serverTime"]
            .as_u64()
            .context("Spotify sent no server time")?;
        let code = totp(secret.as_bytes(), timestamp);
        let token_result: Result<Token> = async {
            Ok(serde_json::from_slice(
                &body(
                    self.0
                        .http
                        .get("https://open.spotify.com/api/token")
                        .query(&[
                            ("reason", "transport"),
                            ("productType", "web-player"),
                            ("totp", &code),
                            ("totpServer", &code),
                            ("totpVer", &version.to_string()),
                        ])
                        .header("Cookie", format!("sp_dc={}", self.0.cookie))
                        .header("Origin", "https://open.spotify.com")
                        .header("Referer", "https://open.spotify.com/")
                        .send()
                        .await?,
                )
                .await?,
            )
            .context("invalid Spotify cookie exchange response")?)
        }
        .await;
        let token = match token_result {
            Ok(token) => token,
            Err(error) => {
                *self.0.material.lock().await = None;
                return Err(error);
            }
        };
        ensure!(
            !token.is_anonymous && !token.access_token.is_empty(),
            "Spotify cookie expired; sign in again"
        );
        let answer: Value = serde_json::from_slice(&body(self.0.http.post("https://clienttoken.spotify.com/v1/clienttoken")
            .json(&json!({"client_data": {"client_version": VERSION, "client_id": token.client_id,
                "js_sdk_data": {"device_brand": "unknown", "device_model": "unknown", "os": std::env::consts::OS,
                "os_version": "unknown", "device_id": self.0.device, "device_type": "computer"}}}))
            .send().await?).await?).context("invalid Spotify client token response")?;
        let client_token = answer["granted_token"]["token"]
            .as_str()
            .context("Spotify granted no client token")?
            .to_owned();
        *held = Some(Access {
            token,
            client_token,
        });
        Ok(held)
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        let bytes = self.request(Method::GET, path, None).await?;
        serde_json::from_slice(&bytes).context("invalid Spotify playback response")
    }
    pub async fn license(&self, challenge: Vec<u8>) -> Result<Vec<u8>> {
        self.request(
            Method::POST,
            "/widevine-license/v1/audio/license",
            Some(challenge),
        )
        .await
    }
    pub async fn auth_token(&self) -> Result<librespot_core::token::Token> {
        let held = self.access().await?;
        let access = held.as_ref().context("Spotify session is missing")?;
        let now = SystemTime::now();
        let expiry =
            UNIX_EPOCH + Duration::from_millis(access.token.access_token_expiration_timestamp_ms);
        Ok(librespot_core::token::Token {
            access_token: access.token.access_token.clone(),
            expires_in: expiry.duration_since(now).unwrap_or_default(),
            token_type: "Bearer".to_owned(),
            scopes: vec![],
            timestamp: now,
        })
    }
    pub async fn client_token(&self) -> Result<String> {
        let held = self.access().await?;
        Ok(held
            .as_ref()
            .context("Spotify session is missing")?
            .client_token
            .clone())
    }
    async fn request(
        &self,
        method: Method,
        path: &str,
        payload: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let mut headers = http::HeaderMap::new();
        if payload.is_some() {
            headers.insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            );
            headers.insert(
                http::header::ACCEPT,
                http::HeaderValue::from_static("application/octet-stream"),
            );
        }
        self.request_url(method, &format!("{PLAYER}{path}"), headers, payload)
            .await
    }
    pub async fn request_url(
        &self,
        method: Method,
        url: &str,
        mut headers: http::HeaderMap,
        payload: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        let url = reqwest::Url::parse(url).context("invalid Spotify request URL")?;
        ensure!(
            url.scheme() == "https"
                && url
                    .host_str()
                    .is_some_and(|host| host == "spotify.com" || host.ends_with(".spotify.com")),
            "unexpected Spotify request host"
        );
        headers.remove(http::header::AUTHORIZATION);
        headers.remove("client-token");
        for attempt in 0..2 {
            let held = self.access().await?;
            let access = held.as_ref().context("Spotify session is missing")?;
            let used_token = access.token.access_token.clone();
            let mut request = self
                .0
                .http
                .request(method.clone(), url.clone())
                .headers(headers.clone())
                .bearer_auth(&used_token)
                .header("client-token", &access.client_token)
                .header("app-platform", "WebPlayer")
                .header("spotify-app-version", VERSION)
                .header("Origin", "https://open.spotify.com")
                .header("Referer", "https://open.spotify.com/");
            if let Some(payload) = &payload {
                request = request.body(payload.clone());
            }
            drop(held);
            let response = request.send().await.map_err(reqwest::Error::without_url)?;
            if attempt == 0 && response.status() == reqwest::StatusCode::UNAUTHORIZED {
                let mut held = self.0.access.lock().await;
                invalidate(&mut held, &used_token);
                continue;
            }
            return body(response).await;
        }
        unreachable!("the final attempt returns its response")
    }
    pub async fn audio(&self, url: &str) -> Result<reqwest::Response> {
        let url = reqwest::Url::parse(url).context("invalid Spotify audio URL")?;
        ensure!(
            url.scheme() == "https"
                && url.host_str().is_some_and(|host| [
                    ".scdn.co",
                    ".spotifycdn.com",
                    ".spotifycdn.net"
                ]
                .iter()
                .any(|suffix| host.ends_with(suffix))),
            "unexpected Spotify audio host"
        );
        self.0
            .http
            .get(url)
            .header(reqwest::header::ACCEPT, "*/*")
            .send()
            .await
            .map_err(reqwest::Error::without_url)?
            .error_for_status()
            .map_err(reqwest::Error::without_url)
            .context("cannot fetch Spotify audio")
    }
}

fn invalidate(access: &mut Option<Access>, used_token: &str) {
    // A concurrent request may already have refreshed the token.
    if access
        .as_ref()
        .is_some_and(|access| access.token.access_token == used_token)
    {
        *access = None;
    }
}

fn decode_material(secrets: &BTreeMap<String, Vec<u8>>) -> Result<(u32, String)> {
    let (version, cipher) = secrets
        .iter()
        .filter_map(|(v, c)| v.parse::<u32>().ok().map(|v| (v, c)))
        .max_by_key(|(v, _)| *v)
        .context("Spotify TOTP dictionary is empty")?;
    ensure!(
        !cipher.is_empty() && cipher.len() <= 1024,
        "invalid Spotify TOTP material"
    );
    let secret = cipher
        .iter()
        .enumerate()
        .map(|(i, b)| (b ^ ((i % 33 + 9) as u8)).to_string())
        .collect();
    Ok((version, secret))
}

async fn body(mut response: reqwest::Response) -> Result<Vec<u8>> {
    ensure!(
        response.status().is_success(),
        "Spotify refused the request: HTTP {}",
        response.status()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        ensure!(
            bytes.len() + chunk.len() <= CEILING,
            "Spotify response is too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn cookie(input: &str) -> Result<String> {
    let input = input
        .trim()
        .strip_prefix("Cookie:")
        .unwrap_or(input.trim())
        .trim();
    let value = if input.contains('=') {
        input
            .split(';')
            .filter_map(|part| part.trim().split_once('='))
            .find_map(|(name, value)| (name == "sp_dc").then_some(value))
            .context("paste the sp_dc cookie value")?
    } else {
        input
    };
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_graphic() && b != b';') {
        bail!("invalid sp_dc cookie value");
    }
    Ok(value.to_owned())
}
fn totp(secret: &[u8], timestamp: u64) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let digest = hmac::sign(&key, &(timestamp / 30).to_be_bytes());
    let bytes = digest.as_ref();
    let at = usize::from(bytes[bytes.len() - 1] & 15);
    let code =
        u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) & 0x7fff_ffff;
    format!("{:06}", code % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_latest_material_and_rejects_empty_cipher() {
        let mut secrets = BTreeMap::from([
            ("9".to_owned(), vec![9]),
            ("10".to_owned(), vec![8, 8]),
            ("invalid".to_owned(), vec![0]),
        ]);
        assert_eq!(decode_material(&secrets).unwrap(), (10, "12".to_owned()));
        secrets.insert("11".to_owned(), vec![]);
        assert!(decode_material(&secrets).is_err());
    }

    #[test]
    fn stale_unauthorized_response_preserves_refreshed_token() {
        let mut access = Some(Access {
            token: Token {
                access_token: "new".to_owned(),
                access_token_expiration_timestamp_ms: 0,
                client_id: String::new(),
                is_anonymous: false,
            },
            client_token: String::new(),
        });
        invalidate(&mut access, "old");
        assert!(access.is_some());
        invalidate(&mut access, "new");
        assert!(access.is_none());
    }

    #[test]
    fn uses_rfc6238_six_digit_vectors() {
        for (time, expected) in [
            (59, "287082"),
            (1111111109, "081804"),
            (1234567890, "005924"),
        ] {
            assert_eq!(totp(b"12345678901234567890", time), expected);
        }
    }
    #[test]
    fn accepts_value_and_native_webview_header() {
        assert_eq!(cookie("abc-123").unwrap(), "abc-123");
        assert_eq!(
            cookie("other=x; sp_dc=abc-123; sp_key=y").unwrap(),
            "abc-123"
        );
        for bad in ["", "other=x", "sp_dc=", "abc\r\nInjected: yes", "abc def"] {
            assert!(cookie(bad).is_err());
        }
    }
}
