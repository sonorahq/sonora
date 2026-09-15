//! Decryption of Deezer's `BF_CBC_STRIPE` audio: the stream is split into 2048-byte blocks
//! and only every third block (0, 3, 6, …) is encrypted with Blowfish CBC, the cipher reset
//! per block against a fixed IV. The per-track key derives from the track id and a master
//! secret extracted at runtime from Deezer's own web player bundle.

use anyhow::{Context as _, Result, bail};
use blowfish::Blowfish;
use blowfish::cipher::{BlockDecryptMut, InnerIvInit, KeyInit, block_padding::NoPadding};
use md5::{Digest, Md5};

/// Deezer's encryption block size.
pub const BLOCK: usize = 2048;

/// Every third block is encrypted.
const STRIPE: usize = 3;

/// The fixed IV of the BF_CBC_STRIPE scheme.
const IV: [u8; 8] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07];

/// A page of the web player; the script bundle it names carries the master secret.
const WEB_PLAYER: &str = "https://www.deezer.com/en/channels/explore/";

/// The MD5 a correctly reassembled master secret hashes to.
const SECRET_MD5: &str = "7ebf40da848f4a0fb3cc56ddbe6c2d09";

/// The last secret the extraction above produced, kept so a bundle redesign degrades to a
/// warning instead of killing playback.
const SECRET_FALLBACK: &[u8; 16] = b"g4el58wc0zvf9na1";

/// The 16-byte master secret shared by every track.
pub type Secret = [u8; 16];

/// Derives the Blowfish key of one track: the two halves of the MD5 hex of the track id,
/// XORed against each other and against the master secret.
pub fn track_key(track_id: &str, secret: &Secret) -> Secret {
    let hash = format!("{:x}", Md5::digest(track_id.as_bytes()));
    let hash = hash.as_bytes();
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = hash[i] ^ hash[i + 16] ^ secret[i];
    }
    key
}

/// One track's Blowfish, keyed once so the key schedule is not rebuilt for every stripe.
/// Each encrypted block still starts its CBC chain afresh from the fixed IV.
pub struct Cipher {
    blowfish: Blowfish,
}

impl Cipher {
    pub fn new(key: &Secret) -> Self {
        let blowfish =
            Blowfish::new_from_slice(key).expect("a 16-byte key is within blowfish's range");
        Self { blowfish }
    }

    /// Decrypts one block in place if its index lands on an encrypted stripe. Partial blocks
    /// (only possible at the very end of a track) arrive unencrypted.
    pub fn decrypt_block(&self, block: &mut [u8], index: u64) {
        if !index.is_multiple_of(STRIPE as u64) || block.len() < BLOCK {
            return;
        }
        let decryptor =
            cbc::Decryptor::<Blowfish>::inner_iv_init(self.blowfish.clone(), &IV.into());
        if let Err(error) = decryptor.decrypt_padded_mut::<NoPadding>(block) {
            log::warn!("deezer: cannot decrypt block {index}: {error}");
        }
    }
}

/// The body of a Deezer track: every complete stripe decrypted as it lands, so what the
/// buffer holds is always in the clear.
///
/// A partial block is kept back until the rest of it arrives, because only whole blocks are
/// encrypted, and whatever is left at the end of the track is appended as it is.
pub struct Striped {
    cipher: Cipher,
    /// The tail of the last chunk, short of a whole block.
    pending: Vec<u8>,
    /// How many blocks have gone by, which is what decides an encrypted stripe.
    blocks: u64,
}

impl Striped {
    pub fn new(key: &Secret) -> Self {
        Self {
            cipher: Cipher::new(key),
            pending: Vec::with_capacity(2 * BLOCK),
            blocks: 0,
        }
    }
}

impl crate::stream::Body for Striped {
    fn feed(&mut self, chunk: &[u8], out: &mut Vec<u8>) {
        self.pending.extend_from_slice(chunk);
        while self.pending.len() >= BLOCK {
            let mut block: Vec<u8> = self.pending.drain(..BLOCK).collect();
            self.cipher.decrypt_block(&mut block, self.blocks);
            self.blocks += 1;
            out.extend_from_slice(&block);
        }
    }

