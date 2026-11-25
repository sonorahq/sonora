//! Ogg files, Vorbis and Opus above all, entered part way.
//!
//! An Ogg page names the stream it belongs to and the granule position its last packet ends
//! on, which counts samples from the start of the track. A decoder given the header pages and
//! then pages from the middle of the file finds the next page by its capture pattern and knows
//! where it is from the granule, so the file is entered with [`super::Reach::enter`] like FLAC.

use crate::stream::Spool;

/// How far into the file the header pages may run before the file is taken not to be one a
/// seek can enter. Comment pages carry the cover art, which can run to a few megabytes.
const HEADER_CEILING: usize = 16 * 1024 * 1024;

/// The fixed part of a page header, up to and including its segment count.
const PAGE_HEADER: usize = 27;

/// The flag of a stream's first page.
const FIRST_PAGE: u8 = 0x02;

/// The granule position of a page on which no packet ends.
const NO_GRANULE: u64 = u64::MAX;

/// Where an Ogg file's header pages end, which is where the first page whose granule position
/// counts samples starts. The identification, comment and setup headers all sit on pages of
/// granule zero. `Some(None)` when the file is not Ogg or carries more than one stream.
pub fn head(spool: &Spool) -> Option<Option<usize>> {
    let mut at = 0usize;
    let mut serial = None;
    loop {
        let mut header = [0u8; PAGE_HEADER];
        spool.read_at(at, &mut header).ok()?;
        if &header[..4] != b"OggS" || header[4] != 0 {
            return Some(None);
        }
        let granule = u64::from_le_bytes(header[6..14].try_into().ok()?);
        let stream = u32::from_le_bytes(header[14..18].try_into().ok()?);
        match serial {
            None if header[5] & FIRST_PAGE == 0 => return Some(None),
            None => serial = Some(stream),
            Some(serial) if serial != stream => return Some(None),
            Some(_) => {}
        }
        if at > 0 && granule != 0 && granule != NO_GRANULE {
            return Some(Some(at));
        }
        let mut lacing = [0u8; 255];
        let lacing = &mut lacing[..usize::from(header[26])];
        spool.read_at(at + PAGE_HEADER, lacing).ok()?;
        let body = lacing
            .iter()
            .map(|&length| usize::from(length))
            .sum::<usize>();
        at += PAGE_HEADER + lacing.len() + body;
        if at > HEADER_CEILING {
            return Some(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ranged::spool;

    /// One page of stream `serial` holding a single packet of `body` bytes.
    fn page(flags: u8, granule: u64, serial: u32, body: u8) -> Vec<u8> {
        let mut page = b"OggS".to_vec();
        page.push(0);
        page.push(flags);
        page.extend(granule.to_le_bytes());
        page.extend(serial.to_le_bytes());
        page.extend([0u8; 8]);
        page.push(1);
        page.push(body);
        page.extend(vec![0u8; usize::from(body)]);
        page
    }

    #[test]
    fn an_ogg_head_ends_where_the_granule_starts_counting() {
        let mut file = page(FIRST_PAGE, 0, 7, 19);
        file.extend(page(0, 0, 7, 30));
        let head = file.len();
        file.extend(page(0, 960, 7, 40));
        assert_eq!(head, 47 + 58);
        assert_eq!(super::head(&spool(&file)), Some(Some(head)));
        assert_eq!(
            super::head(&spool(&file[..head + 10])),
            None,
            "still arriving"
        );
    }

    #[test]
    fn a_page_no_packet_ends_on_is_still_header() {
        let mut file = page(FIRST_PAGE, 0, 7, 19);
        file.extend(page(0, NO_GRANULE, 7, 255));
        let head = file.len();
        file.extend(page(0, 960, 7, 40));
        assert_eq!(super::head(&spool(&file)), Some(Some(head)));
    }

    #[test]
    fn an_ogg_head_refuses_other_files_and_streams() {
        assert_eq!(super::head(&spool(&[b'f'; PAGE_HEADER])), Some(None));
        let mut file = page(0, 0, 7, 19);
        file.extend(page(0, 960, 7, 40));
        assert_eq!(super::head(&spool(&file)), Some(None), "no first page");
        let mut file = page(FIRST_PAGE, 0, 7, 19);
        file.extend(page(FIRST_PAGE, 0, 8, 19));
        file.extend(page(0, 960, 7, 40));
        assert_eq!(super::head(&spool(&file)), Some(None), "two streams");
    }
}
