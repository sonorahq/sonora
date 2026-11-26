//! The segment index of a YouTube Music audio file, which is what lets a seek start reading at
//! the right byte instead of decoding its way there.
//!
//! The stream host serves DASH files: `ftyp` and `moov`, then a `sidx` listing every
//! `moof`/`mdat` pair with its size and duration, then the pairs themselves. The index sits in
//! the first kilobyte, so it has arrived long before the decoder opens.

use std::time::Duration;

/// One `moof`/`mdat` pair: where it starts in the file, and where in the media timeline.
#[derive(Clone, Copy, Debug)]
struct Segment {
    at: usize,
    start: Duration,
}

/// Where every segment of a track starts, and where the header in front of them ends.
#[derive(Clone, Debug)]
pub struct Segments {
    head: usize,
    segments: Vec<Segment>,
}

/// Where to start reading a track to land on a position.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// The end of `ftyp` and `moov`, which is what gets spliced in front of the segment.
    pub head: usize,
    /// The byte the segment starts at.
    pub graft: usize,
    /// How far into that segment the position is.
    pub into: Duration,
}

impl Segments {
    /// Reads the index from the head of a file. `None` when no `sidx` follows the `moov`, as in
    /// a file that is not DASH, or when the head has not fully arrived.
    pub fn read(data: &[u8]) -> Option<Self> {
        let mut at = 0;
        let mut head = None;
        while let Some((kind, body, end)) = next_box(data, at) {
            match &kind {
                b"moov" => head = Some(end),
                b"sidx" => return index(body, end, head?),
                b"moof" | b"mdat" => return None,
                _ => {}
            }
            at = end;
        }
        None
    }

    /// Where to start reading to land on `at` in the media timeline.
    pub fn entry(&self, at: Duration) -> Option<Entry> {
        let found = self
            .segments
            .iter()
            .rev()
            .find(|segment| segment.start <= at)?;
        Some(Entry {
            head: self.head,
            graft: found.at,
            into: at.saturating_sub(found.start),
        })
    }

    /// How many segments the index lists.
    pub fn count(&self) -> usize {
        self.segments.len()
    }
}

/// The box that starts at `at`: its four-letter type, its body, and the byte after it. `None`
/// when the box runs past what has arrived.
fn next_box(data: &[u8], at: usize) -> Option<([u8; 4], &[u8], usize)> {
    let size = u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?) as u64;
    let kind: [u8; 4] = data.get(at + 4..at + 8)?.try_into().ok()?;
    let (header, size) = match size {
        1 => {
            let large = data.get(at + 8..at + 16)?;
            (16usize, u64::from_be_bytes(large.try_into().ok()?))
        }
        0 => (8usize, (data.len() - at) as u64),
        _ => (8usize, size),
    };
    let end = at.checked_add(usize::try_from(size).ok()?)?;
    if size < header as u64 {
        return None;
    }
    let body = data.get(at + header..end)?;
    Some((kind, body, end))
}

/// Reads the segment list out of a `sidx` body. `end` is the byte after the box, which the
/// segment offsets count from.
fn index(sidx: &[u8], end: usize, head: usize) -> Option<Segments> {
    let version = *sidx.first()?;
    let timescale = u64::from(read_u32(sidx, 8)?);
    if timescale == 0 {
        return None;
    }
    let (earliest, first, mut at) = match version {
        0 => (
            u64::from(read_u32(sidx, 12)?),
            u64::from(read_u32(sidx, 16)?),
            20,
        ),
        _ => (read_u64(sidx, 12)?, read_u64(sidx, 20)?, 28),
    };
    let count = u16::from_be_bytes(sidx.get(at + 2..at + 4)?.try_into().ok()?);
    at += 4;
    let mut offset = end.checked_add(usize::try_from(first).ok()?)?;
    let mut ticks = earliest;
    let mut segments = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let reference = read_u32(sidx, at)?;
        let duration = read_u32(sidx, at + 4)?;
        at += 12;
        // A reference to another index rather than to media: YouTube never nests them, and
        // following one is not worth the code.
        if reference >> 31 == 1 {
            return None;
        }
        segments.push(Segment {
            at: offset,
            start: Duration::from_secs_f64(ticks as f64 / timescale as f64),
        });
        offset = offset.checked_add((reference & 0x7fff_ffff) as usize)?;
        ticks += u64::from(duration);
    }
    Some(Segments { head, segments })
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
}