    fn flush(&mut self, out: &mut Vec<u8>) {
        // The tail is a partial block, and partial blocks are never encrypted.
        out.extend_from_slice(&self.pending);
        self.pending.clear();
    }
}

/// Fetches the master secret from the web player's script bundle: two URL-encoded 8-byte hex
/// arrays, reversed and interleaved, validated against a known MD5. Falls back to the last
/// known value when the bundle no longer matches the extraction, so a bundle redesign
/// degrades to a warning instead of killing playback.
pub async fn secret(http: &reqwest::Client) -> Secret {
    match fetch_secret(http).await {
        Ok(secret) => secret,
        Err(error) => {
            log::warn!(
                "deezer: cannot extract the stream secret ({error:#}); using the last known one"
            );
            *SECRET_FALLBACK
        }
    }
}

async fn fetch_secret(http: &reqwest::Client) -> Result<Secret> {
    let html = http
        .get(WEB_PLAYER)
        .send()
        .await
        .context("cannot reach the deezer web player")?
        .error_for_status()
        .context("the deezer web player refused the visit")?
        .text()
        .await
        .context("cannot read the deezer web player page")?;

    let bundle = find_bundle(&html).context("the web player names no script bundle")?;
    let source = http
        .get(bundle.as_str())
        .send()
        .await
        .context("cannot fetch the web player bundle")?
        .error_for_status()
        .context("the web player bundle refused the visit")?
        .text()
        .await
        .context("cannot read the web player bundle")?;

    let first = find_half(&source, 0x61, 0x67).context("the bundle holds no first key half")?;
    let second = find_half(&source, 0x31, 0x34).context("the bundle holds no second key half")?;

    let mut secret = [0u8; 16];
    for i in 0..8 {
        secret[i * 2] = first[i];
        secret[i * 2 + 1] = second[i];
    }
    let hash = format!("{:x}", Md5::digest(secret));
    if hash != SECRET_MD5 {
        bail!("the extracted stream secret fails validation ({hash})");
    }
    Ok(secret)
}

/// The `app-web` script url in the player page, without pulling in a regex engine.
fn find_bundle(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(at) = rest.find("app-web") {
        // walk back to the start of the url
        let head = &rest[..at];
        let Some(from) = head.rfind("https://") else {
            rest = &rest[at + 7..];
            continue;
        };
        let tail = &rest[from..];
        let Some(end) = tail.find('"') else {
            rest = &rest[at + 7..];
            continue;
        };
        let url = &tail[..end];
        if url.ends_with(".js") {
            return Some(url.to_owned());
        }
        rest = &rest[at + 7..];
    }
    None
}

/// One 8-byte half of the secret, found in the bundle as a URL-encoded hex array that starts
/// with `first` and ends with `last`: `0x61%2C…%2C0x67`. Returned reversed.
fn find_half(source: &str, first: u8, last: u8) -> Option<[u8; 8]> {
    let needle = format!("0x{first:02x}%2C");
    let terminator = format!("0x{last:02x}");
    let mut rest = source;
    while let Some(at) = rest.find(&needle) {
        let candidate = &rest[at..];
        let bytes = parse_half(candidate, &terminator);
        rest = &rest[at + needle.len()..];
        if let Some(bytes) = bytes {
            return Some(bytes);
        }
    }
    None
}

/// Parses `0xNN%2C…` up to and including `terminator` into 8 reversed bytes. None when the
/// window is not exactly 8 bytes.
fn parse_half(candidate: &str, terminator: &str) -> Option<[u8; 8]> {
    let end = candidate.find(terminator)? + terminator.len();
    if end > 8 * "0xNN%2C".len() {
        return None;
    }
    let mut bytes = Vec::with_capacity(8);
    for part in candidate[..end].split("%2C") {
        let hex = part.strip_prefix("0x")?;
        bytes.push(u8::from_str_radix(hex, 16).ok()?);
    }
    bytes.reverse();
    <[u8; 8]>::try_from(bytes.as_slice()).ok()
}
