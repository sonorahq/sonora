//! The edit list of an MP4, which is how an AAC encode says where the music actually starts.
//!
//! Every AAC encoder writes priming samples before the first real one, and an `elst` box names
//! the part that should be heard. Trimming to it is what makes one track end where the next
//! begins, so both providers whose tracks are MP4 read it: YouTube Music and Apple Music.

use std::time::Duration;

/// What to drop from the front of a decode, and how much of the rest is music.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    pub skip: Duration,
    pub take: Option<Duration>,
}

pub fn from_mp4(data: &[u8]) -> Option<Trim> {
    let moov = child(data, b"moov")?;
    let movie = timescale(child(moov, b"mvhd")?)?;
    let trak = child(moov, b"trak")?;
    let media = timescale(child(child(trak, b"mdia")?, b"mdhd")?)?;

    let elst = child(child(trak, b"edts")?, b"elst")?;
    let (segment, at) = edit(elst)?;

    Some(Trim {
        skip: Duration::from_secs_f64(at as f64 / media as f64),
        take: (segment > 0).then(|| Duration::from_secs_f64(segment as f64 / movie as f64)),
    })
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut at = 0usize;
    while at + 8 <= data.len() {
        let size = u32::from_be_bytes(data[at..at + 4].try_into().ok()?) as u64;
        let name = &data[at + 4..at + 8];
        let (head, size) = match size {
            1 => {
                let large = data.get(at + 8..at + 16)?;
                (16usize, u64::from_be_bytes(large.try_into().ok()?))
            }
            0 => (8usize, (data.len() - at) as u64),
            _ => (8usize, size),
        };
        let size = usize::try_from(size).ok()?;
        if size < head || at + size > data.len() {
            return None;
        }
        if name == kind {
            return Some(&data[at + head..at + size]);
        }
        at += size;
    }
    None
}

fn timescale(header: &[u8]) -> Option<u64> {
    let at = match header.first()? {
        1 => 20,
        _ => 12,
    };
    let field = header.get(at..at + 4)?;
    let scale = u32::from_be_bytes(field.try_into().ok()?);
    (scale > 0).then_some(scale as u64)
}

fn edit(elst: &[u8]) -> Option<(u64, u64)> {
    let version = *elst.first()?;
    let count = u32::from_be_bytes(elst.get(4..8)?.try_into().ok()?);
    let (width, mut at) = match version {
        1 => (20usize, 8usize),
        _ => (12usize, 8usize),
    };

    for _ in 0..count {
        let entry = elst.get(at..at + width)?;
        let (segment, media) = match version {
            1 => (
                u64::from_be_bytes(entry.get(0..8)?.try_into().ok()?),
                i64::from_be_bytes(entry.get(8..16)?.try_into().ok()?),
            ),
            _ => (
                u32::from_be_bytes(entry.get(0..4)?.try_into().ok()?) as u64,
                i32::from_be_bytes(entry.get(4..8)?.try_into().ok()?) as i64,
            ),
        };
        if media >= 0 {
            return Some((segment, media as u64));
        }
        at += width;
    }
    None
}
