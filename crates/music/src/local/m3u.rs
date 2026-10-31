//! M3U and M3U8 playlists, the plain lists of file paths most players read and write.

use std::path::{Component, Path, PathBuf};

use percent_encoding::percent_decode_str;

use crate::Track;

use super::wire;

const HEADER: &str = "#EXTM3U";
const FILE_URL: &str = "file://";
const BOM: &[u8] = b"\xEF\xBB\xBF";

/// The files an M3U playlist names, in its order. A relative entry is read against `folder`, the
/// folder the playlist sits in, and a `file://` URL is turned back into a path. Comments, `#EXT`
/// lines and web addresses are left out.
pub fn read(bytes: &[u8], folder: &Path) -> Vec<PathBuf> {
    decode(bytes)
        .lines()
        .filter_map(|line| entry(line.trim(), folder))
        .collect()
}

/// `tracks` as an M3U8 playlist to be saved in `folder`. A track inside `folder` is written
/// relative to it, so the folder can be moved or copied whole with its playlist, and any other
/// track keeps its full path.
pub fn write(tracks: &[Track], folder: &Path) -> String {
    let mut text = format!("{HEADER}\n");
    for track in tracks {
        let Some(path) = track.id.as_deref().and_then(wire::path_from_track_id) else {
            continue;
        };
        let path = path.strip_prefix(folder).unwrap_or(path);
        let title = match track.artists.is_empty() {
            true => track.name.clone(),
            false => format!("{} - {}", track.artists, track.name),
        };
        text.push_str(&format!(
            "#EXTINF:{},{}\n{}\n",
            track.duration.as_secs(),
            title.replace(['\r', '\n'], " "),
            path.display()
        ));
    }
    text
}

/// The text of a playlist file. An `.m3u8` is UTF-8, with or without a byte order mark, while an
/// older `.m3u` may be Latin-1, which is read byte for byte when the file is not valid UTF-8.
fn decode(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(BOM).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&byte| char::from(byte)).collect(),
    }
}

/// The file one line of a playlist names, if it names one.
fn entry(line: &str, folder: &Path) -> Option<PathBuf> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let path = match line.get(..FILE_URL.len()) {
        Some(scheme) if scheme.eq_ignore_ascii_case(FILE_URL) => file_url(&line[FILE_URL.len()..]),
        _ if line.contains("://") => return None,
        _ => separated(line),
    };
    // Joining an absolute path, or a rooted one on Windows, keeps it rather than the folder.
    Some(folder.join(path))
}

/// The path a `file://` URL points at, given what follows the scheme. An empty or `localhost`
/// host means this computer, and a Windows drive letter loses the slash in front of it.
fn file_url(rest: &str) -> PathBuf {
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let decoded = percent_decode_str(rest).decode_utf8_lossy().into_owned();
    let drive = decoded.as_bytes();
    let path = match drive {
        [b'/', letter, b':', ..] if cfg!(windows) && letter.is_ascii_alphabetic() => &decoded[1..],
        _ => &decoded,
    };
    separated(path)
}

/// A path written with the other system's separator, such as a playlist made on Windows read on
/// Linux, turned into this system's. A line that already uses this system's separator is kept
/// as it is, since a Unix file name may hold a backslash.
fn separated(path: &str) -> PathBuf {
    let path = match cfg!(windows) {
        true => path.replace('/', "\\"),
        false if !path.contains('/') => path.replace('\\', "/"),
        false => path.to_owned(),
    };
    PathBuf::from(path)
        .components()
        .filter(|component| !matches!(component, Component::CurDir))
        .collect()
}
