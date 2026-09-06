use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use anyhow::{Result, anyhow};
use async_trait::async_trait;
use moka::{future::Cache, sync::Cache as SyncCache};
use music::{AlbumDetail, Artist, ArtistProfile, GenreDetail, MusicApi, PlaylistDetail, Track};

const ARTISTS: usize = 32;
const ARTIST_PROFILES: usize = 64;
const ALBUMS: usize = 48;
const PLAYLISTS: usize = 16;
const SONGS: usize = 64;
const GENRES: usize = 24;

#[async_trait]
trait CatalogBackend: Send + Sync {
    async fn artist(&self, id: &str) -> Result<Artist>;
    async fn artist_profile(&self, id: &str) -> Result<ArtistProfile>;
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>>;
    async fn track(&self, id: &str) -> Result<Track>;
    async fn track_playcount(&self, id: &str) -> Result<Option<u64>>;
    async fn album(&self, id: &str) -> Result<AlbumDetail>;
    async fn playlist(&self, id: &str) -> Result<PlaylistDetail>;
    async fn genre(&self, id: &str) -> Result<GenreDetail>;
}

struct ApiBackend(Arc<dyn MusicApi>);

#[async_trait]
impl CatalogBackend for ApiBackend {
    async fn artist(&self, id: &str) -> Result<Artist> {
        self.0.artist(id).await
    }
    async fn artist_profile(&self, id: &str) -> Result<ArtistProfile> {
        self.0.artist_profile(id).await
    }
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        self.0.artist_images(ids).await
    }
    async fn track(&self, id: &str) -> Result<Track> {
        self.0.track(id).await
    }
    async fn track_playcount(&self, id: &str) -> Result<Option<u64>> {
        self.0.track_playcount(id).await
    }
    async fn album(&self, id: &str) -> Result<AlbumDetail> {
        self.0.album(id).await
    }
    async fn playlist(&self, id: &str) -> Result<PlaylistDetail> {
        self.0.playlist(id).await
    }
    async fn genre(&self, id: &str) -> Result<GenreDetail> {
        self.0.genre(id).await
    }
}

pub(crate) struct CatalogCache<T> {
    cache: Cache<String, Arc<T>>,
    ready: SyncCache<String, Weak<T>>,
}

impl<T: Send + Sync + 'static> CatalogCache<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            cache: Cache::new(capacity as u64),
            ready: SyncCache::new(capacity as u64),
        }
    }

    pub(crate) fn peek(&self, id: &str) -> Option<Arc<T>> {
        match self.ready.get(id).and_then(|value| value.upgrade()) {
            Some(value) => Some(value),
            None => {
                self.ready.invalidate(id);
                None
            }
        }
    }

    pub(crate) async fn load<F>(&self, id: &str, load: F) -> Result<Arc<T>>
    where
        F: Future<Output = Result<T>>,
    {
        let value = self
            .cache
            .try_get_with(id.to_owned(), async { load.await.map(Arc::new) })
            .await
            .map_err(|error| anyhow!("{error:#}"))?;
        self.ready.insert(id.to_owned(), Arc::downgrade(&value));
        Ok(value)
    }

    pub(crate) async fn invalidate(&self, id: &str) {
        self.ready.invalidate(id);
        self.cache.invalidate(id).await;
    }
}

pub(crate) struct SongPage {
    pub(crate) track: Track,
    pub(crate) album: Option<Arc<AlbumDetail>>,
    pub(crate) artist: Option<Arc<ArtistProfile>>,
    pub(crate) portraits: HashMap<String, String>,
    pub(crate) playcount: Option<u64>,
}

pub(crate) struct CatalogSource {
    backend: Arc<dyn CatalogBackend>,
    artists: CatalogCache<Artist>,
    artist_profiles: CatalogCache<ArtistProfile>,
    albums: CatalogCache<AlbumDetail>,
    playlists: CatalogCache<PlaylistDetail>,
    songs: CatalogCache<SongPage>,
    genres: CatalogCache<GenreDetail>,
}

impl CatalogSource {
    pub(crate) fn new(client: Arc<dyn MusicApi>) -> Self {
        Self::from_backend(Arc::new(ApiBackend(client)))
    }

