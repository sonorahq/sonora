use std::time::Duration;

use opensubsonic::data::{AlbumId3, ArtistId3, Child, RecordLabel};

use crate::{Album, ArtistRef, Playlist, ReleaseType, SavedArtist, Track, UserProfile};

pub fn track(song: Child, cover: Option<String>) -> Track {
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
        album_id: song.album_id.filter(|id| !id.is_empty()),
        cover,
        duration: Duration::from_secs(song.duration.unwrap_or(0).max(0) as u64),
        added_at: None,
        added_by: None,
        playcount: song.play_count.map(|count| count as u64),
        popularity: 0,
        explicit: song.explicit_status.as_deref() == Some("explicit"),
        track_number: song.track.unwrap_or(0).max(0) as u32,
        disc_number: song.disc_number.unwrap_or(1).max(1) as u32,
        tags: Vec::new(),
        languages: Vec::new(),
        credits: Vec::new(),
    }
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
        release_type: ReleaseType::Album,
        year,
        track_count: source.song_count.unwrap_or(0).max(0) as u32,
        release_date: match year {
            0 => String::new(),
            _ => year.to_string(),
        },
        label,
        copyrights: Vec::new(),
        added_at: None,
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
        }],
    };
    (name, refs)
}
