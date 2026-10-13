use anyhow::{Context as _, Result};
use librespot_core::Session;

use crate::Track;
use crate::spotify::collection;
use crate::spotify::pace::paced;

const TRACK_PREFIX: &str = "spotify:track:";

pub async fn search(session: &Session, query: &str) -> Result<Vec<Track>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let uri = format!("spotify:search:{}", escaped(query));
    let context = paced(|| session.spclient().get_context(&uri))
        .await
        .context("cannot resolve the search context")?;

    let uris: Vec<String> = context
        .pages
        .iter()
        .flat_map(|page| page.tracks.iter())
        .filter_map(|track| track.uri.clone())
        .filter(|uri| uri.starts_with(TRACK_PREFIX))
        .collect();
    if uris.is_empty() {
        return Ok(Vec::new());
    }

    let known = collection::metadata(session, &uris).await?;
    Ok(uris
        .iter()
        .filter_map(|uri| known.get(uri).cloned())
        .collect())
}

fn escaped(query: &str) -> String {
    query
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            b' ' => "+".to_owned(),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::escaped;

    #[test]
    fn keeps_latin_and_folds_spaces() {
        assert_eq!(escaped("queen bohemian"), "queen+bohemian");
    }

    #[test]
    fn percent_encodes_utf8() {
        assert_eq!(escaped("дора"), "%D0%B4%D0%BE%D1%80%D0%B0");
        assert_eq!(escaped("sigur rós"), "sigur+r%C3%B3s");
    }
}
