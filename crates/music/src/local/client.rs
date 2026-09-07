use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use storage::Database;

use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, MediaKind, MusicApi, Playlist, PlaylistDetail,
    SavedArtist, Track, TrackTags, UserProfile, distinct_covers,
};

use super::scan::Scanned;
use super::store::Store;
use super::{tags, wire};

const COVERS: usize = 4;
const NOT_SUPPORTED: &str = "local playlists are not shared";

pub struct LocalClient {
    scanned: RwLock<Scanned>,
    store: Store,
}

impl LocalClient {
    pub fn new(scanned: Scanned, database: Database) -> Self {
        Self {
            scanned: RwLock::new(scanned),
            store: Store::new(database),
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
        modified_at: Some(modified_at),
    }
}

#[async_trait]
impl MusicApi for LocalClient {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let path = match kind {
            MediaKind::Track => wire::path_from_track_id(id)?,
            MediaKind::Album => wire::path_from_album_id(id)?,
            MediaKind::Artist | MediaKind::Playlist => return None,
        };
        Some(format!("file://{}", path.display()))
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(UserProfile {
            id: "local".to_owned(),
            display_name: "Local Files".to_owned(),
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

    async fn saved_tracks(&self, limit: u32) -> Result<Vec<Track>> {
        let favorites = self.store.favorites()?;
        let scanned = self.scanned.read().unwrap();
        Ok(favorites
            .into_iter()
            .take(limit as usize)
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

    async fn all_tracks(&self, limit: u32) -> Result<Vec<Track>> {
        let scanned = self.scanned.read().unwrap();
        Ok(scanned
            .tracks
            .iter()
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.store.set_favorite(track_id, saved)
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
        if let Some(album_tracks) = album_tracks {
            for sibling in album_tracks.into_iter().filter(|sibling| sibling != path) {
                tags::write_year(&sibling, &updated.year)?;
            }
        }
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

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn playlists(&self, limit: u32) -> Result<Vec<Playlist>> {
        Ok(self
            .store
            .list()?
            .into_iter()
            .take(limit as usize)
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

    async fn saved_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let scanned = self.scanned.read().unwrap();
        Ok(scanned
            .albums
            .iter()
            .take(limit as usize)
            .cloned()
            .collect())
    }

    async fn set_album_saved(&self, _album_id: &str, _saved: bool) -> Result<()> {
        Ok(())
    }

    async fn saved_artists(&self, limit: u32) -> Result<Vec<SavedArtist>> {
        let scanned = self.scanned.read().unwrap();
        let mut artists: Vec<SavedArtist> = Vec::new();
        for track in &scanned.tracks {
            if artists.iter().any(|known| known.name == track.artists) {
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
                added_at: None,
            });
        }
        artists.sort_by_key(|artist| artist.name.to_lowercase());
        artists.truncate(limit as usize);
        Ok(artists)
    }

    async fn set_artist_saved(&self, _artist_id: &str, _saved: bool) -> Result<()> {
        Ok(())
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let scanned = self.scanned.read().unwrap();
        let album = scanned
            .albums
            .iter()
            .find(|album| album.id == album_id)
            .cloned()
            .ok_or_else(|| anyhow!("cannot find local album {album_id}"))?;
        let tracks = scanned
            .tracks
            .iter()
            .filter(|track| track.album_id.as_deref() == Some(album_id))
            .cloned()
            .collect();
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        let scanned = self.scanned.read().unwrap();
        Ok(scanned
            .tracks
            .iter()
            .filter(|track| track.album_id.as_deref() == Some(album_id))
            .cloned()
            .collect())
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

    async fn track_radio(&self, _track_id: &str) -> Result<Vec<Track>> {
        Ok(Vec::new())
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
}
