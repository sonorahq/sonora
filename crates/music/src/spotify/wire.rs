use std::fmt::Write as _;

use spotifyapis::proto::metadata::Image;
use spotifyapis::proto::metadata::ImageSize;
use spotifyapis::proto::playlist::{ListAttributes, SelectedListContent as RootList};

use crate::models;

pub const UNKNOWN: &str = "Unknown";
const IMAGE_CDN: &str = "https://image-cdn-fa.spotifycdn.com/image/";
const IMAGE_PREFIXES: [&str; 3] = [
    IMAGE_CDN,
    "https://i.scdn.co/image/",
    "https://image-cdn-ak.spotifycdn.com/image/",
];
const ALBUM_COVER: &str = "ab67616d0000";
const ARTIST_PORTRAIT: &str = "ab6761610000";
/// The size codes that follow `ALBUM_COVER` in an album cover id.
const LARGE: &str = "b273";
const MEDIUM: &str = "1e02";
const ORIGINAL: &str = "82c1";
const BLEND: &str = "blend";
const BY_SIZE: [&str; 4] = ["xlarge", "large", "default", "small"];

pub fn image_url(file_id: &[u8]) -> Option<String> {
    if file_id.is_empty() {
        return None;
    }

    let mut url = String::with_capacity(IMAGE_CDN.len() + file_id.len() * 2);
    url.push_str(IMAGE_CDN);
    append_hex(&mut url, file_id);
    Some(url)
}

/// Appends a lower-case hexadecimal representation without allocating per byte.
pub(super) fn append_hex(output: &mut String, bytes: &[u8]) {
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
}

/// Joins credit names directly into the result string.
pub(super) fn join_names<'a>(mut names: impl Iterator<Item = &'a str> + Clone) -> String {
    let Some(first) = names.next() else {
        return String::new();
    };
    let remaining: usize = names.clone().map(|name| name.len() + 2).sum();
    let mut joined = String::with_capacity(first.len() + remaining);
    joined.push_str(first);
    for name in names {
        joined.push_str(", ");
        joined.push_str(name);
    }
    joined
}

/// Reads a playlist picture URL or Spotify image URI from a projected list.
pub(super) fn playlist_image(address: &str) -> Option<String> {
    if let Some(ids) = address.strip_prefix("spotify:mosaic:") {
        let mut ids = ids.split(':');
        let images = [ids.next()?, ids.next()?, ids.next()?, ids.next()?];
        if ids.next().is_some()
            || !images
                .iter()
                .all(|id| id.len() == 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return None;
        }
        let prefix = "https://mosaic.scdn.co/640/";
        let mut url = String::with_capacity(prefix.len() + 4 * 40);
        url.push_str(prefix);
        for image in images {
            url.push_str(image);
        }
        return Some(url);
    }
    if let Some(id) = address.strip_prefix("spotify:image:") {
        return (id.len() == 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then(|| format!("{IMAGE_CDN}{id}"));
    }
    fetchable(address).then(|| address.to_owned())
}

/// An album cover from `image_url` at another size. Album cover ids are `ab67616d0000`, a
/// two-byte size code and a hash shared by every size, and `82c1` asks the CDN for the original
/// upload, up to 2000 pixels where the listed sizes stop at 640.
pub fn cover_at(url: &str, size: models::CoverSize) -> Option<String> {
    let code = match size {
        models::CoverSize::Large => LARGE,
        models::CoverSize::Full => ORIGINAL,
    };
    let (_, hash) = album_cover_id(url)?;
    Some(format!("{IMAGE_CDN}{ALBUM_COVER}{code}{hash}"))
}

/// Uses a verified Spotify CDN size that covers the requested physical pixels.
pub(super) fn thumbnail_at(url: &str, edge: u32) -> Option<String> {
    if let Some((code, hash)) = album_cover_id(url) {
        return (code == LARGE && edge <= 300)
            .then(|| format!("{IMAGE_CDN}{ALBUM_COVER}{MEDIUM}{hash}"));
    }
    let (current, hash) = sized_image_id(url, ARTIST_PORTRAIT)?;
    let (pixels, _) = [(160, "f178"), (320, "5174"), (640, "e5eb")]
        .into_iter()
        .find(|&(_, code)| code == current)?;
    let smaller = match edge {
        0..=160 if pixels > 160 => "f178",
        161..=320 if pixels > 320 => "5174",
        _ => return None,
    };
    Some(format!("{IMAGE_CDN}{ARTIST_PORTRAIT}{smaller}{hash}"))
}

fn album_cover_id(url: &str) -> Option<(&str, &str)> {
    sized_image_id(url, ALBUM_COVER)
}

fn sized_image_id<'a>(url: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let id = IMAGE_PREFIXES
        .iter()
        .find_map(|prefix| url.strip_prefix(*prefix))?
        .strip_prefix(kind)?;
    if id.len() != 28 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(id.split_at(4))
}

