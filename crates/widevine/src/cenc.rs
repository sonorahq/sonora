//! Just enough ISO-BMFF to decrypt a CENC fragmented MP4.
//!
//! CENC is ISO/IEC 23001-7, the container encryption every Widevine stream uses, so nothing
//! here belongs to one service: a CENC fMP4 is an init segment (`ftyp` then `moov`) whose
//! sample entry is `enca` rather than `mp4a`, followed by `moof`/`mdat` pairs. Every encrypted
//! sample carries its own IV in its fragment's `senc` box, and CENC is size preserving, so a
//! sample can be decrypted on its own, in place, once its fragment has arrived. That is what
//! lets playback start on the first fragment rather than the last.
//!
//! This is not a general MP4 reader. It finds the init segment, the sample entries to relabel,
//! and the position, IV and subsample layout of every encrypted sample. Every read is bounds
//! checked against the box it came from, because the bytes arrive from the network.
//!
//! Apple Music's `28:ctrp256` asset is the only stream this reads today, so the shapes it does
//! not handle are the ones that asset never uses: a `moof` whose media is not the box behind
//! it, and a box size of zero meaning "to the end of the file".

use smallvec::SmallVec;

/// The Widevine sample entry this relabels, and the AAC one it relabels it to.
const ENCA: &[u8; 4] = b"enca";
const MP4A: &[u8; 4] = b"mp4a";

/// How many samples one fragment may declare. A `trun` claiming more than this is malformed
/// rather than long: at 1024 frames a sample it is already over four hours.
const SAMPLE_LIMIT: usize = 1 << 20;

/// The flag bits of a `FullBox`, whose first word is one version byte and three flag bytes.
const FLAGS: u32 = 0x00ff_ffff;

/// `tfhd` flags, from ISO/IEC 14496-12. Each field is in the box only when its flag is set,
/// so reading one means stepping over every set flag before it.
const TFHD_BASE_DATA_OFFSET: u32 = 0x01;
const TFHD_SAMPLE_DESCRIPTION_INDEX: u32 = 0x02;
const TFHD_DEFAULT_SAMPLE_DURATION: u32 = 0x08;
const TFHD_DEFAULT_SAMPLE_SIZE: u32 = 0x10;

/// `trun` flags, the same way. The first two say what the box header carries, the rest what
/// each of its per-sample records does.
const TRUN_DATA_OFFSET: u32 = 0x0001;
const TRUN_FIRST_SAMPLE_FLAGS: u32 = 0x0004;
const TRUN_SAMPLE_DURATION: u32 = 0x0100;
const TRUN_SAMPLE_SIZE: u32 = 0x0200;
const TRUN_SAMPLE_FLAGS: u32 = 0x0400;
const TRUN_COMPOSITION_OFFSET: u32 = 0x0800;

/// The `senc` flag, from ISO/IEC 23001-7, that puts a subsample layout after each sample's IV.
const SENC_SUBSAMPLES: u32 = 0x02;

/// One box: its four-character kind, where its body sits, and how long the whole box is.
#[derive(Clone, Copy, Debug)]
struct Bx {
    kind: [u8; 4],
    body: usize,
    end: usize,
    total: usize,
}

/// What the init segment says about one encrypted track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Encrypted {
    pub track: u32,
    /// The per-sample IV size `tenc` declares. `senc` is read against it first and against the
    /// other sizes in use if that does not account for the box exactly.
    pub iv_size: u8,
}

/// The init segment: where it ends, which sample entries claim encryption, and what each
/// encrypted track needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Init {
    /// Widevine init data and default content key id, when carried by the file itself.
    pub pssh: Option<Vec<u8>>,
    pub key_id: Option<[u8; 16]>,
    /// One past the last byte of `moov`, which is where the first fragment starts.
    pub end: usize,
    /// Offsets of the `enca` boxes, so they can be relabelled `mp4a` once the samples under
    /// them are cleartext.
    pub entries: Vec<usize>,
    /// Entry-count offsets of duplicate AAC descriptions that become equivalent after decrypting.
    pub duplicate_descriptions: Vec<usize>,
    pub tracks: Vec<Encrypted>,
    /// Ticks per second of the media timeline, from `mdhd`. Every fragment's decode time is in
    /// these, so without it a position cannot be turned into a place in the file.
    pub timescale: u32,
}

/// The `(clear, encrypted)` byte runs of one sample. Two fit inline, so a track's samples,
/// which live as long as the track does, need no heap block of their own each.
pub type Runs = SmallVec<[(u32, u32); 2]>;

/// One encrypted sample. Offsets are absolute and identical in ciphertext and cleartext.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub start: usize,
    pub len: usize,
    /// Keep the original length: the CDM distinguishes eight-byte and sixteen-byte IVs.
    pub iv: SmallVec<[u8; 16]>,
    /// `(clear, encrypted)` byte runs. Empty means the whole sample is encrypted.
    pub subs: Runs,
}

impl Sample {
    pub fn end(&self) -> usize {
        self.start + self.len
    }
}

