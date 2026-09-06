use anyhow::{Context as _, Result, bail};
use blowfish::Blowfish;
use cbc::Decryptor;
use cipher::{BlockDecryptMut, KeyIvInit, block_padding::NoPadding};
use md5::{Digest, Md5};

const CHUNK: usize = 2048;
const SECRET: &[u8] = b"g4el58wc0zvf9na1";
const IV: [u8; 8] = [0, 1, 2, 3, 4, 5, 6, 7];

/// Decrypts a Deezer `BF_CBC_STRIPE` body for `track_id`.
pub fn unlock(track_id: &str, data: &[u8]) -> Result<Vec<u8>> {
    let key = blowfish_key(track_id);
    let mut out = Vec::with_capacity(data.len());
    for (index, chunk) in data.chunks(CHUNK).enumerate() {
        if index % 3 == 0 && chunk.len() == CHUNK {
            out.extend(decrypt(&key, chunk)?);
        } else {
            out.extend_from_slice(chunk);
        }
    }
    Ok(out)
}

fn blowfish_key(track_id: &str) -> [u8; 16] {
    let digest = Md5::digest(track_id.as_bytes());
    let hex: Vec<u8> = digest
        .iter()
        .flat_map(|byte| format!("{byte:02x}").into_bytes())
        .collect();
    let mut key = [0u8; 16];
    for i in 0..16 {
        key[i] = hex[i] ^ hex[i + 16] ^ SECRET[i];
    }
    key
}

fn decrypt(key: &[u8; 16], chunk: &[u8]) -> Result<Vec<u8>> {
    let mut buf = chunk.to_vec();
    Decryptor::<Blowfish>::new_from_slices(key, &IV)
        .context("cannot start blowfish")?
        .decrypt_padded_mut::<NoPadding>(&mut buf)
        .map_err(|error| anyhow::anyhow!("cannot decrypt audio: {error}"))?;
    if buf.len() != CHUNK {
        bail!("decrypted chunk was truncated");
    }
    Ok(buf)
}
