use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::store::Store;
use super::wire;
use crate::{PlaylistImportSummary, Track};

/// Imports every playlist file the scan turned up: a file whose name matches an existing
/// playlist (imported before, or made by hand) is left alone, so a playlist is only ever
/// imported once and an in-app edit is never overwritten by a later scan.
pub fn import(paths: &[PathBuf], tracks: &[Track], store: &Store) -> PlaylistImportSummary {
    let known: HashSet<String> = tracks
        .iter()
        .filter_map(|track| track.id.as_deref())
        .filter_map(wire::path_from_track_id)
        .map(|path| path.display().to_string())
        .collect();

    let mut summary = PlaylistImportSummary::default();
    for path in paths {
        let Some(name) = path
            .file_stem()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        match store.name_exists(&name) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                log::warn!("local: cannot check for playlist '{name}': {error:#}");
                continue;
            }
        }

        let Some(entries) = parse(path) else {
            continue;
        };
        let matched: Vec<String> = entries
            .iter()
            .filter(|entry| known.contains(entry.as_str()))
            .map(|entry| format!("{}{entry}", crate::LOCAL_TRACK_PREFIX))
            .collect();
        summary.unmatched += entries.len() - matched.len();
        if matched.is_empty() {
            continue;
        }

        let playlist_id = match store.create(&name) {
            Ok(id) => id,
            Err(error) => {
                log::warn!("local: cannot import playlist '{name}': {error:#}");
                continue;
            }
        };
        if let Err(error) = store.add_all(&playlist_id, &matched) {
            log::warn!("local: cannot import playlist '{name}': {error:#}");
            continue;
        }

        summary.playlists += 1;
        summary.tracks += matched.len();
    }
    summary
}

/// One playlist file's entries, in order, as the raw paths it names. `None` where the file
/// could not be read or parsed at all.
fn parse(path: &Path) -> Option<Vec<String>> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())?;

    match extension.as_str() {
        "m3u" | "m3u8" => parse_m3u(path),
        "pls" => parse_pls(path),
        "xspf" => parse_xspf(path),
        "zpl" | "wpl" => parse_smil(path),
        "asx" => parse_asx(path),
        "b4s" => parse_b4s(path),
        _ => None,
    }
}

fn read(path: &Path) -> Option<String> {
    std::fs::read(path)
        .inspect_err(|error| log::warn!("local: cannot read playlist {path:?}: {error:#}"))
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_m3u(path: &Path) -> Option<Vec<String>> {
    let text = read(path)?;
    Some(
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect(),
    )
}

/// `File1=...`, `File2=...`, in whatever order they appear; Winamp does not guarantee numbering
/// starts at 1 or has no gaps, so this reads every `FileN=` line rather than counting up.
fn parse_pls(path: &Path) -> Option<Vec<String>> {
    let text = read(path)?;
    Some(
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("File"))
            .filter_map(|rest| rest.split_once('='))
            .filter(|(index, _)| index.chars().all(|letter| letter.is_ascii_digit()))
            .map(|(_, value)| value.trim().to_owned())
            .collect(),
    )
}

/// `<track><location>file:///...</location></track>`, per the XSPF spec's URI locations.
fn parse_xspf(path: &Path) -> Option<Vec<String>> {
    let text = read(path)?;
    let document = roxmltree::Document::parse(&text).ok()?;
    Some(
        document
            .descendants()
            .filter(|node| node.has_tag_name("location"))
            .filter_map(|node| node.text())
            .filter_map(uri_to_path)
            .collect(),
    )
}

/// WMP's `.wpl` and Zune's `.zpl` are both this SMIL-based shape:
/// `<media src="C:\path\to\file.mp3" />`.
fn parse_smil(path: &Path) -> Option<Vec<String>> {
    let text = read(path)?;
    let document = roxmltree::Document::parse(&text).ok()?;
    Some(
        document
            .descendants()
            .filter(|node| node.has_tag_name("media"))
            .filter_map(|node| node.attribute("src"))
            .map(str::to_owned)
            .collect(),
    )
}

/// WMP's `.asx`: `<ref href="..." />`, an SGML-descended shape that plays fast and loose with tag
/// and attribute case, so both are matched case-insensitively rather than assuming lowercase.
fn parse_asx(path: &Path) -> Option<Vec<String>> {
    let text = read(path)?;
    let document = roxmltree::Document::parse(&text).ok()?;
    Some(
        document
            .descendants()
            .filter(|node| node.tag_name().name().eq_ignore_ascii_case("ref"))
            .filter_map(|node| find_attribute(node, "href"))
            .map(str::to_owned)
            .collect(),
    )
}

/// Winamp's `.b4s`: `<entry Playstring="file:C:\...\track.mp3">`.
fn parse_b4s(path: &Path) -> Option<Vec<String>> {
    let text = read(path)?;
    let document = roxmltree::Document::parse(&text).ok()?;
    Some(
        document
            .descendants()
            .filter(|node| node.tag_name().name().eq_ignore_ascii_case("entry"))
            .filter_map(|node| find_attribute(node, "playstring"))
            .map(|value| value.strip_prefix("file:").unwrap_or(value).to_owned())
            .collect(),
    )
}

fn find_attribute<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|attribute| attribute.name().eq_ignore_ascii_case(name))
        .map(|attribute| attribute.value())
}

fn uri_to_path(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file:///").unwrap_or(uri);
    let decoded = urlencoding_decode(path);
    Some(decoded.replace('/', "\\"))
}

/// A minimal percent-decoder: XSPF locations are URIs, so a space or a non-ASCII character in a
/// filename comes back escaped. No dependency is pulled in for the handful of escapes a file
/// path actually uses.
fn urlencoding_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
