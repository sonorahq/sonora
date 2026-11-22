//! MPEG audio files, MP3 above all, entered part way.
//!
//! An MPEG frame carries no time of its own, so a decoder entered part way counts from zero and
//! the position it starts at is an estimate. A VBR file usually opens with a Xing frame whose
//! table says where each whole percent of the track starts in the file, and [`Layout`] reads
//! the estimate from that. Any other file is taken to spread its frames evenly, which is exact
//! for a constant bitrate. A graft lands on the first frame [`sync`] can vouch for, since the
//! decoder would otherwise scan stray bytes for any format's marker.

use std::time::Duration;

use super::Place;
use crate::stream::{Body, Spool, Stream};

/// The bitrates in kbit/s by the header's bitrate index, one row each for MPEG-1 layers I, II
/// and III, then MPEG-2 and 2.5 layer I, then MPEG-2 and 2.5 layers II and III.
const BITRATES: [[u16; 15]; 5] = [
    [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
    ],
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
    ],
    [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ],
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
    ],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

/// The version bits of an MPEG-1 header. MPEG-2 is 2, MPEG-2.5 is 0 and 1 is reserved.
const MPEG1: u8 = 3;

/// An ID3v2 tag's own header, and the footer it may end with.
const TAG_HEADER: usize = 10;

/// How many stray bytes may sit between the ID3 tags and the first frame, as the padding some
/// taggers leave without counting it in the tag.
const PADDING: usize = 4096;

/// How far past the ID3 tags the first frame is looked for, which also bounds what one look
/// at the start of the file reads.
const SCAN_WINDOW: usize = 16 * 1024;

/// What one MPEG audio frame header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The two version bits, [`MPEG1`] for MPEG-1.
    version: u8,
    /// The layer, from 1 to 3.
    layer: u8,
    rate: u32,
    mono: bool,
    /// The whole frame in bytes, header included.
    length: usize,
}

/// Where an MP3 file's frames sit, which is what placing a time in it by its bytes goes on.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    length: Duration,
    size: u64,
    /// Where the ID3 tags end, which a decoder entered part way is given in front of the frames.
    head: usize,
    /// Where the first frame of audio starts, past a Xing or Info frame.
    start: u64,
    toc: Option<Toc>,
}

/// A Xing table. Entry `i` is where `i` percent of the track starts, in 256ths of `bytes`
/// counted from the first frame at `first`.
#[derive(Clone, Copy, Debug)]
struct Toc {
    first: u64,
    bytes: u64,
    entries: [u8; 100],
}

/// What the start of an MP3 file says about where its frames are.
#[derive(Clone, Copy, Debug)]
struct Found {
    head: usize,
    first: usize,
    start: usize,
    xing: Option<Xing>,
}

/// What a Xing frame says that placing a time needs: how long the stream is from its first
/// frame, and the table.
#[derive(Clone, Copy, Debug)]
struct Xing {
    bytes: Option<u64>,
    toc: [u8; 100],
}

impl Header {
    /// The header at the start of `bytes`, when the first four bytes are one. Free-format and
    /// reserved values are refused, which keeps random bytes from passing for a header.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let [first, second, third, fourth] = *bytes.first_chunk::<4>()?;
        if first != 0xff || second & 0xe0 != 0xe0 || fourth & 0x03 == 0x02 {
            return None;
        }
        let version = (second >> 3) & 0x03;
        let layer = match (second >> 1) & 0x03 {
            1 => 3,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        let rates = match version {
            MPEG1 => [44100, 48000, 32000],
            2 => [22050, 24000, 16000],
            0 => [11025, 12000, 8000],
            _ => return None,
        };
        let rate = *rates.get(usize::from((third >> 2) & 0x03))?;
        let row = match (version, layer) {
            (MPEG1, layer) => usize::from(layer) - 1,
            (_, 1) => 3,
            _ => 4,
        };
        let bitrate = match usize::from(third >> 4) {
            0 | 15 => return None,
            index => u32::from(BITRATES[row][index]) * 1000,
        };
        let padding = usize::from((third >> 1) & 0x01);
        let length = match (layer, version) {
            (1, _) => ((12 * bitrate / rate) as usize + padding) * 4,
            (3, version) if version != MPEG1 => (72 * bitrate / rate) as usize + padding,
            _ => (144 * bitrate / rate) as usize + padding,
        };
        Some(Self {
            version,
            layer,
            rate,
            mono: fourth >> 6 == 0x03,
            length,
        })
    }

    /// Whether `other` belongs to the same stream, which a frame and the one after it do.
    fn similar(&self, other: &Self) -> bool {
        self.version == other.version
            && self.layer == other.layer
            && self.rate == other.rate
            && self.mono == other.mono
    }

