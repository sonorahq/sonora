use std::time::Duration;

use opensubsonic::data::{AlbumId3, ArtistId3, Child, ItemGenre, RecordLabel};

use crate::{
    Album, ArtistRef, Playlist, ReleaseType, SavedArtist, Track, UserProfile, iso_8601_to_epoch,
};

pub fn track(song: Child, cover: Option<String>) -> Track {
    // only an OpenSubsonic server names the album's artists, so a plain one leaves this empty
    let (album_artist, _) = artists_of(
        None,
        None,
        song.album_artists.as_ref(),
        // a blank display name would hide the listed album artists
        song.display_album_artist
            .filter(|name| !name.trim().is_empty()),
    );
    let (artists, artist_refs) = artists_of(
        song.artist,
        song.artist_id,
        song.artists.as_ref(),
        song.display_artist,
    );
    Track {
        id: Some(song.id.clone()),
        name: song.title,
        playable: !song.is_video.unwrap_or(false),
        artists,
        artist_refs,
        album: song.album.unwrap_or_default(),
        album_artist,
        album_id: song.album_id.filter(|id| !id.is_empty()),
        cover,
        duration: Duration::from_secs(song.duration.unwrap_or(0).max(0) as u64),
        added_at: iso_8601_to_epoch(song.created.as_deref()),
        added_by: None,
        playcount: song.play_count.map(|count| count as u64),
        popularity: 0,
        explicit: song.explicit_status.as_deref() == Some("explicit"),
        track_number: song.track.unwrap_or(0).max(0) as u32,
        disc_number: song.disc_number.unwrap_or(1).max(1) as u32,
        tags: genres(song.genres.as_deref(), song.genre),
        languages: Vec::new(),
        credits: Vec::new(),
    }
}

/// The genres a song is filed under. An OpenSubsonic server lists each of them, and any other
/// server names one genre at most.
fn genres(listed: Option<&[ItemGenre]>, single: Option<String>) -> Vec<String> {
    let listed: Vec<String> = listed
        .unwrap_or_default()
        .iter()
        .map(|genre| genre.name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if !listed.is_empty() {
        return listed;
    }

    single
        .map(|genre| genre.trim().to_owned())
        .filter(|genre| !genre.is_empty())
        .into_iter()
        .collect()
}

pub fn album(source: AlbumId3, cover: Option<String>, cover_large: Option<String>) -> Album {
    let (artists, artist_refs) = artists_of(
        source.artist,
        source.artist_id,
        source.artists.as_ref(),
        source.display_artist,
    );
    let year = source.year.unwrap_or(0);
    let label = labels(source.record_labels.as_deref());
    Album {
        id: source.id,
        name: source.name,
        artists,
        artist_refs,
        cover,
        cover_large,
        release_type: release_type(source.release_types.as_deref(), source.is_compilation),
        year,
        track_count: source.song_count.unwrap_or(0).max(0) as u32,
        release_date: match year {
            0 => String::new(),
            _ => year.to_string(),
        },
        label,
        copyrights: Vec::new(),
        added_at: iso_8601_to_epoch(source.created.as_deref()),
    }
}

/// The record labels an OpenSubsonic server lists for an album, joined into one credit. A
/// server without the extension lists none, which leaves the label empty.
pub fn labels(labels: Option<&[RecordLabel]>) -> String {
    labels
        .unwrap_or_default()
        .iter()
        .map(|label| label.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The kind of release an OpenSubsonic server lists for an album. A server without the extension
/// lists no types, which leaves every album an album unless it is flagged as a compilation.
pub fn release_type(types: Option<&[String]>, compilation: Option<bool>) -> ReleaseType {
    ReleaseType::from_musicbrainz(
        types.unwrap_or_default().iter().map(String::as_str),
        compilation.unwrap_or(false),
    )
}

pub fn playlist(
    id: &str,
    name: &str,
    owner: Option<&str>,
    public: bool,
    track_count: u32,
    cover: Option<String>,
    username: &str,
) -> Playlist {
    let owner = owner.unwrap_or_default().to_owned();
    Playlist {
        id: id.to_owned(),
        name: name.to_owned(),
        owner: owner.clone(),
        owner_id: owner.clone(),
        owned: !owner.is_empty() && owner == username,
        collaborative: false,
        blend: false,
        public,
        cover,
        track_count,
        modified_at: None,
    }
}

pub fn saved_artist(source: &ArtistId3, cover: Option<String>) -> SavedArtist {
    SavedArtist {
        id: source.id.clone(),
        name: source.name.clone(),
        cover,
        added_at: None,
    }
}

pub fn profile(username: String) -> UserProfile {
    UserProfile {
        id: username.clone(),
        display_name: username,
        avatar: None,
    }
}

pub(crate) fn artists_of(
    artist: Option<String>,
    artist_id: Option<String>,
    many: Option<&Vec<ArtistId3>>,
    display: Option<String>,
) -> (String, Vec<ArtistRef>) {
    if let Some(list) = many.filter(|list| !list.is_empty()) {
        let refs = list
            .iter()
            .map(|item| ArtistRef {
                name: item.name.clone(),
                id: Some(item.id.clone()),
                channel: false,
            })
            .collect();
        return match display {
            Some(name) => (name, refs),
            None => {
                let joined = list
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                (joined, refs)
            }
        };
    }
    let name = display.or(artist).unwrap_or_default();
    let id = artist_id.filter(|id| !id.is_empty());
    let refs = match (name.is_empty(), id) {
        (true, _) => Vec::new(),
        (false, id) => vec![ArtistRef {
            name: name.clone(),
            id,
            channel: false,
        }],
    };
    (name, refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(extra: serde_json::Value) -> Child {
        let mut value = serde_json::json!({
            "id": "1",
            "isDir": false,
            "title": "Blue Monday",
            "artist": "New Order, Arthur Baker",
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn an_opensubsonic_song_names_its_album_artist() {
        let shown = track(
            song(serde_json::json!({ "displayAlbumArtist": "New Order" })),
            None,
        );
        assert_eq!(shown.album_artist, "New Order");
        let listed = serde_json::json!({ "albumArtists": [{ "id": "a", "name": "New Order" }] });
        assert_eq!(track(song(listed), None).album_artist, "New Order");
        let blank = serde_json::json!({
            "displayAlbumArtist": " ",
            "albumArtists": [{ "id": "a", "name": "New Order" }],
        });
        assert_eq!(track(song(blank), None).album_artist, "New Order");
    }

    #[test]
    fn a_plain_subsonic_song_leaves_the_album_artist_empty() {
        let plain = track(song(serde_json::json!({})), None);
        assert_eq!(plain.album_artist, "");
        assert_eq!(plain.artists, "New Order, Arthur Baker");
    }
}
