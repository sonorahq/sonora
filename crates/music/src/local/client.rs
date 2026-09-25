use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use storage::Database;

use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistProfile, GenreItem, GenreSection, HomeFeed,
    MediaKind, MusicApi, Playlist, PlaylistDetail, SUGGESTIONS, SavedArtist, Track, TrackTags,
    UserProfile, distinct_covers,
};

use super::index::Index;
use super::scan::Scanned;
use super::store::{Starred, Store};
use super::{tags, wire};

const COVERS: usize = 4;
const NOT_SUPPORTED: &str = "local playlists are not shared";

pub struct LocalClient {
    scanned: RwLock<Scanned>,
    store: Store,
    cache_dir: PathBuf,
    index: Index,
}

impl LocalClient {
    pub fn new(scanned: Scanned, database: Database, cache_dir: PathBuf, index: Index) -> Self {
        Self {
            scanned: RwLock::new(scanned),
            store: Store::new(database),
            cache_dir,
            index,
        }
    }

    fn listed(&self, ids: &[String]) -> Vec<Track> {
        let scanned = self.scanned.read().unwrap();
        ids.iter()
            .filter_map(|id| {
                scanned
                    .tracks
                    .iter()
                    .find(|track| track.id.as_deref() == Some(id.as_str()))
                    .cloned()
            })
            .collect()
    }

    fn assemble(&self, id: &str) -> Result<PlaylistDetail> {
        let stored = self.store.one(id)?;
        let tracks = self.listed(&self.store.tracks(id)?);
        Ok(PlaylistDetail {
            playlist: playlist_from(stored.id, stored.name, stored.modified_at, &tracks),
            tracks,
            continuation: None,
        })
    }

    fn album_track_paths(&self, track_id: &str) -> Vec<PathBuf> {
        let scanned = self.scanned.read().unwrap();
        let Some(album_id) = scanned
            .tracks
            .iter()
            .find(|track| track.id.as_deref() == Some(track_id))
            .and_then(|track| track.album_id.as_deref())
        else {
            return Vec::new();
        };

        scanned
            .tracks
            .iter()
            .filter(|track| track.album_id.as_deref() == Some(album_id))
            .filter_map(|track| track.id.as_deref())
            .filter_map(wire::path_from_track_id)
            .map(Path::to_path_buf)
            .collect()
    }

    /// One album's tracks in playing order. The scan keeps every track in the order the
    /// folders were walked, which is not the order an album is meant to be heard in.
    fn album_songs(&self, album_id: &str) -> Vec<Track> {
        let scanned = self.scanned.read().unwrap();
        let mut tracks: Vec<Track> = scanned
            .tracks
            .iter()
            .filter(|track| track.album_id.as_deref() == Some(album_id))
            .cloned()
            .collect();
        tracks.sort_by(|a, b| {
            (a.disc_number, a.track_number)
                .cmp(&(b.disc_number, b.track_number))
                .then_with(|| a.name.cmp(&b.name))
        });
        tracks
    }

    /// Every artist in the scan, one per distinct artist string, sorted by name.
    fn artists(&self) -> Vec<SavedArtist> {
        let scanned = self.scanned.read().unwrap();
        let mut artists: Vec<SavedArtist> = Vec::new();
        for track in &scanned.tracks {
            if let Some(known) = artists.iter_mut().find(|known| known.name == track.artists) {
                known.added_at = known.added_at.max(track.added_at);
                continue;
            }
            artists.push(SavedArtist {
                id: wire::artist_id(&track.artists),
                name: track.artists.clone(),
                cover: scanned
                    .portraits
                    .get(&track.artists)
                    .cloned()
                    .or_else(|| {
                        scanned
                            .albums
                            .iter()
                            .find(|album| album.artists == track.artists)
                            .and_then(|album| album.cover.clone())
                    })
                    .or_else(|| track.cover.clone()),
                added_at: track.added_at,
            });
        }
        artists.sort_by_key(|artist| artist.name.to_lowercase());
        artists
    }
}