    /// How long a layer III frame's side information is, which a Xing tag follows.
    fn side_info(&self) -> usize {
        match (self.version == MPEG1, self.mono) {
            (true, true) => 17,
            (true, false) => 32,
            (false, true) => 9,
            (false, false) => 17,
        }
    }
}

impl Layout {
    /// The layout of an MP3 file that `main` is downloading, once it knows the file's size and
    /// the start of the file has arrived. This waits for the start, bounded by the patience of
    /// a read. `None` for a track of no known length or a file that is not MP3.
    pub fn measure<B: Body>(main: &Stream<B>, length: Duration) -> Option<Self> {
        if length.is_zero() {
            return None;
        }
        let size = main.total()?;
        let found = main.awaiting(|_, spool, _| scan(spool)).flatten()?;
        Some(Self::new(found, length, size))
    }

    /// The layout `found` describes, for a track of `length` in a file of `size` bytes. A Xing
    /// table whose stream length does not fit in the file is measured against the file instead.
    fn new(found: Found, length: Duration, size: u64) -> Self {
        let first = found.first as u64;
        let toc = found.xing.map(|xing| Toc {
            first,
            bytes: xing
                .bytes
                .filter(|bytes| *bytes > 0 && first + bytes <= size)
                .unwrap_or_else(|| size.saturating_sub(first)),
            entries: xing.toc,
        });
        Self {
            length,
            size,
            head: found.head,
            start: (found.start as u64).min(size),
            toc: toc.filter(|toc| toc.bytes > 0),
        }
    }

    /// How far into the track `at` is, from nought to one.
    fn share(&self, at: Duration) -> f64 {
        (at.as_secs_f64() / self.length.as_secs_f64()).clamp(0.0, 1.0)
    }
}

impl Place for Layout {
    fn head(&self) -> usize {
        self.head
    }

    fn byte(&self, at: Duration) -> u64 {
        let share = self.share(at);
        let byte = match &self.toc {
            Some(toc) => toc.byte(share),
            None => self.start + ((self.size - self.start) as f64 * share) as u64,
        };
        byte.clamp(self.start, self.size)
    }

    fn time(&self, from: u64) -> Duration {
        let share = match &self.toc {
            Some(toc) => toc.share(from),
            None if self.size > self.start => {
                from.saturating_sub(self.start) as f64 / (self.size - self.start) as f64
            }
            None => 0.0,
        };
        self.length.mul_f64(share.clamp(0.0, 1.0))
    }
}

impl Toc {
    /// The byte `share` of the track starts at, between the table's whole percents.
    fn byte(&self, share: f64) -> u64 {
        let percent = share * 100.0;
        let index = (percent as usize).min(99);
        let (low, high) = self.bounds(index);
        let fraction = low + (high - low) * (percent - index as f64);
        self.first + (fraction / 256.0 * self.bytes as f64) as u64
    }

    /// The share of the track byte `from` falls in, the other way round from
    /// [`byte`](Self::byte).
    fn share(&self, from: u64) -> f64 {
        let fraction =
            (from.saturating_sub(self.first) as f64 / self.bytes as f64 * 256.0).clamp(0.0, 256.0);
        let index = self
            .entries
            .iter()
            .rposition(|entry| f64::from(*entry) <= fraction)
            .unwrap_or(0);
        let (low, high) = self.bounds(index);
        let within = match high > low {
            true => ((fraction - low) / (high - low)).clamp(0.0, 1.0),
            false => 0.0,
        };
        (index as f64 + within) / 100.0
    }

    /// Where percent `index` and the one after it start, in 256ths. The end of the stream is
    /// the bound after the last.
    fn bounds(&self, index: usize) -> (f64, f64) {
        let low = f64::from(self.entries[index]);
        let high = self
            .entries
            .get(index + 1)
            .map_or(256.0, |entry| f64::from(*entry));
        (low, high.max(low))
    }
}

/// Where the first frame in `bytes` starts that the next frame follows straight after, which
/// random bytes almost never manage. A frame whose follower is past the end of `bytes` is not
/// vouched for yet.
pub fn sync(bytes: &[u8]) -> Option<usize> {
    (0..bytes.len()).find(|&at| {
        let Some(frame) = Header::parse(&bytes[at..]) else {
            return false;
        };
        bytes
            .get(at + frame.length..)
            .and_then(Header::parse)
            .is_some_and(|next| frame.similar(&next))
    })
}

/// Where an MP3 file's ID3 tags end, which is all a decoder entered part way needs in front of
/// the frames. `Some(None)` when no MPEG frame follows the tags.
pub fn head(spool: &Spool) -> Option<Option<usize>> {
    scan(spool).map(|found| found.map(|found| found.head))
}

