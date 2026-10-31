use music::{ArtistRef, Track};
use serde::{Deserialize, Serialize};

/// One thing the listener never wants queued again: a single song, or every song an artist is
/// credited on. The names are kept so the settings page can list an entry without a lookup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Blocked {
    Song {
        id: String,
        name: String,
        #[serde(default)]
        artists: String,
    },
    /// An artist, matched by id where both sides have one and by name otherwise, so a source
    /// that reports no artist ids still has its tracks caught.
    Artist {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        name: String,
    },
}

impl Blocked {
    /// The song a track is, or `None` for a track with no id, which nothing can match later.
    pub fn song(track: &Track) -> Option<Self> {
        Some(Self::Song {
            id: track.id.clone()?,
            name: track.name.clone(),
            artists: track.artists.clone(),
        })
    }

    pub fn artist(id: Option<String>, name: impl Into<String>) -> Self {
        Self::Artist {
            id,
            name: name.into(),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Song { name, .. } | Self::Artist { name, .. } => name,
        }
    }

    /// Whether the entry is an artist rather than a song.
    pub fn is_artist(&self) -> bool {
        matches!(self, Self::Artist { .. })
    }

    /// Whether the entry catches `track`. An artist catches every track they are credited on,
    /// features included.
    pub fn blocks(&self, track: &Track) -> bool {
        match self {
            Self::Song { id, .. } => track.id.as_deref() == Some(id.as_str()),
            Self::Artist { id, name } => match track.artist_refs.is_empty() {
                true => same_name(&track.artists, name),
                false => track
                    .artist_refs
                    .iter()
                    .any(|artist| same_artist(artist, id.as_deref(), name)),
            },
        }
    }

    /// Whether two entries name the same song or artist, so blocking twice keeps one entry.
    fn same(&self, other: &Self) -> bool {
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
}

/// Every blocked song and artist, oldest first. Stored in `settings.json` as a plain list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Blocklist(Vec<Blocked>);

impl Blocklist {
    pub fn entries(&self) -> &[Blocked] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether any entry catches `track`.
    pub fn blocks(&self, track: &Track) -> bool {
        self.0.iter().any(|blocked| blocked.blocks(track))
    }

    /// Whether the list has an entry for the same song or artist as `blocked`.
    pub fn holds(&self, blocked: &Blocked) -> bool {
        self.0.iter().any(|held| held.same(blocked))
    }

    /// Adds `blocked` unless an entry for the same song or artist is there already. Says
    /// whether the list changed.
    pub fn add(&mut self, blocked: Blocked) -> bool {
        if self.holds(&blocked) {
            return false;
        }
        self.0.push(blocked);
        true
    }

    /// Takes out the entry for the same song or artist as `blocked`. Says whether there was one.
    pub fn remove(&mut self, blocked: &Blocked) -> bool {
        let before = self.0.len();
        self.0.retain(|held| !held.same(blocked));
        before != self.0.len()
    }
}

/// Whether a credited artist is the blocked one. Two ids decide on their own, since two artists
/// can share a name; without both, the names decide.
fn same_artist(artist: &ArtistRef, id: Option<&str>, name: &str) -> bool {
    match (artist.id.as_deref(), id) {
        (Some(held), Some(id)) => held == id,
        _ => same_name(&artist.name, name),
    }
}

/// Compares names the way a listener reads them, ignoring case and surrounding space. An empty
/// name matches nothing.
fn same_name(left: &str, right: &str) -> bool {
    let (left, right) = (left.trim(), right.trim());
    !left.is_empty() && left.to_lowercase() == right.to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use music::{ArtistRef, Track};

    use super::{Blocked, Blocklist};

    fn track(id: &str, artists: &[(&str, Option<&str>)]) -> Track {
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
            duration: Duration::from_secs(180),
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
    fn adding_and_removing_entries() {
        let mut list = Blocklist::default();
        let song = Blocked::song(&track("a", &[])).expect("a track with an id is blockable");
        let artist = Blocked::artist(Some("artist-1".into()), "Someone");

        assert!(list.add(song.clone()));
        assert!(list.add(artist.clone()));
        assert_eq!(list.entries(), [song.clone(), artist.clone()]);

        assert!(list.remove(&song));
        assert!(!list.remove(&song));
        assert_eq!(list.entries(), [artist]);
    }

    #[test]
    fn blocking_twice_keeps_one_entry() {
        let mut list = Blocklist::default();

        assert!(list.add(Blocked::artist(Some("x".into()), "Someone")));
        assert!(!list.add(Blocked::artist(Some("x".into()), "Someone renamed")));
        assert!(list.add(Blocked::artist(None, "Other")));
        assert!(!list.add(Blocked::artist(None, " other ")));
        assert_eq!(list.entries().len(), 2);
    }

    #[test]
    fn a_track_without_an_id_cannot_be_blocked_as_a_song() {
        let mut nameless = track("a", &[]);
        nameless.id = None;

        assert!(Blocked::song(&nameless).is_none());
    }

    #[test]
    fn a_song_matches_by_track_id_only() {
        let blocked = Blocked::song(&track("a", &[])).unwrap();

        assert!(blocked.blocks(&track("a", &[("Anyone", None)])));
        assert!(!blocked.blocks(&track("b", &[])));
    }

    #[test]
    fn an_artist_matches_by_id_when_both_sides_have_one() {
        let blocked = Blocked::artist(Some("x".into()), "Someone");

        assert!(blocked.blocks(&track("a", &[("Renamed", Some("x"))])));
        assert!(!blocked.blocks(&track("b", &[("Someone", Some("y"))])));
    }

    #[test]
    fn an_artist_falls_back_to_the_name_without_ids() {
        let blocked = Blocked::artist(Some("x".into()), "Someone");
        let unnamed = Blocked::artist(None, "Someone");

        assert!(blocked.blocks(&track("a", &[("someone", None)])));
        assert!(unnamed.blocks(&track("b", &[("Someone", Some("y"))])));
        assert!(!unnamed.blocks(&track("c", &[("Someone Else", None)])));
    }

    #[test]
    fn an_artist_catches_their_features() {
        let blocked = Blocked::artist(Some("x".into()), "Guest");

        assert!(blocked.blocks(&track("a", &[("Lead", Some("l")), ("Guest", Some("x"))])));
    }

    #[test]
    fn an_artist_matches_the_credit_line_when_a_track_has_no_artist_refs() {
        let blocked = Blocked::artist(None, "Someone");
        let mut bare = track("a", &[]);
        bare.artists = "Someone".into();

        assert!(blocked.blocks(&bare));
    }

    #[test]
    fn an_empty_name_matches_nothing() {
        let blocked = Blocked::artist(None, "");

        assert!(!blocked.blocks(&track("a", &[("", None)])));
    }

    #[test]
    fn the_list_round_trips_through_json() {
        let mut list = Blocklist::default();
        list.add(Blocked::song(&track("a", &[("Someone", None)])).unwrap());
        list.add(Blocked::artist(None, "Someone"));

        let json = serde_json::to_string(&list).unwrap();
        let loaded: Blocklist = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded, list);
        assert!(json.contains(r#""kind":"song""#));
        assert!(!json.contains(r#""id":null"#));
    }
}
