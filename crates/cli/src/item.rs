use std::path::{self, PathBuf};

/// Whether a launch argument is an address rather than a path, so one no provider reads is
/// never tried as a file.
pub fn is_address(item: &str) -> bool {
    !item.starts_with("file://") && (item.contains("://") || item.starts_with("spotify:"))
}

/// An item as the running Sonora should read it. A path is made absolute against this
/// process's folder, since the running one sits in another; an address is left alone.
pub fn anchored(item: String) -> String {
    if is_address(&item) {
        return item;
    }
    match path::absolute(local_path(&item)) {
        Ok(path) => path.to_string_lossy().into_owned(),
        Err(_) => item,
    }
}

/// A bare filesystem path, or a `file://` URI decoded back into one — the two shapes an "Open
/// With" launch or drop hands us across platforms.
pub fn local_path(arg: &str) -> PathBuf {
    match arg.strip_prefix("file://") {
        Some(rest) => PathBuf::from(file_uri_path(rest)),
        None => PathBuf::from(arg),
    }
}

/// A `file://` URI body turned into a filesystem path. Windows `file:///C:/…`
/// keeps a slash in front of the drive, which is not a path the OS will open.
fn file_uri_path(rest: &str) -> String {
    let decoded = percent_decode(rest);
    let path = decoded
        .strip_prefix("localhost")
        .or_else(|| decoded.strip_prefix("LOCALHOST"))
        .unwrap_or(decoded.as_str());
    #[cfg(windows)]
    {
        if let Some(drive) = path.strip_prefix('/')
            && drive
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && drive.as_bytes().get(1) == Some(&b':')
        {
            return drive.to_owned();
        }
    }
    path.to_owned()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percent_before_a_multibyte_letter_stays_in_the_path() {
        assert_eq!(percent_decode("%가나"), "%가나");
        assert_eq!(percent_decode("%a가"), "%a가");
        assert_eq!(percent_decode("a%20b%EA%B0%80"), "a b가");
    }
}
