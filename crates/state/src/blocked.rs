//! Songs and artists the listener never wants to hear. The list lives in `settings.json` with
//! the other preferences; this module only says what an entry is and which tracks it covers.

use music::Track;
use serde::{Deserialize, Serialize};

/// One blocked song or artist, with the name it is listed under in settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Blocked {
    /// A song, matched by its track id only.
    Song {
        id: String,
        name: String,
        #[serde(default)]
        artists: String,
    },
    /// An artist, matched on every track that credits them: by id, or by name, so blocking one
    /// on a service also covers them on the others, and on tracks that carry no artist ids.
    Artist {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        name: String,
    },
}

impl Blocked {
    /// The song `track`, or `None` for a track with no id to match it by.
    pub fn song(track: &Track) -> Option<Self> {
        Some(Self::Song {
            id: track.id.clone()?,
            name: track.name.clone(),
            artists: track.artists.clone(),
        })
    }

    pub fn artist(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::Artist {
            id: Some(id.into()),
            name: name.into(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Song { name, .. } | Self::Artist { name, .. } => name,
        }
    }

    /// Whether the entry stands for the same song or artist as `other`, whatever it is named.
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Song { id, .. }, Self::Song { id: other, .. }) => id == other,
            (
                Self::Artist { id, name },
                Self::Artist {
                    id: other_id,
                    name: other_name,
                },
            ) => match (id, other_id) {
                (Some(id), Some(other)) => id == other,
                _ => same_name(name, other_name),
            },
            _ => false,
        }
    }

    pub fn matches(&self, track: &Track) -> bool {
        match self {
            Self::Song { id, .. } => track.id.as_deref() == Some(id.as_str()),
            Self::Artist { id, name } => match track.artist_refs.is_empty() {
                true => track
                    .artists
                    .split(',')
                    .any(|credit| same_name(credit, name)),
                false => track.artist_refs.iter().any(|credit| {
                    let by_id = id.is_some() && credit.id.as_deref() == id.as_deref();
                    by_id || same_name(&credit.name, name)
                }),
            },
        }
    }
}

/// Whether any entry of `blocked` covers `track`.
pub fn blocks(blocked: &[Blocked], track: &Track) -> bool {
    blocked.iter().any(|entry| entry.matches(track))
}

/// Adds `entry` to the end of `blocked`, saying whether it was not there yet.
pub(crate) fn add(blocked: &mut Vec<Blocked>, entry: Blocked) -> bool {
    if blocked.iter().any(|held| held.same(&entry)) {
        return false;
    }
    blocked.push(entry);
    true
}

/// Takes `entry` out of `blocked`, saying whether it was there.
pub(crate) fn remove(blocked: &mut Vec<Blocked>, entry: &Blocked) -> bool {
    let before = blocked.len();
    blocked.retain(|held| !held.same(entry));
    before != blocked.len()
}

fn same_name(left: &str, right: &str) -> bool {
    let (left, right) = (left.trim(), right.trim());
    !left.is_empty() && left.to_lowercase() == right.to_lowercase()
}

#[cfg(test)]
pub(crate) mod tests {
    use music::ArtistRef;

    use super::*;

    pub(crate) fn track(id: &str, artists: &[(&str, Option<&str>)]) -> Track {
        Track {
            id: Some(id.to_owned()),
            name: id.to_owned(),
            playable: true,
            artists: artists
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", "),
            artist_refs: artists
                .iter()
                .map(|(name, id)| ArtistRef {
                    name: (*name).to_owned(),
                    id: id.map(str::to_owned),
                })
                .collect(),
            album: String::new(),
            album_id: None,
            cover: None,
            duration: std::time::Duration::from_secs(180),
            added_at: None,
            added_by: None,
            playcount: None,
            popularity: 0,
            explicit: false,
            track_number: 0,
            disc_number: 0,
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        }
    }

    #[test]
    fn a_song_matches_only_its_own_id() {
        let blocked = Blocked::song(&track("a", &[("Band", Some("b1"))])).unwrap();

        assert!(blocked.matches(&track("a", &[])));
        assert!(!blocked.matches(&track("b", &[("Band", Some("b1"))])));
    }

    #[test]
    fn an_artist_matches_every_track_crediting_them() {
        let blocked = Blocked::artist("b1", "Band");

        assert!(blocked.matches(&track("a", &[("Band", Some("b1"))])));
        assert!(blocked.matches(&track("b", &[("Other", None), ("Band", Some("b1"))])));
        assert!(!blocked.matches(&track("c", &[("Other", Some("o1"))])));
    }

    #[test]
    fn an_artist_matches_by_name_across_services() {
        let blocked = Blocked::artist("spotify-id", "The Band");

        assert!(blocked.matches(&track("a", &[("the band", Some("youtube-id"))])));
        let mut plain = track("b", &[]);
        plain.artists = "Someone, The Band".to_owned();
        assert!(blocked.matches(&plain));
    }

    #[test]
    fn adding_twice_keeps_one_entry_and_removing_takes_it_out() {
        let mut blocked = Vec::new();
        let artist = Blocked::artist("b1", "Band");
        let song = Blocked::song(&track("a", &[])).unwrap();

        assert!(add(&mut blocked, artist.clone()));
        assert!(add(&mut blocked, song.clone()));
        assert!(!add(&mut blocked, Blocked::artist("b1", "Renamed")));
        assert_eq!(blocked, [artist.clone(), song.clone()]);

        assert!(remove(&mut blocked, &artist));
        assert!(!remove(&mut blocked, &artist));
        assert_eq!(blocked, [song]);
    }

    #[test]
    fn entries_round_trip_through_json() {
        let blocked = vec![
            Blocked::song(&track("a", &[("Band", None)])).unwrap(),
            Blocked::artist("b1", "Band"),
        ];

        let json = serde_json::to_string(&blocked).unwrap();
        let loaded: Vec<Blocked> = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded, blocked);
    }
}