/// What a walk of one box found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// A `moof`/`mdat` pair, and where the next box starts. `decode` is the fragment's start on
    /// the media timeline, from `tfdt`, which is what makes a position findable in the file.
    Fragment {
        samples: Vec<Sample>,
        next: usize,
        decode: Option<u64>,
    },
    /// A box that is not a fragment, skipped.
    Other { next: usize },
    /// Not all of the box has arrived yet.
    Partial,
}

fn be16(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at + 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at + 4)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn be64(data: &[u8], at: usize) -> Option<u64> {
    let bytes: [u8; 8] = data.get(at..at + 8)?.try_into().ok()?;
    Some(u64::from_be_bytes(bytes))
}

/// Reads the box header at `at`, refusing one whose declared size runs past what has arrived.
/// A size of zero means "to the end of the file", which a fragmented stream never uses, so it
/// is refused rather than guessed at.
fn read(data: &[u8], at: usize) -> Option<Bx> {
    let size = be32(data, at)? as usize;
    let kind: [u8; 4] = data.get(at + 4..at + 8)?.try_into().ok()?;
    let (body, total) = match size {
        1 => (at + 16, usize::try_from(be64(data, at + 8)?).ok()?),
        0..8 => return None,
        size => (at + 8, size),
    };
    let end = at.checked_add(total)?;
    (body <= end && end <= data.len()).then_some(Bx {
        kind,
        body,
        end,
        total,
    })
}

/// The children of a box, in order, each with where its header starts. Stops at the first
/// header that does not parse, so a truncated tail is ignored rather than fatal.
fn children(data: &[u8], body: usize, end: usize) -> Children<'_> {
    Children {
        data,
        at: body,
        end,
    }
}

/// A walk over the boxes between two offsets, read as it goes rather than collected first.
struct Children<'a> {
    data: &'a [u8],
    at: usize,
    end: usize,
}

impl Iterator for Children<'_> {
    type Item = (usize, Bx);

    fn next(&mut self) -> Option<Self::Item> {
        if self.at + 8 > self.end {
            return None;
        }
        let Some(bx) = read(self.data, self.at).filter(|bx| bx.end <= self.end) else {
            self.at = self.end;
            return None;
        };
        let at = self.at;
        self.at += bx.total;
        Some((at, bx))
    }
}

fn child(data: &[u8], body: usize, end: usize, kind: &[u8; 4]) -> Option<Bx> {
    children(data, body, end)
        .find(|(_, bx)| &bx.kind == kind)
        .map(|(_, bx)| bx)
}

/// The first box of `kind` anywhere under `body`, depth first.
fn descendant(data: &[u8], body: usize, end: usize, kind: &[u8; 4]) -> Option<Bx> {
    for (_, bx) in children(data, body, end) {
        if &bx.kind == kind {
            return Some(bx);
        }
        if let Some(found) = descendant(data, bx.body, bx.end, kind) {
            return Some(found);
        }
    }
    None
}

/// Reads the init segment out of the front of the file. `None` means `moov` has not arrived
/// whole yet, which is the ordinary answer while the first bytes are still landing.
pub fn read_init(data: &[u8]) -> Option<Init> {
    let mut at = 0usize;
    let moov = loop {
        let bx = read(data, at)?;
        if &bx.kind == b"moov" {
            break bx;
        }
        at += bx.total;
    };

    let mut init = Init {
        pssh: children(data, moov.body, moov.end)
            .find(|(_, bx)| {
                &bx.kind == b"pssh"
                    && bx.body + 20 <= bx.end
                    && data.get(bx.body + 4..bx.body + 20) == Some(&crate::SYSTEM_ID)
            })
            .map(|(at, bx)| data[at..bx.end].to_vec()),
        key_id: None,
        end: moov.end,
        entries: Vec::new(),
        duplicate_descriptions: Vec::new(),
        tracks: Vec::new(),
        timescale: 0,
    };
    for (_, trak) in children(data, moov.body, moov.end).filter(|(_, bx)| &bx.kind == b"trak") {
        let track = track_id(data, trak.body, trak.end).unwrap_or(0);
        if let Some(ticks) = timescale(data, trak.body, trak.end) {
            init.timescale = ticks;
        }
        let Some(stsd) = descendant(data, trak.body, trak.end, b"stsd") else {
            continue;
        };
        if duplicate_aac_descriptions(data, stsd) {
            init.duplicate_descriptions.push(stsd.body + 4);
        }
        // stsd body: version and flags, an entry count, then the sample entries.
        for (at, entry) in children(data, stsd.body + 8, stsd.end) {
            if &entry.kind != ENCA {
                continue;
            }
            // An audio sample entry carries 28 bytes of its own before its children.
            let iv_size = iv_size(data, entry.body + 28, entry.end).unwrap_or(16);
            if let Some(tenc) = descendant(data, entry.body + 28, entry.end, b"tenc")
                && tenc.body + 24 <= tenc.end
            {
                init.key_id = data
                    .get(tenc.body + 8..tenc.body + 24)
                    .and_then(|bytes| bytes.try_into().ok());
            }
            init.entries.push(at);
            init.tracks.push(Encrypted { track, iv_size });
        }
    }
    Some(init)
}

