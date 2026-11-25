use std::fs;
use std::path::Path;

/// The handful of ID3v2 fields worth recovering when neither `lofty` nor `symphonia` could open
/// the tag at all.
#[derive(Default)]
pub struct Lenient {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<i32>,
    pub cover: Option<(Vec<u8>, String)>,
}

/// Reads the frames we care about by hand, skipping every other frame purely by its declared
/// byte size and never looking at what's inside it. `lofty` and `symphonia` both abort the
/// *entire* tag the moment one frame's content doesn't parse — some gamerip taggers write a
/// `WXXX` with no encoding byte at all, just the raw URL in its place — which throws away every
/// other, perfectly well-formed frame right along with it. This only runs as the last resort,
/// once both of those have already given up on the whole tag.
///
/// Deliberately narrow: ID3v2.3/2.4 only (not v2.2's 3-byte frame ids), no extended header, no
/// unsynchronisation — none of which showed up in the files this was written for. A tag using
/// any of them is left exactly as `lofty`/`symphonia` already leave it.
pub fn read(path: &Path) -> Option<Lenient> {
    let data = fs::read(path).ok()?;
    if data.len() < 10 || &data[0..3] != b"ID3" {
        return None;
    }

    let major = data[3];
    if major != 3 && major != 4 {
        return None;
    }
    let flags = data[5];
    if flags & 0x80 != 0 || flags & 0x40 != 0 {
        return None;
    }

    let tag_size = syncsafe(&data[6..10]) as usize;
    let end = (10 + tag_size).min(data.len());

    let mut lenient = Lenient::default();
    let mut pos = 10;
    while pos + 10 <= end {
        let id = &data[pos..pos + 4];
        if id == [0, 0, 0, 0] {
            break;
        }

        let size = if major == 4 {
            syncsafe(&data[pos + 4..pos + 8]) as usize
        } else {
            u32::from_be_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize
        };

        let body_start = pos + 10;
        if body_start > end {
            break;
        }
        let body_end = (body_start + size).min(end);
        let body = &data[body_start..body_end];

        match id {
            b"TIT2" if lenient.title.is_none() => lenient.title = text(body),
            b"TPE1" if lenient.artist.is_none() => lenient.artist = text(body),
            b"TALB" if lenient.album.is_none() => lenient.album = text(body),
            b"TPE2" if lenient.album_artist.is_none() => lenient.album_artist = text(body),
            b"TRCK" if lenient.track_number.is_none() => {
                lenient.track_number = text(body).and_then(|value| leading_number(&value));
            }
            b"TPOS" if lenient.disc_number.is_none() => {
                lenient.disc_number = text(body).and_then(|value| leading_number(&value));
            }
            b"TYER" | b"TDRC" if lenient.year.is_none() => {
                lenient.year = text(body).and_then(|value| value.get(0..4)?.parse().ok());
            }
            b"APIC" if lenient.cover.is_none() => lenient.cover = picture(body),
            _ => {}
        }

        pos = body_end;
    }

    Some(lenient)
}

fn syncsafe(bytes: &[u8]) -> u32 {
    bytes
        .iter()
        .fold(0u32, |acc, byte| (acc << 7) | u32::from(byte & 0x7f))
}

fn leading_number(value: &str) -> Option<u32> {
    value.split('/').next()?.trim().parse().ok()
}

fn text(body: &[u8]) -> Option<String> {
    let (&encoding, rest) = body.split_first()?;
    let decoded = decode_text(encoding, rest)?;
    let trimmed = decoded.trim_matches('\0').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn decode_text(encoding: u8, bytes: &[u8]) -> Option<String> {
    match encoding {
        0 => Some(bytes.iter().map(|&byte| byte as char).collect()),
        3 => Some(String::from_utf8_lossy(bytes).into_owned()),
        1 | 2 => Some(decode_utf16_frame(encoding, bytes)),
        _ => None,
    }
}

fn decode_utf16_frame(encoding: u8, bytes: &[u8]) -> String {
    let (bytes, big_endian) = match (encoding, bytes) {
        (1, [0xFE, 0xFF, rest @ ..]) => (rest, true),
        (1, [0xFF, 0xFE, rest @ ..]) => (rest, false),
        _ => (bytes, true),
    };
    let (pairs, _) = bytes.as_chunks::<2>();
    let units = pairs.iter().map(|&pair| match big_endian {
        true => u16::from_be_bytes(pair),
        false => u16::from_le_bytes(pair),
    });
    char::decode_utf16(units).filter_map(Result::ok).collect()
}

fn picture(body: &[u8]) -> Option<(Vec<u8>, String)> {
    let (&encoding, rest) = body.split_first()?;
    let mime_end = rest.iter().position(|&byte| byte == 0)?;
    let mime = std::str::from_utf8(&rest[..mime_end]).ok()?.to_owned();
    let rest = rest.get(mime_end + 1..)?;
    let (_picture_type, rest) = rest.split_first()?;

    let description_end = match encoding {
        1 | 2 => rest.chunks(2).position(|pair| pair == [0, 0])? * 2 + 2,
        _ => rest.iter().position(|&byte| byte == 0)? + 1,
    };
    let data = rest.get(description_end..)?;
    (!data.is_empty()).then(|| (data.to_owned(), mime))
}