fn playlist_from(id: String, name: String, modified_at: i64, tracks: &[Track]) -> Playlist {
    Playlist {
        id,
        name,
        owner: String::new(),
        owner_id: String::new(),
        owned: true,
        collaborative: false,
        blend: false,
        public: false,
        cover: tracks.iter().find_map(|track| track.cover.clone()),
        track_count: tracks.len() as u32,
        modified_at: Some(modified_at / 1_000),
    }
}

#[async_trait]
impl MusicApi for LocalClient {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        match kind {
            MediaKind::Track => {
                let path = wire::path_from_track_id(id)?;
                Some(format!("file://{}", path.display()))
            }
            MediaKind::Album => {
                let scanned = self.scanned.read().unwrap();
                let track = scanned
                    .tracks
                    .iter()
                    .find(|track| track.album_id.as_deref() == Some(id))?;
                let path = wire::path_from_track_id(track.id.as_deref()?)?;
                let dir = path.parent()?;
                Some(format!("file://{}", dir.display()))
            }
            MediaKind::Artist | MediaKind::Playlist => None,
        }
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(UserProfile {
            id: "local".to_owned(),
            display_name: "Local Files".to_owned(),
            avatar: None,
        })
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let name = wire::artist_name_from_id(artist_id)
            .ok_or_else(|| anyhow!("{artist_id} is not a local artist id"))?;
        let scanned = self.scanned.read().unwrap();
        Ok(Artist {
            name: name.to_owned(),
            cover_large: scanned.portraits.get(name).cloned(),
            biography: None,
            monthly_listeners: None,
            top_tracks: scanned
                .tracks
                .iter()
                .filter(|track| track.artists == name)
                .cloned()
                .collect(),
            albums: scanned
                .albums
                .iter()
                .filter(|album| album.artists == name)
                .cloned()
                .collect(),
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let name = wire::artist_name_from_id(artist_id)
            .ok_or_else(|| anyhow!("{artist_id} is not a local artist id"))?;
        let scanned = self.scanned.read().unwrap();
        Ok(ArtistProfile {
            name: name.to_owned(),
            cover_large: scanned.portraits.get(name).cloned(),
            biography: None,
        })
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let scanned = self.scanned.read().unwrap();
        Ok(ids
            .into_iter()
            .filter_map(|id| {
                let name = wire::artist_name_from_id(&id)?;
                let portrait = scanned.portraits.get(name)?;
                Some((id.clone(), portrait.clone()))
            })
            .collect())
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let starred = self.store.starred(Starred::Tracks)?;
        let scanned = self.scanned.read().unwrap();
        Ok(starred
            .into_iter()
            .filter_map(|(id, added_at)| {
                let mut track = scanned
                    .tracks
                    .iter()
                    .find(|track| track.id.as_deref() == Some(id.as_str()))
                    .cloned()?;
                track.added_at = Some(added_at);
                Some(track)
            })
            .collect())
    }

    async fn all_tracks(&self) -> Result<Vec<Track>> {
        let scanned = self.scanned.read().unwrap();
        Ok(scanned.tracks.clone())
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.store.set_starred(Starred::Tracks, track_id, saved)
    }

    async fn track_tags(&self, track_id: &str) -> Result<TrackTags> {
        let path = wire::path_from_track_id(track_id)
            .ok_or_else(|| anyhow!("{track_id} is not a local track id"))?;
        tags::read(path)
    }

    async fn set_track_tags(&self, track_id: &str, updated: TrackTags) -> Result<()> {
        let path = wire::path_from_track_id(track_id)
            .ok_or_else(|| anyhow!("{track_id} is not a local track id"))?;
        let year_changed = tags::read(path)?.year != updated.year;
        let album_tracks = year_changed.then(|| self.album_track_paths(track_id));

        tags::write(path, &updated)?;
        let mut written = vec![path.to_path_buf()];
        if let Some(album_tracks) = album_tracks {
            for sibling in album_tracks.into_iter().filter(|sibling| sibling != path) {
                tags::write_year(&sibling, &updated.year)?;
                written.push(sibling);
            }
        }
        // A file written in place leaves its folder's time alone, so the next scan would trust
        // the old tags. Dropping the rows here is what makes it read them again.
        self.index.forget(&written);
        Ok(())
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let scanned = self.scanned.read().unwrap();
        scanned
            .tracks
            .iter()
            .find(|track| track.id.as_deref() == Some(track_id))
            .cloned()
            .ok_or_else(|| anyhow!("cannot find local track {track_id}"))
    }

    async fn track_from_path(&self, path: &Path) -> Result<Track> {
        let (track, ..) = wire::track_from_file(path, None, None, &self.cache_dir)
            .ok_or_else(|| anyhow!("cannot read {} as an audio file", path.display()))?;
        Ok(track)
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .map(|stored| {
                let tracks = self
                    .store
                    .tracks(&stored.id)
                    .map(|ids| self.listed(&ids))
                    .unwrap_or_default();
                playlist_from(stored.id, stored.name, stored.modified_at, &tracks)
            })
            .collect())
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        self.store.create(name)
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.store.rename(playlist_id, name)
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.store.delete(playlist_id)
    }

