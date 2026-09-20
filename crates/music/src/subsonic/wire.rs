use std::time::Duration;

use opensubsonic::data::{AlbumId3, ArtistId3, Child};

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
        added_at: when(song.created.as_deref()),
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
        label: String::new(),
        copyrights: Vec::new(),
        added_at: when(source.created.as_deref()),
    }
}

pub fn playlist(
    id: &str,
    name: &str,
    owner: Option<&str>,
    public: bool,
    track_count: u32,
    cover: Option<String>,
    username: &str,
    modified: Option<&str>,
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
        modified_at: when(modified),
    }
}

pub fn saved_artist(source: &ArtistId3, cover: Option<String>) -> SavedArtist {
    SavedArtist {
        id: source.id.clone(),
        name: source.name.clone(),
        cover,
        added_at: when(source.starred.as_deref()),
    }
}

pub fn profile(username: String) -> UserProfile {
    UserProfile {
        id: username.clone(),
        display_name: username,
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

/// An ISO-8601 stamp as seconds since the epoch. Subsonic writes them as `2011-08-20T19:14:18`
/// and sometimes with a `Z` or a numeric offset.
pub(crate) fn when(stamp: Option<&str>) -> Option<i64> {
    datetime(stamp?)
}

fn datetime(stamp: &str) -> Option<i64> {
    let stamp = stamp.trim();
    let (date, time) = match stamp.split_once('T').or_else(|| stamp.split_once(' ')) {
        Some((date, time)) => (date, time),
        None => (stamp, ""),
    };
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let day: i64 = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let (time, zone) = zone_of(time.trim_end_matches('Z'));
    let mut clock = time.split(':');
    let hour: i64 = clock
        .next()
        .filter(|part| !part.is_empty())
        .and_then(|hour| hour.parse().ok())
        .unwrap_or(0);
    let minute: i64 = clock.next().unwrap_or("0").parse().unwrap_or(0);
    let second: i64 = clock
        .next()
        .and_then(|second| second.split('.').next())
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    Some(days(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - zone)
}

fn zone_of(time: &str) -> (&str, i64) {
    let Some(at) = time.rfind(['+', '-']).filter(|&at| at > 0) else {
        return (time, 0);
    };
    let (clock, zone) = time.split_at(at);
    let sign = if zone.starts_with('-') { -1i64 } else { 1 };
    let zone = zone.trim_start_matches(['+', '-']);
    let mut parts = zone.split(':');
    let hours: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let minutes: i64 = parts.next().unwrap_or("0").parse().unwrap_or(0);
    (clock, sign * (hours * 3_600 + minutes * 60))
}

fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