/// Reads the start of an MP3 file: the ID3 tags, the first frame after them, and the Xing tag
/// that frame may be. `None` while too little has arrived to say, and `Some(None)` when no
/// frame turns up close enough behind the tags.
fn scan(spool: &Spool) -> Option<Option<Found>> {
    let head = tags(spool)?;
    let wanted = head + SCAN_WINDOW;
    let full = spool.len() >= wanted;
    let window = spool.view(head..spool.len().min(wanted)).ok()?;
    let Some(skip) = sync(&window).filter(|skip| *skip <= PADDING) else {
        return full.then_some(None);
    };
    let frame = Header::parse(&window[skip..])?;
    let xing = xing(&frame, window.get(skip..skip + frame.length)?);
    let first = head + skip;
    Some(Some(Found {
        head,
        first,
        start: first + xing.map_or(0, |_| frame.length),
        xing: xing.flatten(),
    }))
}

/// Where the ID3v2 tags at the start of the file end, or zero without one. `None` while a tag
/// header has not all arrived.
fn tags(spool: &Spool) -> Option<usize> {
    let mut at = 0usize;
    loop {
        let mut header = [0u8; TAG_HEADER];
        spool.read_at(at, &mut header).ok()?;
        let size = &header[6..];
        if &header[..3] != b"ID3" || size.iter().any(|byte| byte & 0x80 != 0) {
            return Some(at);
        }
        let size = size
            .iter()
            .fold(0usize, |size, byte| (size << 7) | usize::from(*byte));
        let footer = match header[5] & 0x10 {
            0 => 0,
            _ => TAG_HEADER,
        };
        at += TAG_HEADER + size + footer;
    }
}