    fn from_backend(backend: Arc<dyn CatalogBackend>) -> Self {
        Self {
            backend,
            artists: CatalogCache::new(ARTISTS),
            artist_profiles: CatalogCache::new(ARTIST_PROFILES),
            albums: CatalogCache::new(ALBUMS),
            playlists: CatalogCache::new(PLAYLISTS),
            songs: CatalogCache::new(SONGS),
            genres: CatalogCache::new(GENRES),
        }
    }

    #[cfg(test)]
    fn test(backend: Arc<dyn CatalogBackend>) -> Self {
        Self::from_backend(backend)
    }

    pub(crate) fn peek_artist(&self, id: &str) -> Option<Arc<Artist>> {
        self.artists.peek(id)
    }

    pub(crate) async fn artist(&self, id: &str) -> Result<Arc<Artist>> {
        self.artists.load(id, self.backend.artist(id)).await
    }

    pub(crate) async fn artist_profile(&self, id: &str) -> Result<Arc<ArtistProfile>> {
        self.artist_profiles
            .load(id, self.backend.artist_profile(id))
            .await
    }

    pub(crate) fn peek_album(&self, id: &str) -> Option<Arc<AlbumDetail>> {
        self.albums.peek(id)
    }

    pub(crate) async fn album(&self, id: &str) -> Result<Arc<AlbumDetail>> {
        self.albums.load(id, self.backend.album(id)).await
    }

    pub(crate) fn peek_playlist(&self, id: &str) -> Option<Arc<PlaylistDetail>> {
        self.playlists.peek(id)
    }

    pub(crate) async fn playlist(&self, id: &str) -> Result<Arc<PlaylistDetail>> {
        self.playlists.load(id, self.backend.playlist(id)).await
    }

    pub(crate) async fn invalidate_playlist(&self, id: &str) {
        self.playlists.invalidate(id).await;
    }

    pub(crate) fn peek_genre(&self, id: &str) -> Option<Arc<GenreDetail>> {
        self.genres.peek(id)
    }

    pub(crate) async fn genre(&self, id: &str) -> Result<Arc<GenreDetail>> {
        self.genres.load(id, self.backend.genre(id)).await
    }

    pub(crate) fn peek_song(&self, id: &str) -> Option<Arc<SongPage>> {
        self.songs.peek(id)
    }