    async fn remove_playlist_from_library(&self, _playlist_id: &str) -> Result<()> {
        Err(anyhow!(NOT_SUPPORTED))
    }

    async fn add_playlist_to_library(&self, _playlist_id: &str) -> Result<()> {
        Err(anyhow!(NOT_SUPPORTED))
    }

    async fn set_playlist_public(&self, _playlist_id: &str, _public: bool) -> Result<()> {
        Err(anyhow!(NOT_SUPPORTED))
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.store.add(playlist_id, track_id)
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.store.remove(playlist_id, track_id)
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let starred = self.store.starred(Starred::Albums)?;
        let scanned = self.scanned.read().unwrap();
        Ok(starred
            .into_iter()
            .filter_map(|(id, added_at)| {
                let mut album = scanned
                    .albums
                    .iter()
                    .find(|album| album.id == id)
                    .cloned()?;
                album.added_at = Some(added_at);
                Some(album)
            })
            .collect())
    }

    async fn all_albums(&self) -> Result<Vec<Album>> {
        let scanned = self.scanned.read().unwrap();
        Ok(scanned.albums.clone())
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.store.set_starred(Starred::Albums, album_id, saved)
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let starred = self.store.starred(Starred::Artists)?;
        let known = self.artists();
        Ok(starred
            .into_iter()
            .filter_map(|(id, added_at)| {
                let mut artist = known.iter().find(|artist| artist.id == id).cloned()?;
                artist.added_at = Some(added_at);
                Some(artist)
            })
            .collect())
    }

    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        Ok(self.artists())
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.store.set_starred(Starred::Artists, artist_id, saved)
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let album = self
            .scanned
            .read()
            .unwrap()
            .albums
            .iter()
            .find(|album| album.id == album_id)
            .cloned()
            .ok_or_else(|| anyhow!("cannot find local album {album_id}"))?;
        Ok(AlbumDetail {
            album,
            tracks: self.album_songs(album_id),
        })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album_songs(album_id))
    }

    /// The other albums carrying the page's artist credit. Nothing here knows one artist
    /// from another beyond the credit string, so similarity stays out.
    async fn album_catalogue(
        &self,
        album_id: &str,
        _artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        let albums = self.all_albums().await?;
        let Some(artists) = albums
            .iter()
            .find(|album| album.id == album_id)
            .map(|album| album.artists.clone())
        else {
            return Ok(AlbumCatalogue::default());
        };
        Ok(AlbumCatalogue {
            also_like: albums
                .into_iter()
                .filter(|album| album.id != album_id && album.artists == artists)
                .take(SUGGESTIONS)
                .collect(),
            similar: Vec::new(),
        })
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        self.assemble(playlist_id)
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self.listed(&self.store.tracks(playlist_id)?))
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = self.listed(&self.store.tracks(playlist_id)?);
        Ok(distinct_covers(&tracks, wanted.max(COVERS)))
    }

    async fn track_radio(
        &self,
        _track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        Ok((Vec::new(), None))
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let query = query.to_lowercase();
        let scanned = self.scanned.read().unwrap();
        Ok(scanned
            .tracks
            .iter()
            .filter(|track| {
                track.name.to_lowercase().contains(&query)
                    || track.artists.to_lowercase().contains(&query)
                    || track.album.to_lowercase().contains(&query)
            })
            .cloned()
            .collect())
    }

    async fn delete_track_file(&self, track_id: &str) -> Result<()> {
        let path = wire::path_from_track_id(track_id)
            .ok_or_else(|| anyhow!("{track_id} is not a local track id"))?;

        self.store.set_starred(Starred::Tracks, track_id, false)?;
        std::fs::remove_file(path).with_context(|| format!("cannot delete {}", path.display()))?;
        Ok(())
    }

    async fn home(&self) -> Result<HomeFeed> {
        let (tracks, albums) = {
            let scanned = self.scanned.read().unwrap();
            (scanned.tracks.clone(), scanned.albums.clone())
        };
        if tracks.is_empty() && albums.is_empty() {
            return Ok(HomeFeed::default());
        }

        let playable: Vec<Track> = tracks
            .into_iter()
            .filter(|t| t.playable && t.id.is_some())
            .collect();

        const PICKS_TARGET: usize = 30;
        let total = PICKS_TARGET.min(playable.len());
        let familiar_target = total / 2;

        let starred_ids = self.store.starred(Starred::Tracks).unwrap_or_default();
        let most_played_ids = self.store.most_played(PICKS_TARGET).unwrap_or_default();

        let mut familiar_candidates = Vec::new();
        let mut seen_ids = HashSet::new();

        for (id, _) in &starred_ids {
            if seen_ids.insert(id.clone()) {
                familiar_candidates.push(id.clone());
            }
        }
        for id in &most_played_ids {
            if seen_ids.insert(id.clone()) {
                familiar_candidates.push(id.clone());
            }
        }

        let mut familiar_tracks: Vec<Track> = familiar_candidates
            .into_iter()
            .filter_map(|id| {
                playable
                    .iter()
                    .find(|t| t.id.as_deref() == Some(&id))
                    .cloned()
            })
            .collect();

        if familiar_tracks.len() > familiar_target {
            fastrand::shuffle(&mut familiar_tracks);
            familiar_tracks.truncate(familiar_target);
        }

        let familiar_set: HashSet<&str> = familiar_tracks
            .iter()
            .filter_map(|t| t.id.as_deref())
            .collect();

        let mut random_pool: Vec<Track> = playable
            .iter()
            .filter(|t| t.id.as_deref().is_none_or(|id| !familiar_set.contains(id)))
            .cloned()
            .collect();
        fastrand::shuffle(&mut random_pool);

        let random_count = total.saturating_sub(familiar_tracks.len());
        let random_tracks: Vec<Track> = random_pool.into_iter().take(random_count).collect();

        let mut quick_tracks = familiar_tracks;
        quick_tracks.extend(random_tracks);
        fastrand::shuffle(&mut quick_tracks);

        let mut sections = Vec::new();

        let mut recent_albums = albums.clone();
        recent_albums.sort_by_key(|album| std::cmp::Reverse(album.added_at.unwrap_or(i64::MIN)));
        let recent_items: Vec<GenreItem> = recent_albums
            .into_iter()
            .take(15)
            .map(GenreItem::Album)
            .collect();
        if !recent_items.is_empty() {
            sections.push(GenreSection {
                title: "home-recently-added".to_owned(),
                items: recent_items,
            });
        }

        let playlists = self.playlists().await.unwrap_or_default();
        if !playlists.is_empty() {
            sections.push(GenreSection {
                title: "home-playlists".to_owned(),
                items: playlists
                    .into_iter()
                    .take(15)
                    .map(GenreItem::Playlist)
                    .collect(),
            });
        }

        let starred_albums = self.saved_albums().await.unwrap_or_default();
        if !starred_albums.is_empty() {
            sections.push(GenreSection {
                title: "home-favorite-albums".to_owned(),
                items: starred_albums
                    .into_iter()
                    .take(15)
                    .map(GenreItem::Album)
                    .collect(),
            });
        }

        let mut artists = self.artists();
        if !artists.is_empty() {
            fastrand::shuffle(&mut artists);
            sections.push(GenreSection {
                title: "home-artists".to_owned(),
                items: artists
                    .into_iter()
                    .take(15)
                    .map(GenreItem::Artist)
                    .collect(),
            });
        }

        if albums.len() > 15 {
            let mut explore_albums = albums.clone();
            fastrand::shuffle(&mut explore_albums);
            sections.push(GenreSection {
                title: "home-collection-albums".to_owned(),
                items: explore_albums
                    .into_iter()
                    .take(15)
                    .map(GenreItem::Album)
                    .collect(),
            });
        }

        let listen_again = quick_tracks
            .iter()
            .take(10)
            .cloned()
            .map(GenreItem::Track)
            .collect();

        Ok(HomeFeed {
            listen_again,
            quick_picks: Some(quick_tracks),
            sections,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ArtistRef;

    fn test_track(id: &str, name: &str, artists: &str, album: &str) -> Track {
        Track {
            id: Some(id.to_owned()),
            name: name.to_owned(),
            playable: true,
            artists: artists.to_owned(),
            artist_refs: vec![ArtistRef {
                name: artists.to_owned(),
                id: None,
            }],
            album: album.to_owned(),
            album_id: None,
            cover: None,
            duration: Duration::from_secs(180),
            added_at: None,
            added_by: None,
            playcount: None,
            popularity: 50,
            explicit: false,
            track_number: 1,
            disc_number: 1,
            tags: Vec::new(),
            languages: Vec::new(),
            credits: Vec::new(),
        }
    }

    #[tokio::test]
    async fn home_feed_quick_picks_mixes_starred_played_and_random() {
        let dir = std::env::temp_dir().join(format!("sonora-test-{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::at(dir.join("state.sqlite"));
        let cache = storage::Cache::at(dir.join("cache.sqlite"));
        let index = Index::new(cache);

        let mut tracks = Vec::new();
        for i in 0..20 {
            tracks.push(test_track(
                &format!("local:{i}"),
                &format!("Track {i}"),
                &format!("Artist {}", i % 5),
                "Album",
            ));
        }

        let scanned = Scanned {
            tracks,
            albums: vec![],
            portraits: HashMap::new(),
        };

        let client = LocalClient::new(scanned, db.clone(), dir.clone(), index);

        // Star track 0 and 1
        client.set_track_saved("local:0", true).await.unwrap();
        client.set_track_saved("local:1", true).await.unwrap();

        // Record a play for track 2 in plays table
        {
            let conn = db.open().unwrap();
            conn.execute(
                "INSERT INTO plays (scope, provider, track_id, played_at, name, playable, artists, artist_refs, album, album_id, cover, duration_ms, explicit)
                 VALUES ('youtube:youtube-guest', 'local', 'local:2', 1000, 'Track 2', 1, 'Artist 2', '[]', 'Album', NULL, NULL, 180000, 0)",
                [],
            ).unwrap();
        }

        let feed = client.home().await.unwrap();
        let quick = feed.quick_picks.expect("quick picks present");
        assert_eq!(quick.len(), 20);

        // Starred tracks (0, 1) and most played track (2) should be present
        let ids: Vec<&str> = quick.iter().filter_map(|t| t.id.as_deref()).collect();
        assert!(ids.contains(&"local:0"));
        assert!(ids.contains(&"local:1"));
        assert!(ids.contains(&"local:2"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