/// Some streams describe identical encrypted and clear AAC separately. Once decrypted, one
/// description suffices. Require matching audio headers and complete ES descriptors before
/// hiding the duplicates from decoders that only accept one description.
fn duplicate_aac_descriptions(data: &[u8], stsd: Bx) -> bool {
    if stsd.body + 8 > stsd.end {
        return false;
    }
    let Some(count) = be32(data, stsd.body + 4).map(|count| count as usize) else {
        return false;
    };
    if count < 2 || count > (stsd.end - stsd.body - 8) / 36 {
        return false;
    }
    let mut at = stsd.body + 8;
    let mut first = None;
    for _ in 0..count {
        let Some(entry) = read(data, at).filter(|entry| entry.end <= stsd.end) else {
            return false;
        };
        let Some(description) = aac_description(data, entry) else {
            return false;
        };
        if let Some(first) = first {
            if first != description {
                return false;
            }
        } else {
            first = Some(description);
        }
        at = entry.end;
    }
    at == stsd.end
}

fn aac_description(data: &[u8], entry: Bx) -> Option<(&[u8], &[u8])> {
    if entry.body + 28 > entry.end || (&entry.kind != ENCA && &entry.kind != MP4A) {
        return None;
    }
    let header = data.get(entry.body..entry.body + 28)?;
    if &entry.kind == ENCA {
        let sinf = child(data, entry.body + 28, entry.end, b"sinf")?;
        let frma = child(data, sinf.body, sinf.end, b"frma")?;
        if data.get(frma.body..frma.end)? != MP4A {
            return None;
        }
    }
    let mut at = entry.body + 28;
    let mut esds = None;
    while at < entry.end {
        let bx = read(data, at).filter(|bx| bx.end <= entry.end)?;
        if &bx.kind == b"esds" {
            if esds.is_some() {
                return None;
            }
            esds = data.get(bx.body..bx.end);
        }
        at = bx.end;
    }
    Some((header, esds?))
}

/// The track id out of `tkhd`, whose layout depends on its own version byte.
fn track_id(data: &[u8], body: usize, end: usize) -> Option<u32> {
    let tkhd = child(data, body, end, b"tkhd")?;
    let at = match data.get(tkhd.body)? {
        0 => tkhd.body + 12,
        _ => tkhd.body + 20,
    };
    (at + 4 <= tkhd.end).then(|| be32(data, at)).flatten()
}

/// The media timescale, from `mdia/mdhd`, whose layout depends on its own version byte.
fn timescale(data: &[u8], body: usize, end: usize) -> Option<u32> {
    let mdia = child(data, body, end, b"mdia")?;
    let mdhd = child(data, mdia.body, mdia.end, b"mdhd")?;
    let at = match data.get(mdhd.body)? {
        0 => mdhd.body + 12,
        _ => mdhd.body + 20,
    };
    (at + 4 <= mdhd.end)
        .then(|| be32(data, at))
        .flatten()
        .filter(|ticks| *ticks > 0)
}

/// The default per-sample IV size, from `sinf/schi/tenc`.
fn iv_size(data: &[u8], body: usize, end: usize) -> Option<u8> {
    let sinf = child(data, body, end, b"sinf")?;
    let schi = child(data, sinf.body, sinf.end, b"schi")?;
    let tenc = child(data, schi.body, schi.end, b"tenc")?;
    // tenc body: version and flags, a reserved byte, default_isProtected, then the IV size.
    (tenc.body + 8 <= tenc.end)
        .then(|| data.get(tenc.body + 7).copied())
        .flatten()
}

/// Relabels every `enca` sample entry as `mp4a`, so a decoder reads the now cleartext track as
/// plain AAC. Equivalent duplicate descriptions are exposed as one; their bytes remain as
/// padding in `stsd`, so every box size and file offset survives.
pub fn unlock(buf: &mut [u8], init: &Init) {
    for at in &init.entries {
        if let Some(kind) = buf.get_mut(at + 4..at + 8) {
            kind.copy_from_slice(MP4A);
        }
    }
    for at in &init.duplicate_descriptions {
        if let Some(count) = buf.get_mut(*at..at + 4) {
            count.copy_from_slice(&1u32.to_be_bytes());
        }
    }
}

/// The kind and total length of the box whose header starts `head`, which needs to be no
/// longer than sixteen bytes. `None` means the header is incomplete or claims a size no box
/// can have, so a caller can find where a box ends before it has the rest of the box.
pub fn extent(head: &[u8]) -> Option<([u8; 4], usize)> {
    let size = be32(head, 0)? as usize;
    let kind: [u8; 4] = head.get(4..8)?.try_into().ok()?;
    let total = match size {
        1 => usize::try_from(be64(head, 8)?).ok()?,
        0..8 => return None,
        size => size,
    };
    (total >= 8).then_some((kind, total))
}