/// What the Xing or Info tag in the first frame says. `None` for a frame of audio, and
/// `Some(None)` for a tag with no usable table, as an Info tag on a constant bitrate file is.
fn xing(frame: &Header, body: &[u8]) -> Option<Option<Xing>> {
    if frame.layer != 3 {
        return None;
    }
    let at = 4 + frame.side_info();
    let table = match body.get(at..at + 4)? {
        b"Xing" => true,
        b"Info" => false,
        _ => return None,
    };
    let flags = u32::from_be_bytes(*body.get(at + 4..)?.first_chunk::<4>()?);
    let mut field = at + 8;
    if flags & 0x01 != 0 {
        field += 4;
    }
    let bytes = match flags & 0x02 {
        0 => None,
        _ => {
            let bytes = u32::from_be_bytes(*body.get(field..)?.first_chunk::<4>()?);
            field += 4;
            Some(u64::from(bytes))
        }
    };
    let toc = (table && flags & 0x04 != 0)
        .then(|| body.get(field..)?.first_chunk::<100>().copied())
        .flatten()
        .filter(|toc| toc.windows(2).all(|pair| pair[0] <= pair[1]) && toc[99] > toc[0]);
    Some(toc.map(|toc| Xing { bytes, toc }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An MPEG-1 layer III header at 128 kbit/s and 44.1 kHz, stereo, unpadded.
    const FRAME: [u8; 4] = [0xff, 0xfb, 0x90, 0x00];

    /// The length of a [`FRAME`].
    const LENGTH: usize = 417;

    fn spool(bytes: &[u8]) -> Spool {
        let mut spool = Spool::in_memory(None);
        spool.append(bytes).unwrap();
        spool
    }

    /// `count` silent frames.
    fn frames(count: usize) -> Vec<u8> {
        let mut frame = FRAME.to_vec();
        frame.resize(LENGTH, 0);
        frame.repeat(count)
    }

    /// A Xing frame with a table where the first half of the track takes a quarter of the bytes.
    fn xing_frame(bytes: u32) -> Vec<u8> {
        let mut frame = FRAME.to_vec();
        frame.resize(4 + 32, 0);
        frame.extend(b"Xing");
        frame.extend(0x07u32.to_be_bytes());
        frame.extend(1000u32.to_be_bytes());
        frame.extend(bytes.to_be_bytes());
        frame.extend((0..100u32).map(|percent| match percent < 50 {
            true => (percent * 64 / 50) as u8,
            false => (64 + (percent - 50) * 192 / 50) as u8,
        }));
        frame.resize(LENGTH, 0);
        frame
    }

    /// An ID3v2 tag holding `size` bytes after its header.
    fn id3(size: u8) -> Vec<u8> {
        let mut tag = b"ID3\x04\x00\x00\x00\x00\x00".to_vec();
        tag.push(size);
        tag.resize(TAG_HEADER + usize::from(size), 0);
        tag
    }

    fn laid_out(file: &[u8], seconds: u64) -> Layout {
        let found = scan(&spool(file)).flatten().unwrap();
        Layout::new(found, Duration::from_secs(seconds), file.len() as u64)
    }

    #[test]
    fn a_header_gives_its_frame_length() {
        let header = Header::parse(&FRAME).unwrap();
        assert_eq!(header.length, LENGTH);
        assert_eq!(header.rate, 44100);
        let padded = Header::parse(&[0xff, 0xfb, 0x92, 0x00]).unwrap();
        assert_eq!(padded.length, LENGTH + 1);
        let mpeg2 = Header::parse(&[0xff, 0xf3, 0x80, 0xc0]).unwrap();
        assert_eq!(mpeg2.length, 72 * 64000 / 22050);
        assert!(mpeg2.mono);
    }

    #[test]
    fn a_header_refuses_what_no_encoder_writes() {
        assert_eq!(Header::parse(&[0xff, 0xf1, 0x50, 0x80]), None, "adts aac");
        assert_eq!(
            Header::parse(&[0xff, 0xfb, 0x00, 0x00]),
            None,
            "free format"
        );
        assert_eq!(
            Header::parse(&[0xff, 0xfb, 0xf0, 0x00]),
            None,
            "bad bitrate"
        );
        assert_eq!(Header::parse(&[0xff, 0xfb, 0x9c, 0x00]), None, "bad rate");
        assert_eq!(
            Header::parse(&[0xff, 0xeb, 0x90, 0x00]),
            None,
            "reserved version"
        );
        assert_eq!(Header::parse(&FRAME[..3]), None, "too short");
    }

    #[test]
    fn a_sync_lands_on_a_frame_its_follower_vouches_for() {
        let mut bytes = vec![1, 2, 3];
        bytes.extend(FRAME);
        bytes.extend([7u8; 20]);
        let real = bytes.len();
        bytes.extend(frames(2));
        assert_eq!(sync(&bytes), Some(real));
        assert_eq!(
            sync(&bytes[..real + LENGTH + 2]),
            None,
            "follower not in yet"
        );
        assert_eq!(sync(&[0u8; 2000]), None);
    }

    #[test]
    fn an_mp3_head_ends_after_the_id3_tags() {
        let mut file = id3(20);
        file.extend(id3(5));
        file.extend([0u8; 3]);
        file.extend(frames(40));
        assert_eq!(head(&spool(&file)), Some(Some(45)));
        assert_eq!(head(&spool(&frames(40))), Some(Some(0)), "no tag");
        assert_eq!(head(&spool(&file[..20])), None, "tag still arriving");
        assert_eq!(head(&spool(&file[..300])), None, "frames still arriving");
        let mut other = b"fLaC".to_vec();
        other.resize(SCAN_WINDOW + 100, 0);
        assert_eq!(head(&spool(&other)), Some(None));
    }

    #[test]
    fn a_constant_bitrate_file_places_time_by_its_bytes() {
        let mut file = id3(90);
        file.extend(frames(100));
        let layout = laid_out(&file, 100);
        assert_eq!(layout.start, 100);
        assert_eq!(layout.byte(Duration::ZERO), 100);
        assert_eq!(
            layout.byte(Duration::from_secs(50)),
            100 + 50 * LENGTH as u64
        );
        assert_eq!(
            layout.time(100 + 25 * LENGTH as u64),
            Duration::from_secs(25)
        );
        assert_eq!(layout.byte(Duration::from_secs(500)), file.len() as u64);
    }

    #[test]
    fn a_xing_table_places_time_where_the_bytes_say() {
        let mut file = id3(90);
        file.extend(xing_frame(0));
        file.extend(frames(100));
        let layout = laid_out(&file, 100);
        let first = 100u64;
        let bytes = file.len() as u64 - first;
        assert_eq!(layout.start, first + LENGTH as u64);
        assert_eq!(layout.byte(Duration::from_secs(50)), first + bytes / 4);
        let half = layout.time(first + bytes / 4);
        assert!(half.abs_diff(Duration::from_secs(50)) < Duration::from_millis(10));
        let back = layout.time(layout.byte(Duration::from_secs(80)));
        assert!(back.abs_diff(Duration::from_secs(80)) < Duration::from_millis(10));
        assert_eq!(
            layout.byte(Duration::ZERO),
            layout.start,
            "clamped to the audio"
        );
    }

    #[test]
    fn a_xing_table_measures_against_its_own_stream_length() {
        let mut file = xing_frame(4 * 1000);
        file.extend(frames(100));
        let layout = laid_out(&file, 100);
        assert_eq!(layout.byte(Duration::from_secs(50)), 1000);
        let mut file = xing_frame(u32::MAX);
        file.extend(frames(100));
        let layout = laid_out(&file, 100);
        assert_eq!(
            layout.byte(Duration::from_secs(50)),
            file.len() as u64 / 4,
            "a stream length past the file is ignored"
        );
    }
}
