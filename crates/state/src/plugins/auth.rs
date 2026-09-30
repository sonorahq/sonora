use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// Token bytes: 256 bits of OS randomness, 43 base64 characters.
const TOKEN_BYTES: usize = 32;

/// The session's bearer credential, minted on first enable and held only in memory. Never
/// serialized, logged or written anywhere. The manager forgets it once nothing serves
/// it, so a fresh enable mints a fresh token.
#[derive(Clone, Default)]
pub struct Auth {
    token: Arc<RwLock<Option<String>>>,
}

impl Auth {
    /// Starts without a token. The first plugin enable mints one.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current token, while any plugin serves.
    pub fn token(&self) -> Option<String> {
        self.read().clone()
    }

    /// Forgets the current token. The next enable mints a fresh one.
    pub fn clear(&self) {
        *self.write() = None;
    }

    /// Returns the current token, minting the session's first one when none exists. `None`
    /// means the OS would not hand out randomness, and the plugin must not start.
    pub fn ensure(&self) -> Option<String> {
        if let Some(token) = self.read().clone() {
            return Some(token);
        }
        let token = mint()?;
        *self.write() = Some(token.clone());
        Some(token)
    }

    /// Replaces the current token and returns the new one. `None` leaves the old token in
    /// place when the OS would not hand out randomness.
    pub fn rotate(&self) -> Option<String> {
        let token = mint()?;
        *self.write() = Some(token.clone());
        Some(token)
    }

    /// Whether `header` carries the current token as `Bearer <token>`.
    pub fn check(&self, header: Option<&str>) -> bool {
        let Some(header) = header else {
            return false;
        };
        let Some(presented) = header.strip_prefix("Bearer ") else {
            return false;
        };
        self.read()
            .as_deref()
            .is_some_and(|expected| constant_time_eq(expected, presented))
    }

    fn read(&self) -> RwLockReadGuard<'_, Option<String>> {
        self.token
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Option<String>> {
        self.token
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Mints one bearer token from the OS random source.
fn mint() -> Option<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).ok()?;
    Some(URL_SAFE_NO_PAD.encode(bytes))
}

/// Compares two tokens without leaking their bytes through timing. Tokens are fixed length,
/// so a length mismatch fails up front.
fn constant_time_eq(expected: &str, presented: &str) -> bool {
    let (expected, presented) = (expected.as_bytes(), presented.as_bytes());
    if expected.len() != presented.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left, right) in expected.iter().zip(presented) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bearer(token: &str) -> String {
        format!("Bearer {token}")
    }

    #[test]
    fn nothing_authenticates_before_the_first_enable() {
        let auth = Auth::new();

        assert_eq!(auth.token(), None);
        assert!(!auth.check(None));
        assert!(!auth.check(Some("Bearer anything")));
    }

    #[test]
    fn ensure_mints_one_url_safe_token() {
        let auth = Auth::new();
        let token = auth.ensure().expect("a token is minted");

        assert_eq!(token.len(), 43);
        assert!(
            token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        );
        assert_eq!(auth.ensure().as_deref(), Some(token.as_str()));
        assert!(auth.check(Some(&bearer(&token))));
    }

    #[test]
    fn rotate_replaces_the_token() {
        let auth = Auth::new();
        let before = auth.ensure().expect("a token is minted");
        let after = auth.rotate().expect("rotation mints");

        assert_ne!(before, after);
        assert_eq!(auth.token().as_deref(), Some(after.as_str()));
        assert!(!auth.check(Some(&bearer(&before))));
        assert!(auth.check(Some(&bearer(&after))));
    }

    #[test]
    fn minted_tokens_differ() {
        let first = Auth::new().ensure().expect("a token is minted");
        let second = Auth::new().ensure().expect("a token is minted");

        assert_ne!(first, second);
    }

    #[test]
    fn check_rejects_anything_but_a_bearer_token() {
        let auth = Auth::new();
        let token = auth.ensure().expect("a token is minted");

        for header in [
            "Bearer".to_owned(),
            "Bearer ".to_owned(),
            format!("Bearer  {token}"),
            format!("bearer {token}"),
            format!("Basic {token}"),
            bearer("wrong"),
            token.clone(),
        ] {
            assert!(!auth.check(Some(&header)), "{header:?} is rejected");
        }
    }
}