/// Walks the one box at the front of `window`, collecting the encrypted samples of a
/// `moof`/`mdat` pair. `window` holds the file from byte `origin` on, so a fragment can be read
/// without the rest of the file at hand, and every position in the answer is a file position.
///
/// A fragment is only reported once both its `moof` and the `mdat` behind it are in the window
/// whole, so the samples it hands back always index into bytes that are there.
pub fn read_fragment(window: &[u8], origin: usize, tracks: &[Encrypted]) -> Step {
    let Some(moof) = read(window, 0) else {
        return Step::Partial;
    };
    if &moof.kind != b"moof" {
        return Step::Other {
            next: origin + moof.end,
        };
    }
    let Some(mdat) = read(window, moof.end) else {
        return Step::Partial;
    };
    if &mdat.kind != b"mdat" {
        // A moof whose media is not the box behind it is not a shape Apple's assets use, and
        // skipping its samples is better than guessing where they live.
        log::warn!("cenc: a moof at {origin} is not followed by its mdat");
        return Step::Other {
            next: origin + moof.end,
        };
    }

    let mut samples = Vec::new();
    let mut decode = None;
    for (_, traf) in children(window, moof.body, moof.end).filter(|(_, bx)| &bx.kind == b"traf") {
        decode = decode.or_else(|| decode_time(window, &traf));
        let found = read_traf(window, origin, &mdat, &traf, tracks);
        match samples.is_empty() {
            true => samples = found,
            false => samples.extend(found),
        }
    }
    Step::Fragment {
        samples,
        next: origin + mdat.end,
        decode,
    }
}

/// Where a fragment starts on the media timeline, from `tfdt`.
fn decode_time(data: &[u8], traf: &Bx) -> Option<u64> {
    let tfdt = child(data, traf.body, traf.end, b"tfdt")?;
    match data.get(tfdt.body)? {
        0 => be32(data, tfdt.body + 4).map(u64::from),
        _ => be64(data, tfdt.body + 4),
    }
}

/// The encrypted samples of one track fragment, read out of a window that starts at the
/// fragment's `moof` and sits at `origin` in the file.
fn read_traf(
    data: &[u8],
    origin: usize,
    mdat: &Bx,
    traf: &Bx,
    tracks: &[Encrypted],
) -> Vec<Sample> {
    let head = child(data, traf.body, traf.end, b"tfhd");
    let track = head.and_then(|tfhd| be32(data, tfhd.body + 4)).unwrap_or(0);
    // A single-track asset is matched whatever its ids say, since an audio-only encode has
    // nowhere else for the samples to belong.
    let Some(encrypted) = tracks
        .iter()
        .find(|known| known.track == track)
        .or_else(|| tracks.first().filter(|_| tracks.len() == 1))
    else {
        return Vec::new();
    };

    let Some(trun) = child(data, traf.body, traf.end, b"trun") else {
        return Vec::new();
    };
    let tfhd = head.map(|tfhd| read_tfhd(data, &tfhd)).unwrap_or_default();
    let Some(run) = read_trun(data, &trun, &tfhd) else {
        return Vec::new();
    };

    // The sample data is offset from the enclosing moof, unless tfhd names a base of its own.
    // That base is a file position, and the window starts at `origin`.
    let base = match tfhd.base {
        Some(base) => base.checked_sub(origin as u64),
        None => Some(0),
    };
    let Some(start) = base
        .and_then(|base| base.checked_add_signed(run.offset))
        .and_then(|start| usize::try_from(start).ok())
        .filter(|start| *start >= mdat.body && *start <= mdat.end)
    else {
        log::warn!("cenc: a trun points its samples outside the mdat behind it");
        return Vec::new();
    };

    let senc_box = child(data, traf.body, traf.end, b"senc");
    let (ivs, mut subs) = match senc_box {
        Some(senc) => {
            let parsed = read_senc(data, &senc, encrypted.iv_size, run.sizes.len());
            if parsed.0.is_empty() && encrypted.iv_size != 0 {
                log::warn!("cenc: a fragment carries no per-sample ivs");
                return Vec::new();
            }
            parsed
        }
        None => (Vec::new(), Vec::new()),
    };

    let mut samples = Vec::with_capacity(run.sizes.len());
    let mut at = start;
    let mut iv = SmallVec::new();
    for (index, len) in run.sizes.iter().copied().enumerate() {
        let len = len as usize;
        if let Some(found) = ivs.get(index) {
            iv = found.clone();
        }
        let end = at.saturating_add(len);
        if end > mdat.end {
            log::warn!("cenc: a fragment's samples run past its mdat");
            break;
        }
        if len > 0 {
            samples.push(Sample {
                start: origin + at,
                len,
                iv: iv.clone(),
                subs: subs.get_mut(index).map(std::mem::take).unwrap_or_default(),
            });
        }
        at = end;
    }
    samples
}

/// The `tfhd` fields this needs: an explicit data base, and a default sample size for a `trun`
/// that lists none.
#[derive(Clone, Copy, Debug, Default)]
struct Tfhd {
    base: Option<u64>,
    default_size: Option<u32>,
}

/// Reads the two `tfhd` fields this needs, stepping over the ones in front of them. The box
/// body is the flag word and the track id, then whichever optional fields the flags named.
fn read_tfhd(data: &[u8], tfhd: &Bx) -> Tfhd {
    let Some(flags) = be32(data, tfhd.body).map(|word| word & FLAGS) else {
        return Tfhd::default();
    };
    let mut at = tfhd.body + 8;
    let mut found = Tfhd::default();
    if flags & TFHD_BASE_DATA_OFFSET != 0 {
        found.base = be64(data, at);
        at += 8;
    }
    if flags & TFHD_SAMPLE_DESCRIPTION_INDEX != 0 {
        at += 4;
    }
    if flags & TFHD_DEFAULT_SAMPLE_DURATION != 0 {
        at += 4;
    }
    if flags & TFHD_DEFAULT_SAMPLE_SIZE != 0 {
        found.default_size = be32(data, at).filter(|size| *size > 0);
    }
    found
}