/// An image's width in pixels, taken from its size class when the metadata leaves the width
/// out.
pub fn image_width(image: &Image) -> i32 {
    if image.width() > 0 {
        return image.width();
    }

    match image.size() {
        ImageSize::Small => 64,
        ImageSize::Default => 300,
        ImageSize::Large => 640,
        ImageSize::Xlarge => 1_000,
    }
}

pub fn blend(attributes: &ListAttributes) -> bool {
    attributes.format().to_ascii_lowercase().contains(BLEND)
        || attributes
            .format_attributes
            .iter()
            .any(|attribute| attribute.key().to_ascii_lowercase().starts_with(BLEND))
}

pub fn fetchable(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://")
}

fn playlist_cover(attributes: &ListAttributes) -> Option<String> {
    for target in BY_SIZE {
        if let Some(size) = attributes
            .picture_size
            .iter()
            .find(|size| size.target_name() == target)
            .filter(|size| fetchable(size.url()))
        {
            return Some(size.url().to_owned());
        }
    }

    attributes
        .picture_size
        .first()
        .map(|size| size.url())
        .filter(|url| fetchable(url))
        .map(str::to_owned)
        .or_else(|| image_url(attributes.picture()))
}

pub fn playlist_from(id: &str, content: &RootList, username: &str) -> models::Playlist {
    let mut playlist = playlist_row(
        id,
        content.attributes.as_ref(),
        content.owner_username(),
        content.length(),
        content.timestamp(),
        content.capabilities.as_ref(),
    );
    playlist.owned = playlist.owner == username;
    playlist
}

pub fn playlists_from(rootlist: &RootList) -> Vec<models::Playlist> {
    let Some(contents) = &rootlist.contents else {
        return Vec::new();
    };
    contents
        .items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            let id = item.uri.strip_prefix("spotify:playlist:")?;
            let mut playlist = match contents.meta_items.get(index) {
                Some(meta) => playlist_row(
                    id,
                    meta.attributes.as_ref(),
                    meta.owner_username(),
                    meta.length(),
                    meta.timestamp(),
                    meta.capabilities.as_ref(),
                ),
                None => playlist_row(id, None, "", 0, 0, None),
            };
            playlist.public = item.attributes.as_ref().is_some_and(|attrs| attrs.public());
            Some(playlist)
        })
        .collect()
}

/// Maps the common playlist metadata carried by detail and root-list responses.
fn playlist_row(
    id: &str,
    attributes: Option<&ListAttributes>,
    owner: &str,
    length: i32,
    stamp: i64,
    permissions: Option<&spotifyapis::proto::playlist::Capabilities>,
) -> models::Playlist {
    let label = |name: &str| {
        if name.is_empty() {
            UNKNOWN.to_owned()
        } else {
            name.to_owned()
        }
    };
    models::Playlist {
        id: id.to_owned(),
        name: label(attributes.map(ListAttributes::name).unwrap_or_default()),
        owner: label(owner),
        owner_id: owner.to_owned(),
        collaborative: attributes.is_some_and(ListAttributes::collaborative)
            || permissions.is_some_and(|permissions| permissions.can_edit_items()),
        blend: attributes.is_some_and(blend),
        cover: attributes.and_then(playlist_cover),
        track_count: length.max(0) as u32,
        modified_at: seconds(stamp),
        ..Default::default()
    }
}

/// A copyright line as a release page prints it, led by ℗ for the sound recording or © for
/// the work. Spotify keeps the symbol out of the text and names the kind beside it, so a
/// line that already carries a symbol is left as it is.
pub fn copyright(recording: bool, text: &str) -> String {
    match text.starts_with(['©', '℗']) {
        true => text.to_owned(),
        false => match recording {
            true => format!("℗ {text}"),
            false => format!("© {text}"),
        },
    }
}

pub fn seconds(millis: i64) -> Option<i64> {
    (millis > 0).then_some(millis / 1_000)
}