    pub(crate) async fn song(&self, id: &str) -> Result<Arc<SongPage>> {
        self.songs
            .load(id, async {
                let track = self.backend.track(id).await?;
                let album_id = track.album_id.clone();
                let artist_id = track
                    .artist_refs
                    .first()
                    .and_then(|artist| artist.id.clone());
                let mut credit_ids = track
                    .credits
                    .iter()
                    .filter_map(|credit| credit.id.clone())
                    .chain(
                        track
                            .artist_refs
                            .iter()
                            .filter_map(|artist| artist.id.clone()),
                    )
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                if let Some(artist_id) = artist_id.as_deref() {
                    credit_ids.retain(|id| id != artist_id);
                }
                let primary_artist = artist_id.clone();
                let album = async {
                    match album_id {
                        Some(album_id) => self.album(&album_id).await.ok(),
                        None => None,
                    }
                };
                let artist = async {
                    match artist_id {
                        Some(artist_id) => self.artist_profile(&artist_id).await.ok(),
                        None => None,
                    }
                };
                let portraits = async {
                    match credit_ids.is_empty() {
                        true => HashMap::new(),
                        false => self
                            .backend
                            .artist_images(credit_ids)
                            .await
                            .unwrap_or_default(),
                    }
                };
                let (album, artist, mut portraits) = tokio::join!(album, artist, portraits);
                if let (Some(id), Some(cover)) = (
                    primary_artist,
                    artist
                        .as_ref()
                        .and_then(|artist| artist.cover_large.clone()),
                ) {
                    portraits.insert(id, cover);
                }
                let album_track = album.as_ref().and_then(|album| {
                    album
                        .tracks
                        .iter()
                        .find(|album_track| album_track.id == track.id)
                });
                let playcount = match (album_track, track.id.as_deref()) {
                    (Some(album_track), _) => album_track.playcount,
                    (None, Some(track_id)) => match self.backend.track_playcount(track_id).await {
                        Ok(playcount) => playcount,
                        Err(error) => {
                            log::warn!("song: cannot read track play count: {error:#}");
                            None
                        }
                    },
                    (None, None) => None,
                };
                Ok(SongPage {
                    track,
                    album,
                    artist,
                    portraits,
                    playcount,
                })
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, atomic::AtomicBool};
    use std::time::Duration;

    use anyhow::anyhow;
    use async_trait::async_trait;
    use music::{
        Album, AlbumDetail, Artist, ArtistProfile, ArtistRef, GenreDetail, Playlist,
        PlaylistDetail, ReleaseType, Track,
    };

    use super::{CatalogBackend, CatalogCache, CatalogSource};

    struct FakeBackend {
        label: &'static str,
        calls: Mutex<HashMap<String, usize>>,
        fail_optional: AtomicBool,
    }

    impl FakeBackend {
        fn new(label: &'static str) -> Self {
            Self {
                label,
                calls: Mutex::new(HashMap::new()),
                fail_optional: AtomicBool::new(false),
            }
        }

        fn failing_optional(label: &'static str) -> Self {
            Self {
                fail_optional: AtomicBool::new(true),
                ..Self::new(label)
            }
        }

        fn called(&self, method: &str, id: &str) -> usize {
            *self
                .calls
                .lock()
                .unwrap()
                .get(&format!("{method}:{id}"))
                .unwrap_or(&0)
        }

        fn count(&self, method: &str, id: &str) {
            *self
                .calls
                .lock()
                .unwrap()
                .entry(format!("{method}:{id}"))
                .or_default() += 1;
        }

        fn track_value(&self, id: &str) -> Track {
            Track {
                id: Some(id.to_owned()),
                name: format!("{}:{id}", self.label),
                playable: true,
                artists: "Artist".to_owned(),
                artist_refs: vec![ArtistRef {
                    name: "Artist".to_owned(),
                    id: Some("artist".to_owned()),
                }],
                album: "Album".to_owned(),
                album_id: Some("album".to_owned()),
                cover: None,
                cover_large: None,
                duration: Duration::ZERO,
                added_at: None,
                added_by: None,
                playcount: None,
                popularity: 0,
                explicit: false,
                track_number: 1,
                disc_number: 1,
                tags: Vec::new(),
                languages: Vec::new(),
                credits: Vec::new(),
            }
        }

        fn album_value(&self, id: &str) -> AlbumDetail {
            AlbumDetail {
                album: Album {
                    id: id.to_owned(),
                    name: format!("{}:{id}", self.label),
                    artists: String::new(),
                    artist_refs: Vec::new(),
                    cover: None,
                    cover_large: None,
                    release_type: ReleaseType::Album,
                    year: 0,
                    track_count: 0,
                    release_date: String::new(),
                    label: String::new(),
                    copyrights: Vec::new(),
                    added_at: None,
                },
                tracks: Vec::new(),
            }
        }

        fn playlist_value(&self, id: &str) -> PlaylistDetail {
            PlaylistDetail {
                playlist: Playlist {
                    id: id.to_owned(),
                    name: format!("{}:{id}", self.label),
                    owner: String::new(),
                    owner_id: String::new(),
                    owned: false,
                    collaborative: false,
                    blend: false,
                    public: false,
                    cover: None,
                    track_count: 0,
                    modified_at: None,
                },
                tracks: Vec::new(),
            }
        }
    }

    #[async_trait]
    impl CatalogBackend for FakeBackend {
        async fn artist(&self, id: &str) -> anyhow::Result<Artist> {
            self.count("artist", id);
            Ok(Artist {
                name: format!("{}:{id}", self.label),
                cover_large: None,
                biography: None,
                monthly_listeners: None,
                top_tracks: Vec::new(),
                albums: Vec::new(),
            })
        }

        async fn artist_profile(&self, id: &str) -> anyhow::Result<ArtistProfile> {
            self.count("artist_profile", id);
            if self.fail_optional.load(Ordering::Relaxed) {
                return Err(anyhow!("profile failed"));
            }
            Ok(ArtistProfile {
                name: format!("{}:{id}", self.label),
                cover_large: None,
                biography: None,
            })
        }

        async fn artist_images(
            &self,
            _ids: Vec<String>,
        ) -> anyhow::Result<HashMap<String, String>> {
            if self.fail_optional.load(Ordering::Relaxed) {
                Err(anyhow!("images failed"))
            } else {
                Ok(HashMap::new())
            }
        }

        async fn track(&self, id: &str) -> anyhow::Result<Track> {
            self.count("track", id);
            Ok(self.track_value(id))
        }
        async fn track_playcount(&self, _id: &str) -> anyhow::Result<Option<u64>> {
            if self.fail_optional.load(Ordering::Relaxed) {
                Err(anyhow!("playcount failed"))
            } else {
                Ok(None)
            }
        }
        async fn album(&self, id: &str) -> anyhow::Result<AlbumDetail> {
            self.count("album", id);
            if self.fail_optional.load(Ordering::Relaxed) {
                Err(anyhow!("album failed"))
            } else {
                Ok(self.album_value(id))
            }
        }
        async fn playlist(&self, id: &str) -> anyhow::Result<PlaylistDetail> {
            self.count("playlist", id);
            Ok(self.playlist_value(id))
        }
        async fn genre(&self, id: &str) -> anyhow::Result<GenreDetail> {
            self.count("genre", id);
            Ok(GenreDetail {
                name: format!("{}:{id}", self.label),
                sections: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn hits_and_retries_errors() {
        let cache = CatalogCache::new(2);
        let calls = AtomicUsize::new(0);
        let first = cache
            .load("a", async {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(7)
            })
            .await
            .unwrap();
        let second = cache
            .load("a", async {
                calls.fetch_add(1, Ordering::Relaxed);
                Ok(8)
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        assert!(
            cache
                .load("bad", async { Err(anyhow!("bad")) })
                .await
                .is_err()
        );
        assert_eq!(*cache.load("bad", async { Ok(9) }).await.unwrap(), 9);
    }

    #[tokio::test]
    async fn artist_navigation_reuses_the_first_artist() {
        let api = Arc::new(FakeBackend::new("remote"));
        let catalog = CatalogSource::test(api.clone());
        catalog.artist("a").await.unwrap();
        catalog.artist("b").await.unwrap();
        catalog.artist("a").await.unwrap();
        assert_eq!(api.called("artist", "a"), 1);
        assert_eq!(api.called("artist", "b"), 1);
    }

    #[tokio::test]
    async fn song_reuses_an_album_loaded_by_album_detail() {
        let api = Arc::new(FakeBackend::new("remote"));
        let catalog = CatalogSource::test(api.clone());
        let album = catalog.album("album").await.unwrap();
        let song = catalog.song("song").await.unwrap();
        assert!(Arc::ptr_eq(&album, song.album.as_ref().unwrap()));
        assert_eq!(api.called("album", "album"), 1);
    }

    #[tokio::test]
    async fn song_keeps_its_required_track_when_enrichment_fails() {
        let api = Arc::new(FakeBackend::failing_optional("remote"));
        let catalog = CatalogSource::test(api);
        let song = catalog.song("song").await.unwrap();
        assert_eq!(song.track.id.as_deref(), Some("song"));
        assert!(song.album.is_none());
        assert!(song.artist.is_none());
        assert!(song.portraits.is_empty());
        assert_eq!(song.playcount, None);
    }

    #[tokio::test]
    async fn playlist_invalidation_is_scoped_to_one_id() {
        let api = Arc::new(FakeBackend::new("remote"));
        let catalog = CatalogSource::test(api.clone());
        catalog.playlist("one").await.unwrap();
        catalog.playlist("two").await.unwrap();
        catalog.invalidate_playlist("one").await;
        catalog.playlist("one").await.unwrap();
        catalog.playlist("two").await.unwrap();
        assert_eq!(api.called("playlist", "one"), 2);
        assert_eq!(api.called("playlist", "two"), 1);
    }

    #[tokio::test]
    async fn replacing_a_source_cannot_return_the_old_sessions_values() {
        let old = CatalogSource::test(Arc::new(FakeBackend::new("old")));
        assert_eq!(old.artist("same").await.unwrap().name, "old:same");
        let new = CatalogSource::test(Arc::new(FakeBackend::new("new")));
        assert_eq!(new.artist("same").await.unwrap().name, "new:same");
    }

    #[tokio::test]
    async fn empty_details_are_cache_hits() {
        let api = Arc::new(FakeBackend::new("remote"));
        let catalog = CatalogSource::test(api.clone());
        for _ in 0..2 {
            assert!(catalog.album("empty").await.unwrap().tracks.is_empty());
            assert!(catalog.playlist("empty").await.unwrap().tracks.is_empty());
            assert!(catalog.genre("empty").await.unwrap().sections.is_empty());
        }
        assert_eq!(api.called("album", "empty"), 1);
        assert_eq!(api.called("playlist", "empty"), 1);
        assert_eq!(api.called("genre", "empty"), 1);
    }
}
