use std::path::PathBuf;

use anyhow::{Context as _, Result};
use opensubsonic::Auth;
use serde::{Deserialize, Serialize};

use crate::credentials;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub server: String,
    pub username: String,
    pub password: String,
    /// The token and salt every cover url is signed with. Made once at sign-in and kept, so a
    /// cover keeps one url across launches and the image caches can hold it.
    #[serde(default)]
    pub signature: Option<Signature>,
    /// The library the account is scoped to, or `None` for the whole server. Accounts signed
    /// in before a library could be chosen have no field here and keep reading everything.
    #[serde(default)]
    pub library: Option<Library>,
}

/// One library the server offers, kept beside the account so a launch can name the scope
/// before the server is asked, and so a library the server no longer lists is still reported
/// by name rather than silently widened back to everything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Library {
    pub id: i64,
    /// The server's name for it, when it has one.
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub token: String,
    pub salt: String,
}

/// Signs once with a fresh salt, the way the server expects each request to be signed.
pub fn sign(username: &str, password: &str) -> Signature {
    let mut signature = Signature {
        token: String::new(),
        salt: String::new(),
    };
    for (key, value) in Auth::token(username, password).params() {
        match key {
            "t" => signature.token = value,
            "s" => signature.salt = value,
            _ => {}
        }
    }
    signature
}

fn path() -> PathBuf {
    credentials::dir("subsonic").join(credentials::FILE)
}

pub fn normalize_server(raw: &str) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        anyhow::bail!("the server address is empty");
    }
    let with_scheme = match trimmed.contains("://") {
        true => trimmed.to_owned(),
        false => format!("http://{trimmed}"),
    };
    Ok(with_scheme)
}

pub fn load() -> Option<Credentials> {
    let bytes = std::fs::read(path()).ok()?;
    let mut credentials: Credentials = serde_json::from_slice(&bytes).ok()?;
    credentials.server = normalize_server(&credentials.server).ok()?;
    match credentials.username.is_empty() {
        true => None,
        false => Some(credentials),
    }
}

pub fn store(stored: &Credentials) -> Result<()> {
    let bytes =
        serde_json::to_vec_pretty(stored).context("cannot serialize subsonic credentials")?;
    credentials::write(&path(), &bytes).context("cannot store subsonic credentials")
}

pub fn forget() {
    credentials::remove(&path());
}

#[cfg(test)]
mod tests {
    use super::{Credentials, Library, Signature};

    /// Accounts written before libraries could be chosen carry no `library` field. They must
    /// still load, scoped to the whole server as they always were.
    #[test]
    fn credentials_without_a_library_still_load() {
        let stored = r#"{
            "server": "https://music.example.com",
            "username": "ada",
            "password": "secret",
            "signature": { "token": "abc", "salt": "xyz" }
        }"#;
        let credentials: Credentials = serde_json::from_str(stored).unwrap();
        assert_eq!(credentials.server, "https://music.example.com");
        assert_eq!(
            credentials.signature.as_ref().map(|s| s.token.as_str()),
            Some("abc")
        );
        assert_eq!(credentials.library, None);
    }

    /// Even older accounts have no `signature` field either.
    #[test]
    fn credentials_without_signature_or_library_still_load() {
        let stored = r#"{
            "server": "https://music.example.com",
            "username": "ada",
            "password": "secret"
        }"#;
        let credentials: Credentials = serde_json::from_str(stored).unwrap();
        assert_eq!(credentials.signature, None);
        assert_eq!(credentials.library, None);
    }

    #[test]
    fn a_scoped_account_keeps_its_library() {
        let credentials = Credentials {
            server: "https://music.example.com".to_owned(),
            username: "ada".to_owned(),
            password: "secret".to_owned(),
            signature: Some(Signature {
                token: "abc".to_owned(),
                salt: "xyz".to_owned(),
            }),
            library: Some(Library {
                id: 7,
                name: Some("Classical".to_owned()),
            }),
        };
        let bytes = serde_json::to_vec(&credentials).unwrap();
        let read: Credentials = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            read.library,
            Some(Library {
                id: 7,
                name: Some("Classical".to_owned()),
            })
        );
    }

    /// A folder the server never named is kept by id alone.
    #[test]
    fn a_library_needs_no_name() {
        let stored = r#"{
            "server": "https://music.example.com",
            "username": "ada",
            "password": "secret",
            "library": { "id": 3 }
        }"#;
        let credentials: Credentials = serde_json::from_str(stored).unwrap();
        assert_eq!(credentials.library, Some(Library { id: 3, name: None }));
    }
}
