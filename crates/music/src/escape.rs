use std::borrow::Cow;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// The unreserved characters of RFC 3986, the only ones a url carries as they are.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The longest reference `unescape` looks for, `&#x10FFFF;` included, so a stray `&` in a
/// title costs no more than a short scan.
const LONGEST_ENTITY: usize = 10;

/// Percent-encodes a value for one path segment or one query value, the way
/// `encodeURIComponent` does. Every runtime value pasted into a url goes through here, since
/// usernames, queries and even provider ids may carry `/`, `&`, spaces or non-ASCII letters.
pub fn component(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

/// Decodes the HTML character references some catalogs store in plain-text fields, such as
/// `J&#039;ai` or `Body &amp; Soul`. Numeric references and the common named ones are
/// decoded, and anything else, a lone `&` included, is kept as written.
pub fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        let reference = rest
            .char_indices()
            .take(LONGEST_ENTITY + 1)
            .find(|&(_, character)| character == ';')
            .and_then(|(end, _)| Some((character(&rest[1..end])?, end)));
        match reference {
            Some((character, end)) => {
                decoded.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    Cow::Owned(decoded)
}

/// The character a reference names, given what sits between its `&` and `;`.
fn character(name: &str) -> Option<char> {
    if let Some(number) = name.strip_prefix('#') {
        let code = match number.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => number.parse().ok()?,
        };
        return char::from_u32(code).filter(|&character| character != '\0');
    }
    Some(match name {
        "amp" => '&',
        "quot" => '"',
        "apos" => '\'',
        "lt" => '<',
        "gt" => '>',
        "nbsp" => '\u{a0}',
        _ => return None,
    })
}