/// One `trun`: where its samples start relative to the fragment's base, and how long each is.
struct Trun {
    offset: i64,
    sizes: Vec<u32>,
}

/// Reads a `trun`: the header fields its flags named, then one record per sample, of which
/// only the size is wanted. A record's fields are fixed width, so the ones that are not read
/// are stepped over.
fn read_trun(data: &[u8], trun: &Bx, tfhd: &Tfhd) -> Option<Trun> {
    let flags = be32(data, trun.body)? & FLAGS;
    let count = be32(data, trun.body + 4)? as usize;
    if count > SAMPLE_LIMIT {
        log::warn!("cenc: a trun declares {count} samples, which is not a real fragment");
        return None;
    }
    let mut at = trun.body + 8;
    let offset = match flags & TRUN_DATA_OFFSET != 0 {
        true => {
            let offset = i64::from(be32(data, at)? as i32);
            at += 4;
            offset
        }
        false => 0,
    };
    if flags & TRUN_FIRST_SAMPLE_FLAGS != 0 {
        at += 4;
    }

    let mut sizes = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        if flags & TRUN_SAMPLE_DURATION != 0 {
            at += 4;
        }
        match flags & TRUN_SAMPLE_SIZE != 0 {
            true => {
                sizes.push(be32(data, at)?);
                at += 4;
            }
            false => sizes.push(tfhd.default_size?),
        }
        if flags & TRUN_SAMPLE_FLAGS != 0 {
            at += 4;
        }
        if flags & TRUN_COMPOSITION_OFFSET != 0 {
            at += 4;
        }
        if at > trun.end {
            return None;
        }
    }
    Some(Trun { offset, sizes })
}

/// Per-sample IVs and subsample runs out of a `senc` box.
type Senc = (Vec<SmallVec<[u8; 16]>>, Vec<Runs>);

/// Reads `senc` against the IV size `tenc` declared, and against the other sizes in use when
/// that does not account for the box exactly. A wrong size produces IVs that are silently
/// wrong rather than an error, so the length check is what makes this safe.
fn read_senc(data: &[u8], senc: &Bx, declared: u8, samples: usize) -> Senc {
    let Some(flags) = be32(data, senc.body).map(|word| word & FLAGS) else {
        return Senc::default();
    };
    let Some(count) = be32(data, senc.body + 4).map(|count| count as usize) else {
        return Senc::default();
    };
    if count > SAMPLE_LIMIT.min(samples.max(1) * 2) {
        log::warn!("cenc: a senc declares {count} ivs for a run of {samples} samples");
        return Senc::default();
    }
    let Some(raw) = data.get(senc.body + 8..senc.end) else {
        return Senc::default();
    };
    let subsampled = flags & SENC_SUBSAMPLES != 0;

    for size in [declared, 16, 8, 0] {
        if let Some(read) = fit_senc(raw, size, count, subsampled) {
            if size != declared {
                log::debug!("cenc: senc read with an inferred iv size of {size}");
            }
            return read;
        }
    }
    log::warn!("cenc: cannot read a senc box with any iv size");
    Senc::default()
}

