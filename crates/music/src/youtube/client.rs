use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use tokio::task::JoinSet;
use ytmusic::YtMusic;

use crate::youtube::{genres, subscriptions, wire};
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, Genre, GenreDetail, HomeFeed, MediaKind, MusicApi,
    Playlist, PlaylistDetail, PlaylistEntry, SavedArtist, Track, UserProfile,
};

const PORTRAIT_LIMIT: usize = 24;
const EPISODES: &str = "SE";

pub struct YouTubeClient {
    api: Arc<YtMusic>,
    account: String,
}

impl YouTubeClient {
    pub fn new(api: Arc<YtMusic>) -> Self {
        Self {
            api,
            account: String::new(),
        }
    }

    pub fn owned_by(mut self, account: impl Into<String>) -> Self {
        self.account = account.into();
        self
    }

    async fn hydrate_durations(&self, tracks: &mut [ytmusic::Track]) {
        let mut tasks = JoinSet::new();
        for (index, track) in tracks.iter().enumerate() {
            if track.duration.is_some() {
                continue;
            }
            let Some(video_id) = track.video_id.clone() else {
                continue;
            };
            let api = self.api.clone();
            tasks.spawn(async move { (index, api.track_duration(&video_id).await) });
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok((index, Some(duration))) = result {
                tracks[index].duration = Some(duration);
            }
        }
    }
}

