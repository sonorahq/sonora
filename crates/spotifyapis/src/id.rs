//! Spotify track URIs and binary and base62 catalogue IDs.

pub const TRACK_PREFIX: &str = "spotify:track:";

/// Returns the ID from a nonempty Spotify track URI.
pub fn track_id(uri: &str) -> Option<&str> {
    uri.strip_prefix(TRACK_PREFIX).filter(|id| !id.is_empty())
}

pub fn track_uri(id: &str) -> String {
    format!("{TRACK_PREFIX}{id}")
}

/// A borrowed catalogue ID or the inline base-62 form of a binary GID.
pub enum CatalogueId<'a> {
    Borrowed(&'a str),
    Encoded([u8; 22]),
}

impl CatalogueId<'_> {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Borrowed(id) => id.as_bytes(),
            Self::Encoded(id) => id,
        }
    }

    pub fn into_owned(self) -> String {
        match self {
            Self::Borrowed(id) => id.to_owned(),
            Self::Encoded(id) => String::from_utf8_lossy(&id).into_owned(),
        }
    }
}

/// Encodes a binary catalogue ID as Spotify’s 22-character text ID.
pub fn base62(gid: &[u8]) -> Option<String> {
    String::from_utf8(base62_bytes(gid)?.to_vec()).ok()
}

/// Encodes a 16-byte catalogue ID for callers that only need to compare its text.
pub fn base62_bytes(gid: &[u8]) -> Option<[u8; 22]> {
    const DIGITS: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut value = u128::from_be_bytes(gid.try_into().ok()?);
    let mut encoded = [b'0'; 22];
    for digit in encoded.iter_mut().rev() {
        *digit = DIGITS[(value % 62) as usize];
        value /= 62;
    }
    Some(encoded)
}

/// Decodes the 22-character Spotify ID into the 16 bytes used by the access point.
pub fn raw_id(id: &str) -> Option<[u8; 16]> {
    if id.len() != 22 {
        return None;
    }
    let value = id.bytes().try_fold(0_u128, |value, byte| {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'z' => byte - b'a' + 10,
            b'A'..=b'Z' => byte - b'A' + 36,
            _ => return None,
        };
        value.checked_mul(62)?.checked_add(u128::from(digit))
    })?;
    Some(value.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogue_ids_round_trip_and_reject_overflow() {
        for gid in [[0; 16], [0x80; 16], [0xff; 16]] {
            assert_eq!(raw_id(&base62(&gid).unwrap()), Some(gid));
        }
        assert_eq!(base62(&[0; 16]).as_deref(), Some("0000000000000000000000"));
        assert_eq!(raw_id("ZZZZZZZZZZZZZZZZZZZZZZ"), None);
        assert_eq!(raw_id("100000000000000000000!"), None);
    }
}