/// Reads `count` entries at one IV size, or `None` when they do not account for the box
/// exactly.
fn fit_senc(raw: &[u8], iv_size: u8, count: usize, subsampled: bool) -> Option<Senc> {
    let mut at = 0usize;
    let mut ivs = Vec::with_capacity(count);
    let mut subs = Vec::with_capacity(count);
    for _ in 0..count {
        if iv_size > 0 {
            let size = usize::from(iv_size).min(16);
            ivs.push(SmallVec::from_slice(raw.get(at..at + size)?));
            at += usize::from(iv_size);
        }
        match subsampled {
            true => {
                let runs = usize::from(be16(raw, at)?);
                at += 2;
                let mut pattern = Runs::with_capacity(runs);
                for _ in 0..runs {
                    let clear = u32::from(be16(raw, at)?);
                    let encrypted = be32(raw, at + 2)?;
                    pattern.push((clear, encrypted));
                    at += 6;
                }
                subs.push(pattern);
            }
            false => subs.push(Runs::new()),
        }
    }
    (at == raw.len()).then_some((ivs, subs))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box: four-byte size, four-byte kind, then the body.
    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn joined(parts: &[Vec<u8>]) -> Vec<u8> {
        parts.iter().flatten().copied().collect()
    }

    /// An `enca` sample entry with a `tenc` declaring `iv_size`, inside the boxes `read_init`
    /// walks to reach it.
    fn init_segment(track: u32, iv_size: u8) -> Vec<u8> {
        let mut encryption = vec![0, 0, 0, 0, 0, 0, 1, iv_size];
        encryption.extend_from_slice(&[5; 16]);
        let tenc = boxed(b"tenc", &encryption);
        let schi = boxed(b"schi", &tenc);
        let sinf = boxed(b"sinf", &schi);
        let mut entry = vec![0u8; 28];
        entry.extend_from_slice(&sinf);
        let enca = boxed(ENCA, &entry);
        let mut stsd_body = vec![0, 0, 0, 0, 0, 0, 0, 1];
        stsd_body.extend_from_slice(&enca);
        let stsd = boxed(b"stsd", &stsd_body);
        let stbl = boxed(b"stbl", &stsd);
        let minf = boxed(b"minf", &stbl);
        // mdhd: version and flags, creation, modification, then the timescale.
        let mut mdhd_body = vec![0u8; 12];
        mdhd_body.extend_from_slice(&44_100u32.to_be_bytes());
        mdhd_body.extend_from_slice(&[0u8; 8]);
        let mdhd = boxed(b"mdhd", &mdhd_body);
        let mdia = boxed(b"mdia", &joined(&[mdhd, minf]));
        let mut tkhd_body = vec![0u8; 12];
        tkhd_body.extend_from_slice(&track.to_be_bytes());
        tkhd_body.extend_from_slice(&[0u8; 60]);
        let tkhd = boxed(b"tkhd", &tkhd_body);
        let trak = boxed(b"trak", &joined(&[tkhd, mdia]));
        let moov = boxed(b"moov", &joined(&[trak, crate::pssh(&[5; 16])]));
        joined(&[boxed(b"ftyp", b"isom"), moov])
    }

    #[test]
    fn reads_embedded_widevine_init_and_key_id() {
        let bytes = init_segment(1, 8);
        let init = read_init(&bytes).unwrap();
        assert_eq!(init.key_id, Some([5; 16]));
        assert_eq!(init.pssh, Some(crate::pssh(&[5; 16])));
        assert!(read_init(&bytes[..bytes.len() - 1]).is_none());
    }

    /// One fragment: a `moof` carrying `senc` and `trun` for `sizes`, then the `mdat` holding
    /// the bytes themselves, each sample filled with its own index.
    fn fragment(track: u32, sizes: &[u32], ivs: &[[u8; 16]]) -> Vec<u8> {
        let mut tfhd_body = vec![0, 0, 0, 0];
        tfhd_body.extend_from_slice(&track.to_be_bytes());
        let tfhd = boxed(b"tfhd", &tfhd_body);

        // tfdt: version and flags, then the fragment's start on the media timeline.
        let mut tfdt_body = vec![0u8; 4];
        tfdt_body.extend_from_slice(&88_200u32.to_be_bytes());
        let tfdt = boxed(b"tfdt", &tfdt_body);

        let mut senc_body = vec![0, 0, 0, 0];
        senc_body.extend_from_slice(&(ivs.len() as u32).to_be_bytes());
        for iv in ivs {
            senc_body.extend_from_slice(iv);
        }
        let senc = boxed(b"senc", &senc_body);

        // flags 0x000201: a data offset, and a size for every sample.
        let mut trun_body = vec![0, 0, 0x02, 0x01];
        trun_body.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        let offset_at = trun_body.len();
        trun_body.extend_from_slice(&0u32.to_be_bytes());
        for size in sizes {
            trun_body.extend_from_slice(&size.to_be_bytes());
        }
        let trun = boxed(b"trun", &trun_body);
        let trun_len = trun.len();

        let mut moof = boxed(b"moof", &boxed(b"traf", &joined(&[tfhd, tfdt, senc, trun])));
        // From the start of the moof to the first byte of the mdat body.
        let offset = (moof.len() + 8) as u32;
        let at = moof.len() - trun_len + 8 + offset_at;
        moof[at..at + 4].copy_from_slice(&offset.to_be_bytes());

        let payload: Vec<u8> = sizes
            .iter()
            .enumerate()
            .flat_map(|(index, size)| vec![index as u8 + 1; *size as usize])
            .collect();
        joined(&[moof, boxed(b"mdat", &payload)])
    }

    #[test]
    fn reads_the_init_segment_and_its_encrypted_track() {
        let data = init_segment(7, 16);
        let init = read_init(&data).expect("the moov is whole");
        assert_eq!(init.end, data.len());
        assert_eq!(
            init.tracks,
            vec![Encrypted {
                track: 7,
                iv_size: 16
            }]
        );
        assert_eq!(init.entries.len(), 1);
        let at = init.entries[0];
        assert_eq!(&data[at + 4..at + 8], ENCA);
    }

    #[test]
    fn a_truncated_init_segment_is_not_an_error() {
        let data = init_segment(1, 16);
        assert!(read_init(&data[..data.len() - 20]).is_none());
        assert!(read_init(&[]).is_none());
        assert!(read_init(&[0, 0, 0, 3, b'f', b't', b'y', b'p']).is_none());
    }

    #[test]
    fn relabels_the_sample_entry_as_aac() {
        let mut data = init_segment(1, 16);
        let init = read_init(&data).unwrap();
        unlock(&mut data, &init);
        let at = init.entries[0];
        assert_eq!(&data[at + 4..at + 8], MP4A);
        // The relabel must not move anything: same length, and it still parses.
        assert_eq!(read_init(&data).unwrap().end, init.end);
    }

    fn duplicate_aac_init(second_config: &[u8], second_channels: u8) -> Vec<u8> {
        let description = |encrypted: bool, config: &[u8], channels: u8| {
            let mut header = vec![0u8; 28];
            header[17] = channels;
            header.extend_from_slice(&boxed(b"esds", config));
            if encrypted {
                header.extend_from_slice(&boxed(b"sinf", &boxed(b"frma", MP4A)));
            }
            boxed(if encrypted { ENCA } else { MP4A }, &header)
        };
        let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 2];
        stsd.extend_from_slice(&description(true, &[0, 0, 0, 0, 0x12, 0x10], 2));
        stsd.extend_from_slice(&description(false, second_config, second_channels));
        boxed(b"moov", &boxed(b"trak", &boxed(b"stsd", &stsd)))
    }

    #[test]
    fn exposes_equivalent_aac_descriptions_as_one_without_moving_bytes() {
        let mut data = duplicate_aac_init(&[0, 0, 0, 0, 0x12, 0x10], 2);
        let init = read_init(&data).unwrap();
        assert_eq!(init.duplicate_descriptions.len(), 1);
        let before = data.clone();
        unlock(&mut data, &init);
        let count = init.duplicate_descriptions[0];
        assert_eq!(be32(&data, count), Some(1));
        assert_eq!(data.len(), before.len());
        for (at, (&old, &new)) in before.iter().zip(&data).enumerate() {
            if old != new {
                assert!(
                    (count..count + 4).contains(&at)
                        || init
                            .entries
                            .iter()
                            .any(|entry| (entry + 4..entry + 8).contains(&at))
                );
            }
        }
        assert_eq!(read_init(&data).unwrap().end, init.end);
    }

    #[test]
    fn keeps_distinct_aac_configurations_and_headers() {
        for (config, channels) in [
            (&[0, 0, 0, 0, 0x12, 0x11][..], 2),
            (&[0, 0, 0, 0, 0x12, 0x10][..], 1),
        ] {
            let mut data = duplicate_aac_init(config, channels);
            let init = read_init(&data).unwrap();
            assert!(init.duplicate_descriptions.is_empty());
            let stsd = descendant(&data, 0, data.len(), b"stsd").unwrap();
            unlock(&mut data, &init);
            assert_eq!(be32(&data, stsd.body + 4), Some(2));
        }
    }

    #[test]
    fn duplicate_descriptions_tolerate_truncation_and_reject_incomplete_entries() {
        let data = duplicate_aac_init(&[0, 0, 0, 0, 0x12, 0x10], 2);
        let init = read_init(&data).unwrap();
        for end in 0..data.len() {
            let mut short = data[..end].to_vec();
            assert!(read_init(&short).is_none());
            unlock(&mut short, &init);
        }
        let mut malformed = data;
        let stsd = descendant(&malformed, 0, malformed.len(), b"stsd").unwrap();
        malformed[stsd.body + 4..stsd.body + 8].copy_from_slice(&3u32.to_be_bytes());
        assert!(
            read_init(&malformed)
                .unwrap()
                .duplicate_descriptions
                .is_empty()
        );
    }

    #[test]
    fn finds_every_sample_of_a_fragment() {
        let ivs = [[1u8; 16], [2u8; 16], [3u8; 16]];
        let sizes = [10u32, 20, 30];
        let init = init_segment(1, 16);
        let data = joined(&[init, fragment(1, &sizes, &ivs)]);

        let parsed = read_init(&data).unwrap();
        let Step::Fragment {
            samples,
            next,
            decode,
        } = read_fragment(&data[parsed.end..], parsed.end, &parsed.tracks)
        else {
            panic!("the fragment is whole");
        };
        assert_eq!(next, data.len());
        // Two seconds in at the builder's timescale, which is what a seek is looked up by.
        assert_eq!(decode, Some(88_200));
        assert_eq!(parsed.timescale, 44_100);
        assert_eq!(samples.len(), 3);
        assert_eq!(samples[0].len, 10);
        assert_eq!(samples[0].iv.as_slice(), &[1u8; 16]);
        assert_eq!(samples[2].iv.as_slice(), &[3u8; 16]);
        assert_eq!(samples[1].start, samples[0].end());
        // Every sample must point at its own bytes, which the builder filled with its index.
        for (index, sample) in samples.iter().enumerate() {
            assert_eq!(
                &data[sample.start..sample.end()],
                vec![index as u8 + 1; sample.len].as_slice(),
                "sample {index} is misplaced"
            );
        }
    }

    #[test]
    fn a_half_arrived_fragment_waits() {
        let init = init_segment(1, 16);
        let data = joined(&[init, fragment(1, &[10, 20], &[[1u8; 16], [2u8; 16]])]);
        let parsed = read_init(&data).unwrap();
        // The moof is whole but the mdat behind it is not.
        let cut = data.len() - 5;
        assert_eq!(
            read_fragment(&data[parsed.end..cut], parsed.end, &parsed.tracks),
            Step::Partial
        );
        // And a moof that is itself only half there.
        assert_eq!(
            read_fragment(
                &data[parsed.end..parsed.end + 12],
                parsed.end,
                &parsed.tracks
            ),
            Step::Partial
        );
    }

    #[test]
    fn a_box_that_is_not_a_fragment_is_skipped() {
        let data = joined(&[boxed(b"free", &[0u8; 16]), boxed(b"moof", &[])]);
        assert_eq!(read_fragment(&data, 0, &[]), Step::Other { next: 24 });
    }

    #[test]
    fn a_senc_the_declared_iv_size_does_not_fit_is_inferred() {
        // Eight-byte IVs in the box, sixteen declared in tenc.
        let mut senc_body = vec![0, 0, 0, 0];
        senc_body.extend_from_slice(&2u32.to_be_bytes());
        senc_body.extend_from_slice(&[9u8; 8]);
        senc_body.extend_from_slice(&[8u8; 8]);
        let data = boxed(b"senc", &senc_body);
        let senc = read(&data, 0).unwrap();
        let (ivs, subs) = read_senc(&data, &senc, 16, 2);
        assert_eq!(ivs.len(), 2);
        assert_eq!(ivs[0][..8], [9u8; 8]);
        assert_eq!(ivs[0].len(), 8, "the CDM needs the original IV length");
        assert_eq!(subs, vec![Runs::new(), Runs::new()]);
    }

    #[test]
    fn reads_subsample_runs_when_the_senc_flags_say_so() {
        let mut senc_body = vec![0, 0, 0, 0x02];
        senc_body.extend_from_slice(&1u32.to_be_bytes());
        senc_body.extend_from_slice(&[7u8; 16]);
        senc_body.extend_from_slice(&1u16.to_be_bytes());
        senc_body.extend_from_slice(&16u16.to_be_bytes());
        senc_body.extend_from_slice(&64u32.to_be_bytes());
        let data = boxed(b"senc", &senc_body);
        let senc = read(&data, 0).unwrap();
        let (ivs, subs) = read_senc(&data, &senc, 16, 1);
        assert_eq!(ivs.len(), 1);
        assert_eq!(ivs[0].as_slice(), &[7u8; 16]);
        assert_eq!(subs, vec![Runs::from_slice(&[(16u32, 64u32)])]);
    }

    /// A box size that claims more than has arrived, or less than a header, must not be read.
    #[test]
    fn refuses_a_box_that_does_not_fit_what_arrived() {
        let mut data = boxed(b"moof", &[0u8; 8]);
        assert!(read(&data, 0).is_some());
        data[..4].copy_from_slice(&999u32.to_be_bytes());
        assert!(read(&data, 0).is_none());
        data[..4].copy_from_slice(&4u32.to_be_bytes());
        assert!(read(&data, 0).is_none());
        data[..4].copy_from_slice(&0u32.to_be_bytes());
        assert!(read(&data, 0).is_none(), "a size of zero is refused");
    }

    /// A `trun` whose sample count is larger than the box could hold must not allocate for it.
    #[test]
    fn refuses_an_oversized_sample_count() {
        let mut trun_body = vec![0, 0, 0x02, 0x00];
        trun_body.extend_from_slice(&u32::MAX.to_be_bytes());
        let data = boxed(b"trun", &trun_body);
        let trun = read(&data, 0).unwrap();
        assert!(read_trun(&data, &trun, &Tfhd::default()).is_none());
    }

    #[test]
    fn unencrypted_fragment_without_senc_preserves_samples_with_empty_iv() {
        let sizes = &[10u32, 20u32];
        let mut tfhd_body = vec![0, 0, 0, 0];
        tfhd_body.extend_from_slice(&1u32.to_be_bytes()); // track 1
        let tfhd = boxed(b"tfhd", &tfhd_body);

        let mut trun_body = vec![0, 0, 0x02, 0x01]; // flags: data-offset-present + sample-size-present
        trun_body.extend_from_slice(&(sizes.len() as u32).to_be_bytes());
        let offset_at = trun_body.len();
        trun_body.extend_from_slice(&0u32.to_be_bytes());
        for size in sizes {
            trun_body.extend_from_slice(&size.to_be_bytes());
        }
        let trun = boxed(b"trun", &trun_body);
        let trun_len = trun.len();

        let mut moof = boxed(b"moof", &boxed(b"traf", &joined(&[tfhd, trun])));
        let offset = (moof.len() + 8) as u32;
        let at = moof.len() - trun_len + 8 + offset_at;
        moof[at..at + 4].copy_from_slice(&offset.to_be_bytes());

        let payload = vec![0u8; 30];
        let window = joined(&[moof, boxed(b"mdat", &payload)]);

        let tracks = vec![Encrypted {
            track: 1,
            iv_size: 16,
        }];
        let step = read_fragment(&window, 0, &tracks);
        match step {
            Step::Fragment { samples, .. } => {
                assert_eq!(samples.len(), 2);
                assert!(samples[0].iv.is_empty());
                assert!(samples[1].iv.is_empty());
                assert_eq!(samples[0].len, 10);
                assert_eq!(samples[1].len, 20);
            }
            _ => panic!("expected fragment"),
        }
    }
}