#[async_trait]
impl MusicApi for YouTubeClient {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let url = match kind {
            MediaKind::Track => format!("https://music.youtube.com/watch?v={id}"),
            MediaKind::Album => format!("https://music.youtube.com/browse/{id}"),
            MediaKind::Artist => format!("https://music.youtube.com/channel/{id}"),
            MediaKind::Playlist => format!("https://music.youtube.com/playlist?list={id}"),
        };
        Some(url)
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(wire::profile(self.api.profile().await?))
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let mut artist = self.api.artist(artist_id).await?;
        self.hydrate_durations(&mut artist.top_tracks).await;
        Ok(wire::artist(artist))
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        Ok(wire::artist_profile(&self.api.artist(artist_id).await?))
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let mut tasks = JoinSet::new();
        for id in ids.into_iter().take(PORTRAIT_LIMIT) {
            let api = self.api.clone();
            tasks.spawn(async move {
                let artist = api.artist(&id).await.ok()?;
                let image = wire::cover_large(&artist.thumbnails)?;
                Some((id, image))
            });
        }
        let mut images = HashMap::new();
        while let Some(result) = tasks.join_next().await {
            if let Ok(Some((id, image))) = result {
                images.insert(id, image);
            }
        }
        Ok(images)
    }

    async fn saved_tracks(&self, limit: u32) -> Result<Vec<Track>> {
        let mut tracks: Vec<Track> = self
            .api
            .liked_songs_resolved()
            .await?
            .into_iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect();
        tracks.truncate(limit as usize);
        Ok(tracks)
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.api.rate_track(track_id, saved).await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let response = self
            .api
            .player_response(track_id, ytmusic::Client::Music)
            .await?;
        let details = response
            .get("videoDetails")
            .context("player response has no video details")?;
        let title = details
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let author = details
            .get("author")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let seconds: u64 = details
            .get("lengthSeconds")
            .and_then(serde_json::Value::as_str)
            .and_then(|length| length.parse().ok())
            .unwrap_or(0);
        let channel = details
            .get("channelId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let source = ytmusic::Track {
            video_id: Some(track_id.to_string()),
            title,
            artists: vec![ytmusic::ArtistRef {
                name: author,
                id: channel,
            }],
            album: None,
            duration: Some(std::time::Duration::from_secs(seconds)),
            thumbnails: Vec::new(),
            explicit: false,
            available: true,
            kind: ytmusic::TrackKind::Song,
            set_video_id: None,
            liked: None,
            views: None,
        };
        let mut track = wire::track(source, 0);
        track.cover = details
            .get("thumbnail")
            .map(|thumbnail| {
                let thumbs = collect_thumbnails(thumbnail);
                wire::cover(&thumbs)
            })
            .unwrap_or_default();
        Ok(track)
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn playlists(&self, limit: u32) -> Result<Vec<PlaylistEntry>> {
        let mut playlists: Vec<Playlist> = self
            .api
            .library_playlists()
            .await?
            .into_iter()
            .filter(|playlist| playlist.id != EPISODES)
            .map(|playlist| {
                let mut playlist = wire::playlist(playlist, false, false);
                if playlist.owned && playlist.owner.is_empty() {
                    playlist.owner = self.account.clone();
                }
                playlist
            })
            .collect();
        playlists.truncate(limit as usize);
        Ok(PlaylistEntry::flat(playlists))
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        self.api.create_playlist(name).await
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.api.rename_playlist(playlist_id, name).await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.api.delete_playlist(playlist_id).await
    }

    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        self.api.rate_playlist(playlist_id, false).await
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        self.api.rate_playlist(playlist_id, true).await
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        self.api.set_playlist_privacy(playlist_id, public).await
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.api.add_playlist_track(playlist_id, track_id).await
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let tracks = self.api.playlist(playlist_id).await?.tracks;
        let seat = |list: &[ytmusic::Track]| {
            list.iter()
                .position(|track| track.video_id.as_deref() == Some(track_id))
        };
        let index = match seat(&tracks) {
            Some(index) => index,
            None => seat(&self.api.swap_playable(tracks.clone()).await)
                .context("track is not in the playlist")?,
        };
        let track = tracks.get(index).context("track is not in the playlist")?;
        let video_id = track.video_id.as_deref().context("track has no video id")?;
        let set_video_id = track
            .set_video_id
            .as_deref()
            .context("track cannot be removed from this playlist")?;
        self.api
            .remove_playlist_track(playlist_id, video_id, set_video_id)
            .await
    }

    async fn saved_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let mut albums: Vec<Album> = self
            .api
            .library_albums()
            .await?
            .into_iter()
            .map(wire::album)
            .collect();
        albums.truncate(limit as usize);
        Ok(albums)
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        let detail = self.api.album(album_id).await?;
        let playlist_id = detail
            .album
            .playlist_id
            .context("album has no audio playlist")?;
        self.api
            .rate_playlist(&playlist_id, saved)
            .await
            .with_context(|| format!("cannot rate the album {album_id} as {playlist_id}"))
    }

    async fn saved_artists(&self, limit: u32) -> Result<Vec<SavedArtist>> {
        subscriptions::saved(&self.api, limit).await
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        subscriptions::set_saved(&self.api, artist_id, saved).await
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let mut detail = self.api.album(album_id).await?;
        detail.tracks = self.api.swap_playable(detail.tracks).await;
        Ok(wire::album_detail(detail))
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let mut detail = self.api.playlist_page(playlist_id).await?;
        detail.tracks = self.api.swap_playable(detail.tracks).await;
        Ok(wire::playlist_detail(detail))
    }

    async fn playlist_continuation(
        &self,
        continuation: &str,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let mut page = self.api.playlist_continuation(continuation).await?;
        page.tracks = self.api.swap_playable(page.tracks).await;
        Ok(wire::playlist_page(page))
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let mut detail = self.api.playlist(playlist_id).await?;
        detail.tracks = self.api.swap_playable(detail.tracks).await;
        Ok(wire::playlist_detail(detail).tracks)
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let mut tracks = self.api.playlist_page(playlist_id).await?.tracks;
        tracks = self.api.swap_playable(tracks).await;
        let tracks: Vec<Track> = tracks
            .into_iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect();
        Ok(crate::distinct_covers(&tracks, wanted))
    }

    async fn track_radio(&self, track_id: &str) -> Result<Vec<Track>> {
        Ok(self
            .api
            .track_radio(track_id)
            .await?
            .into_iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        Ok(self
            .api
            .search_songs(query)
            .await?
            .into_iter()
            .enumerate()
            .map(|(index, track)| wire::track(track, index as u32))
            .collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        Ok(self
            .api
            .search_albums(query)
            .await?
            .into_iter()
            .map(wire::album)
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        Ok(self
            .api
            .search_playlists(query)
            .await?
            .into_iter()
            .map(|playlist| wire::playlist(playlist, false, false))
            .collect())
    }

    async fn home(&self) -> Result<HomeFeed> {
        genres::home(&self.api).await
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        genres::genres(&self.api).await
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        genres::genre(&self.api, genre_id).await
    }
}

fn collect_thumbnails(node: &serde_json::Value) -> Vec<ytmusic::Thumbnail> {
    node.get("thumbnails")
        .and_then(serde_json::Value::as_array)
        .map(|thumbs| {
            thumbs
                .iter()
                .filter_map(|thumb| {
                    Some(ytmusic::Thumbnail {
                        url: thumb.get("url")?.as_str()?.to_string(),
                        width: thumb.get("width").and_then(serde_json::Value::as_u64)? as u32,
                        height: thumb.get("height").and_then(serde_json::Value::as_u64)? as u32,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}
