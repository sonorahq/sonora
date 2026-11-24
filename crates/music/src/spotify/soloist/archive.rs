//! Extracts the Soloist executable from release archives.

use std::io::Read as _;

use anyhow::{Context as _, Result, bail, ensure};

/// Maximum decompressed archive size.
const LIMIT: usize = 256 * 1024 * 1024;

/// Tar header size and file alignment.
const BLOCK: usize = 512;

/// Extracts the named file using the format indicated by the archive name.
pub(crate) fn extract(archive: &[u8], name: &str, wanted: &str) -> Result<Vec<u8>> {
    if name.ends_with(".zip") {
        unzip(archive, wanted)
    } else {
        untar(archive, wanted)
    }
}

/// One regular file out of a gzipped tar, by its name without folders.
fn untar(archive: &[u8], wanted: &str) -> Result<Vec<u8>> {
    let tar = unpack(
        flate2::read::GzDecoder::new(archive),
        "cannot unpack the soloist archive",
    )?;
    let mut at = 0;
    while let Some(header) = tar.get(at..at + BLOCK) {
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name = text(&header[..100]);
        let size = usize::from_str_radix(text(&header[124..136]).trim(), 8)
            .context("the soloist archive is damaged")?;
        let regular = matches!(header[156], b'0' | 0);
        let start = at + BLOCK;
        if regular && name.rsplit('/').next() == Some(wanted) {
            return tar
                .get(start..start + size)
                .map(<[u8]>::to_vec)
                .context("the soloist archive is cut short");
        }
        at = start + size.div_ceil(BLOCK) * BLOCK;
    }
    bail!("{wanted} is not in the soloist archive")
}

/// Reads a named ZIP entry stored or compressed with deflate.
fn unzip(zip: &[u8], wanted: &str) -> Result<Vec<u8>> {
    const END: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const CENTRAL: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const LOCAL: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];

    let end = (0..zip.len().saturating_sub(21))
        .rev()
        .take(usize::from(u16::MAX) + 1)
        .find(|&at| zip[at..].starts_with(&END))
        .context("cannot find the zip directory")?;
    let count = usize::from(u16_at(zip, end + 10)?);
    let mut at = u32_at(zip, end + 16)? as usize;
    for _ in 0..count {
        ensure!(
            zip.get(at..at + 4) == Some(&CENTRAL),
            "the zip directory is damaged"
        );
        let method = u16_at(zip, at + 10)?;
        let packed = u32_at(zip, at + 20)? as usize;
        let name = usize::from(u16_at(zip, at + 28)?);
        let extra = usize::from(u16_at(zip, at + 30)?);
        let comment = usize::from(u16_at(zip, at + 32)?);
        let local = u32_at(zip, at + 42)? as usize;
        let entry = zip
            .get(at + 46..at + 46 + name)
            .context("the zip directory is damaged")?;
        if text(entry).rsplit('/').next() == Some(wanted) {
            ensure!(
                zip.get(local..local + 4) == Some(&LOCAL),
                "the zip entry is damaged"
            );
            let name = usize::from(u16_at(zip, local + 26)?);
            let extra = usize::from(u16_at(zip, local + 28)?);
            let start = local + 30 + name + extra;
            let data = zip
                .get(start..start + packed)
                .context("the zip entry is cut short")?;
            return match method {
                0 => Ok(data.to_vec()),
                8 => unpack(
                    flate2::read::DeflateDecoder::new(data),
                    "cannot inflate the zip entry",
                ),
                other => bail!("the zip entry uses compression method {other}"),
            };
        }
        at += 46 + name + extra + comment;
    }
    bail!("{wanted} is not in the soloist archive")
}

fn unpack(reader: impl std::io::Read, context: &'static str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    reader
        .take(LIMIT as u64 + 1)
        .read_to_end(&mut out)
        .context(context)?;
    ensure!(out.len() <= LIMIT, "the soloist archive is too large");
    Ok(out)
}

/// A header field as text, up to its first NUL.
fn text(field: &[u8]) -> &str {
    let end = field
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).unwrap_or("")
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(field_at(bytes, at)?))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(field_at(bytes, at)?))
}

fn field_at<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N]> {
    bytes
        .get(at..at + N)
        .and_then(|field| field.try_into().ok())
        .context("the zip is cut short")
}
