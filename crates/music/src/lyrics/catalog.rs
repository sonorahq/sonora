use crate::{Lyrics, LyricsHit, LyricsQuery, Track};

pub(crate) const TRUST: u32 = 25;
const CANDIDATES: usize = 3;

pub(crate) fn hit(source: &'static str, query: &LyricsQuery, lyrics: Lyrics) -> LyricsHit {
    LyricsHit {
        source,
        trust: TRUST,
        lyrics,
        instrumental: false,
        title: query.title.clone(),
        artist: query.artist.clone(),
        album: query.album.clone(),
        duration: (!query.duration.is_zero()).then_some(query.duration),
        writers: Vec::new(),
    }
}

/// Match the recording before requesting its sheet, retaining the catalogue's metadata so
/// the final lyrics ranking can still reject a different artist, version or duration.
pub(crate) fn candidates(
    source: &'static str,
    query: &LyricsQuery,
    tracks: Vec<Track>,
) -> Vec<(String, LyricsHit)> {
    let mut candidates: Vec<_> = tracks
        .into_iter()
        .filter_map(|track| {
            let id = track.id?;
            let candidate = LyricsQuery {
                title: track.name,
                artist: track.artists,
                album: (!track.album.is_empty()).then_some(track.album),
                duration: track.duration,
                track: None,
            };
            let hit = hit(source, &candidate, Lyrics::plain(String::new()));
            super::matched(query, &hit).then_some((id, hit))
        })
        .collect();
    candidates.sort_by_key(|(_, hit)| std::cmp::Reverse(super::score(query, hit)));
    candidates.truncate(CANDIDATES);
    candidates
}
