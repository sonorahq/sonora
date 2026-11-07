//! The framing between Sonora and its CDM host process, over the host's stdin and stdout.
//!
//! A request is an op byte, a little-endian `u32` payload length and the payload. An answer is
//! a status byte, a length and a payload, and a failure carries its message as UTF-8. The host
//! sends one answer before any request, saying whether the module opened. Its payload is the
//! host's [`VERSION`], so an app that finds a different build on disk refuses to talk to it.
//!
//! A decrypt payload is the IV length as a byte and the IV, the key id length as a `u32` and
//! the key id, the subsample count as a `u32` and that many clear and cipher `u32` pairs, then
//! the sample to the end of the frame. Its answer is the cleartext, exactly as long as the sample.

use std::io::{self, Read, Write};

/// What a host says it is in its first answer. Both sides are the same executable, so this only
/// differs when the file was replaced under a running app.
pub const VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "/2");

/// The largest payload either side accepts. A license or a sample is far smaller, so a length
/// past this means the two sides are out of step.
pub const CEILING: usize = 64 << 20;

/// Asks for a license challenge. The payload is the `pssh` box and the answer the challenge.
pub const CHALLENGE: u8 = 1;
/// Hands over a license. The payload is the license and the answer is empty.
pub const UPDATE: u8 = 2;
/// Decrypts one sample, laid out as the module docs describe.
pub const DECRYPT: u8 = 3;

/// The status of an answer that carries what was asked for.
pub const OK: u8 = 0;
/// The status of an answer that carries the reason a request failed.
pub const FAILED: u8 = 1;

/// Writes the tag and length that open a request or an answer.
pub fn put_header(to: &mut impl Write, tag: u8, len: usize) -> io::Result<()> {
    let len = u32::try_from(len)
        .ok()
        .filter(|&len| len as usize <= CEILING)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a frame is too long"))?;
    let [a, b, c, d] = len.to_le_bytes();
    to.write_all(&[tag, a, b, c, d])
}

/// Reads the tag and length that open a request or an answer. A clean end of the stream before
/// the first byte comes back as `UnexpectedEof`, which is how the host learns to exit.
pub fn take_header(from: &mut impl Read) -> io::Result<(u8, usize)> {
    let mut head = [0u8; 5];
    from.read_exact(&mut head)?;
    let [tag, a, b, c, d] = head;
    let len = u32::from_le_bytes([a, b, c, d]) as usize;
    if len > CEILING {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a frame claims {len} bytes"),
        ));
    }
    Ok((tag, len))
}

/// The length of a decrypt payload for these parts.
pub fn decrypt_len(iv: &[u8], key_id: &[u8], subs: &[(u32, u32)], sample: &[u8]) -> usize {
    1 + iv.len() + 4 + key_id.len() + 4 + subs.len() * 8 + sample.len()
}

/// Writes a decrypt payload without gathering it into one buffer first.
pub fn put_decrypt(
    to: &mut impl Write,
    iv: &[u8],
    key_id: &[u8],
    subs: &[(u32, u32)],
    sample: &[u8],
) -> io::Result<()> {
    if !matches!(iv.len(), 8 | 16) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid CENC IV length",
        ));
    }
    to.write_all(&[iv.len() as u8])?;
    to.write_all(iv)?;
    to.write_all(&(key_id.len() as u32).to_le_bytes())?;
    to.write_all(key_id)?;
    to.write_all(&(subs.len() as u32).to_le_bytes())?;
    for &(clear, cipher) in subs {
        to.write_all(&clear.to_le_bytes())?;
        to.write_all(&cipher.to_le_bytes())?;
    }
    to.write_all(sample)
}

/// A decrypt payload taken apart again, borrowing from the frame it came in.
pub struct Decrypt<'a> {
    pub iv: &'a [u8],
    pub key_id: &'a [u8],
    pub sample: &'a [u8],
}

/// Splits a decrypt payload, filling `subs` with the flattened subsample counts. `None` when the
/// payload is shorter than the lengths inside it say.
pub fn split_decrypt<'a>(payload: &'a [u8], subs: &mut Vec<u32>) -> Option<Decrypt<'a>> {
    let (&iv_len, rest) = payload.split_first()?;
    if !matches!(iv_len, 8 | 16) {
        return None;
    }
    let (iv, rest) = rest.split_at_checked(usize::from(iv_len))?;
    let (key_len, rest) = rest.split_at_checked(4)?;
    let key_len = u32::from_le_bytes(key_len.try_into().ok()?) as usize;
    let (key_id, rest) = rest.split_at_checked(key_len)?;
    let (count, rest) = rest.split_at_checked(4)?;
    let count = u32::from_le_bytes(count.try_into().ok()?) as usize;
    let (pairs, sample) = rest.split_at_checked(count.checked_mul(8)?)?;
    subs.clear();
    subs.extend(
        pairs
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&word| u32::from_le_bytes(word)),
    );
    Some(Decrypt { iv, key_id, sample })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decrypt_frames_preserve_both_iv_lengths() {
        for length in [8, 16] {
            let iv = vec![9; length];
            let key = [3; 16];
            let pairs = [(4, 12)];
            let sample = [7; 16];
            let mut frame = Vec::new();
            put_decrypt(&mut frame, &iv, &key, &pairs, &sample).unwrap();
            assert_eq!(frame.len(), decrypt_len(&iv, &key, &pairs, &sample));
            let mut subs = Vec::new();
            let decoded = split_decrypt(&frame, &mut subs).unwrap();
            assert_eq!(decoded.iv, iv);
            assert_eq!(decoded.key_id, key);
            assert_eq!(decoded.sample, sample);
            assert_eq!(subs, [4, 12]);
            for end in 0..frame.len() - sample.len() {
                assert!(split_decrypt(&frame[..end], &mut subs).is_none());
            }
        }
    }

    #[test]
    fn refuses_invalid_iv_lengths() {
        for length in [0, 7, 9, 15, 17] {
            let mut frame = Vec::new();
            assert!(put_decrypt(&mut frame, &vec![0; length], &[], &[], &[]).is_err());
            assert!(frame.is_empty());
            assert!(split_decrypt(&[length as u8; 64], &mut Vec::new()).is_none());
        }
    }
}
