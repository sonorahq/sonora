//! Decryption of Yandex Music's `encraw` transport: the whole file is AES-128 in counter mode,
//! keyed per file by the key `get-file-info` hands out beside its url, with the counter starting
//! at zero. A counter mode cipher is a keystream, so every chunk is decrypted where it lands and
//! nothing is held back.

use aes::Aes128;
use anyhow::{Context as _, Result, bail};
use ctr::cipher::{KeyIvInit, StreamCipher};

type Cipher = ctr::Ctr128BE<Aes128>;

/// The 16-byte key of one file.
pub type Key = [u8; 16];

/// Reads the hex key `get-file-info` names.
pub fn key(hex: &str) -> Result<Key> {
    let hex = hex.trim();
    if hex.len() != 32 {
        bail!("the stream key is {} characters, not 32", hex.len());
    }
    let mut key = [0u8; 16];
    for (at, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * at..2 * at + 2], 16)
            .context("the stream key is not hex")?;
    }
    Ok(key)
}

/// The body of a Yandex track, decrypted as it arrives when it came encrypted.
pub struct Sealed {
    cipher: Option<Cipher>,
}

impl Sealed {
    /// A body decrypted with `key`, or passed on as it is without one, as a `raw` file comes.
    pub fn new(key: Option<&Key>) -> Self {
        Self {
            cipher: key.map(|key| Cipher::new(key.into(), &[0u8; 16].into())),
        }
    }
}

impl crate::stream::Body for Sealed {
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        let start = out.len();
        out.extend_from_slice(chunk);
        if let Some(cipher) = &mut self.cipher {
            cipher.apply_keystream(&mut out[start..]);
        }
    }

    fn confidential(&self) -> bool {
        self.cipher.is_some()
    }
}

#[cfg(test)]
mod tests {
    use crate::stream::Body as _;

    use super::*;

    #[test]
    fn a_key_reads_from_hex() {
        let key = key("000102030405060708090a0b0c0d0e0F").unwrap();
        assert_eq!(key[1], 1);
        assert_eq!(key[15], 15);
        assert!(super::key("abc").is_err());
        assert!(super::key("zz0102030405060708090a0b0c0d0e0f").is_err());
    }

    /// The keystream carries on across chunks, so a body split anywhere decrypts the same as a
    /// body that arrives whole.
    #[test]
    fn chunks_decrypt_like_the_whole() {
        let key = [7u8; 16];
        let plain: Vec<u8> = (0..1000u32).map(|at| at as u8).collect();
        let mut sealed = plain.clone();
        Cipher::new(&key.into(), &[0u8; 16].into()).apply_keystream(&mut sealed);

        let mut body = Sealed::new(Some(&key));
        let mut opened = Vec::new();
        for chunk in sealed.chunks(37) {
            let mut out = Vec::new();
            body.feed(chunk, &mut out);
            opened.extend(out);
        }
        assert_eq!(opened, plain);
    }
}
