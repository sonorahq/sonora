use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use opensubsonic::api::lists::AlbumListType;
use opensubsonic::data::{AlbumId3, AlbumWithSongsId3, Child, Genre as SourceGenre};
use opensubsonic::{Auth, Client};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::task::JoinSet;

use crate::subsonic::auth::Signature;
use crate::subsonic::wire;
use crate::{
    Album, AlbumDetail, Artist, ArtistProfile, Genre, GenreDetail, GenreItem, GenreSection,
    HomeFeed, MediaKind, MusicApi, Playlist, PlaylistDetail, SavedArtist, Track, UserProfile,
    distinct_covers,
};

const PORTRAIT_LIMIT: usize = 24;
const RADIO_COUNT: i32 = 25;
const HOME_SONGS: i32 = 25;
const HOME_ALBUMS: i32 = 12;
const LIBRARY_PAGE: i32 = 500;
const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "sonora";

#[derive(Clone)]
pub struct SubsonicClient {
    client: Client,
    username: String,
    http: reqwest::Client,
    /// The cover art endpoint with the fixed signature already on it. The library client signs
    /// every url afresh, which would give one cover a new url on every conversion and defeat
    /// every image cache between here and the screen.
    covers: String,
}

impl SubsonicClient {
    pub fn new(
        server: String,
        username: String,
        password: String,
        signature: &Signature,
    ) -> Result<Self> {
        let server = server.trim_end_matches('/').to_owned();
        let client = Client::new(&server, Auth::token(&username, password))
            .context("cannot parse the subsonic server address")?
            .with_client_name(CLIENT_NAME);
        let covers = format!(
            "{server}/rest/getCoverArt?u={}&t={}&s={}&v={API_VERSION}&c={CLIENT_NAME}&f=json",
            utf8_percent_encode(&username, NON_ALPHANUMERIC),
            utf8_percent_encode(&signature.token, NON_ALPHANUMERIC),
            utf8_percent_encode(&signature.salt, NON_ALPHANUMERIC),
        );
        Ok(Self {
            client,
            username,
            http: reqwest::Client::new(),
            covers,
        })
    }

    fn cover_url(&self, id: &str, size: i32) -> Option<String> {
        Some(format!(
            "{}&id={}&size={size}",
            self.covers,
            utf8_percent_encode(id, NON_ALPHANUMERIC)
        ))
    }

    fn cover(&self, art: Option<&str>, fallback: &str) -> Option<String> {
        self.cover_url(art.filter(|id| !id.is_empty()).unwrap_or(fallback), 300)
    }

    fn cover_large(&self, art: Option<&str>, fallback: &str) -> Option<String> {
        self.cover_url(art.filter(|id| !id.is_empty()).unwrap_or(fallback), 600)
    }

    fn song(&self, song: Child) -> Track {
        let cover = self.cover(song.cover_art.as_deref(), &song.id);
        wire::track(song, cover)
    }

    fn convert_album(&self, source: AlbumId3) -> Album {
        let cover = self.cover(source.cover_art.as_deref(), &source.id);
        let large = self.cover_large(source.cover_art.as_deref(), &source.id);
        wire::album(source, cover, large)
    }

    fn detail_album(&self, detail: &AlbumWithSongsId3, tracks: usize) -> Album {
        let id = detail.id.clone();
        let cover = self.cover(detail.cover_art.as_deref(), &id);
        let large = self.cover_large(detail.cover_art.as_deref(), &id);
        let (artists, artist_refs) = wire::artists_of(
            detail.artist.clone(),
            detail.artist_id.clone(),
            detail.artists.as_ref(),
            detail.display_artist.clone(),
        );
        let year = detail.year.unwrap_or(0);
        Album {
            id,
            name: detail.name.clone(),
            artists,
            artist_refs,
            cover,
            cover_large: large,
            release_type: crate::ReleaseType::Album,
            year,
            track_count: detail
                .song_count
                .map(|count| count.max(0) as u32)
                .unwrap_or(tracks as u32),
            release_date: match year {
                0 => String::new(),
                _ => year.to_string(),
            },
            label: String::new(),
            copyrights: Vec::new(),
            added_at: None,
        }
    }

    fn convert_playlist(&self, source: &opensubsonic::data::Playlist) -> Playlist {
        let cover = source
            .cover_art
            .as_deref()
            .and_then(|id| self.cover_url(id, 300));
        wire::playlist(
            &source.id,
            &source.name,
            source.owner.as_deref(),
            source.public.unwrap_or(false),
            source.song_count.unwrap_or(0).max(0) as u32,
            cover,
            &self.username,
        )
    }

    fn artist_cover(
        &self,
        art: Option<&str>,
        image: Option<&str>,
        fallback: &str,
    ) -> Option<String> {
        if let Some(url) = image.filter(|url| !url.is_empty()) {
            return Some(url.to_owned());
        }
        self.cover_large(art, fallback)
    }
}

#[async_trait]
impl MusicApi for SubsonicClient {
    fn share_url(&self, _kind: MediaKind, _id: &str) -> Option<String> {
        None
    }

    async fn profile(&self) -> Result<UserProfile> {
        self.client
            .ping()
            .await
            .context("cannot reach the subsonic server")?;
        Ok(wire::profile(self.username.clone()))
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let detail = self
            .client
            .get_artist(artist_id)
            .await
            .context("cannot load the artist")?;
        let name = detail.name.clone();
        let cover_large = self.artist_cover(
            detail.cover_art.as_deref(),
            detail.artist_image_url.as_deref(),
            artist_id,
        );

        let biography = self
            .client
            .get_artist_info2(artist_id, None, None)
            .await
            .ok()
            .and_then(|info| info.biography)
            .filter(|bio| !bio.trim().is_empty());

        let mut top_tracks = match self.client.get_top_songs(&name, Some(20)).await {
            Ok(songs) => songs.into_iter().map(|song| self.song(song)).collect(),
            Err(_) => Vec::new(),
        };
        if top_tracks.is_empty()
            && let Ok(similar) = self
                .client
                .search3(&name, None, None, None, None, Some(10), None, None)
                .await
        {
            top_tracks = similar
                .song
                .into_iter()
                .map(|song| self.song(song))
                .collect();
        }

        let albums = detail
            .album
            .into_iter()
            .map(|album| self.convert_album(album))
            .collect();

        Ok(Artist {
            name,
            cover_large,
            biography,
            monthly_listeners: None,
            top_tracks,
            albums,
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let artist = self.artist(artist_id).await?;
        Ok(ArtistProfile {
            name: artist.name,
            cover_large: artist.cover_large,
            biography: artist.biography,
        })
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        let mut tasks = JoinSet::new();
        for id in ids.into_iter().take(PORTRAIT_LIMIT) {
            let client = self.clone();
            tasks.spawn(async move {
                let detail = client.client.get_artist(&id).await.ok()?;
                let cover = client.artist_cover(
                    detail.cover_art.as_deref(),
                    detail.artist_image_url.as_deref(),
                    &id,
                )?;
                Some((id, cover))
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
        let starred = self
            .client
            .get_starred2(None)
            .await
            .context("cannot load the starred songs")?;
        let mut tracks: Vec<Track> = starred
            .song
            .into_iter()
            .map(|song| self.song(song))
            .collect();
        tracks.truncate(limit as usize);
        Ok(tracks)
    }

    /// Every song on the server. An empty `search3` query lists the whole library on
    /// OpenSubsonic servers, a page at a time.
    async fn all_tracks(&self, limit: u32) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        let mut offset = 0i32;
        loop {
            let wanted = (limit - tracks.len() as u32).min(LIBRARY_PAGE as u32) as i32;
            if wanted <= 0 {
                break;
            }
            let page = self
                .client
                .search3(
                    "",
                    Some(0),
                    None,
                    Some(0),
                    None,
                    Some(wanted),
                    Some(offset),
                    None,
                )
                .await
                .context("cannot load the songs")?;
            let fetched = page.song.len();
            tracks.extend(page.song.into_iter().map(|song| self.song(song)));
            if fetched == 0 || fetched < wanted as usize {
                break;
            }
            offset += fetched as i32;
        }
        Ok(tracks)
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.change_saved(saved, &[track_id], &[], &[])
            .await
            .with_context(|| format!("cannot change the star for {track_id}"))
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let song = self
            .client
            .get_song(track_id)
            .await
            .with_context(|| format!("cannot load the song {track_id}"))?;
        Ok(self.song(song))
    }

    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>> {
        let song = self.client.get_song(track_id).await.ok();
        Ok(song
            .and_then(|song| song.play_count)
            .map(|count| count as u64))
    }

    async fn now_playing(&self, track_id: &str) -> Result<()> {
        self.client
            .scrobble(track_id, None, Some(false))
            .await
            .context("cannot report now playing")?;
        Ok(())
    }

    async fn scrobble(&self, track_id: &str) -> Result<()> {
        self.client
            .scrobble(track_id, None, Some(true))
            .await
            .context("cannot scrobble the track")?;
        Ok(())
    }

    async fn playlists(&self, limit: u32) -> Result<Vec<Playlist>> {
        let mut playlists: Vec<Playlist> = self
            .client
            .get_playlists(None)
            .await
            .context("cannot load the playlists")?
            .iter()
            .map(|playlist| self.convert_playlist(playlist))
            .collect();
        playlists.truncate(limit as usize);
        Ok(playlists)
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let created = self
            .client
            .create_playlist(None, Some(name), &[])
            .await
            .context("cannot create the playlist")?;
        Ok(created.id)
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        self.client
            .update_playlist(playlist_id, Some(name), None, None, &[], &[])
            .await
            .context("cannot rename the playlist")
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.client
            .delete_playlist(playlist_id)
            .await
            .context("cannot delete the playlist")
    }

    async fn remove_playlist_from_library(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    async fn add_playlist_to_library(&self, _playlist_id: &str) -> Result<()> {
        Ok(())
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        self.client
            .update_playlist(playlist_id, None, None, Some(public), &[], &[])
            .await
            .context("cannot change the playlist visibility")
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.client
            .update_playlist(playlist_id, None, None, None, &[track_id], &[])
            .await
            .context("cannot add the track to the playlist")
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let detail = self
            .client
            .get_playlist(playlist_id)
            .await
            .context("cannot load the playlist")?;
        let index = detail
            .entry
            .iter()
            .position(|song| song.id == track_id)
            .context("track is not in the playlist")? as i32;
        self.client
            .update_playlist(playlist_id, None, None, None, &[], &[index])
            .await
            .context("cannot remove the track from the playlist")
    }

    async fn saved_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let starred = self
            .client
            .get_starred2(None)
            .await
            .context("cannot load the starred albums")?;
        Ok(starred
            .album
            .into_iter()
            .take(limit as usize)
            .map(|album| self.convert_album(album))
            .collect())
    }

    async fn all_albums(&self, limit: u32) -> Result<Vec<Album>> {
        let mut albums = Vec::new();
        let mut offset = 0i32;
        loop {
            let wanted = (limit - albums.len() as u32).min(LIBRARY_PAGE as u32) as i32;
            if wanted <= 0 {
                break;
            }
            let page = self
                .client
                .get_album_list2(
                    AlbumListType::AlphabeticalByName,
                    Some(wanted),
                    Some(offset),
                    None,
                    None,
                    None,
                    None,
                )
                .await
                .context("cannot load the albums")?;
            let fetched = page.len();
            albums.extend(page.into_iter().map(|album| self.convert_album(album)));
            if fetched == 0 {
                break;
            }
            offset += fetched as i32;
        }
        Ok(albums)
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.change_saved(saved, &[], &[album_id], &[])
            .await
            .with_context(|| format!("cannot change the star for album {album_id}"))
    }

    async fn saved_artists(&self, limit: u32) -> Result<Vec<SavedArtist>> {
        let starred = self
            .client
            .get_starred2(None)
            .await
            .context("cannot load the starred artists")?;
        Ok(starred
            .artist
            .iter()
            .take(limit as usize)
            .map(|artist| {
                let cover = self.artist_cover(
                    artist.cover_art.as_deref(),
                    artist.artist_image_url.as_deref(),
                    &artist.id,
                );
                wire::saved_artist(artist, cover)
            })
            .collect())
    }

    async fn all_artists(&self, limit: u32) -> Result<Vec<SavedArtist>> {
        let artists = self
            .client
            .get_artists(None)
            .await
            .context("cannot load the artists")?;
        Ok(artists
            .index
            .into_iter()
            .flat_map(|index| index.artist)
            .take(limit as usize)
            .map(|artist| {
                let cover = self.artist_cover(
                    artist.cover_art.as_deref(),
                    artist.artist_image_url.as_deref(),
                    &artist.id,
                );
                wire::saved_artist(&artist, cover)
            })
            .collect())
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.change_saved(saved, &[], &[], &[artist_id])
            .await
            .with_context(|| format!("cannot change the star for artist {artist_id}"))
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let detail = self
            .client
            .get_album(album_id)
            .await
            .with_context(|| format!("cannot load the album {album_id}"))?;
        let album = self.detail_album(&detail, detail.song.len());
        let tracks = detail
            .song
            .into_iter()
            .map(|song| self.song(song))
            .collect();
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let detail = self
            .client
            .get_playlist(playlist_id)
            .await
            .with_context(|| format!("cannot load the playlist {playlist_id}"))?;
        let tracks: Vec<Track> = detail
            .entry
            .iter()
            .map(|song| self.song(song.clone()))
            .collect();
        let cover = detail
            .cover_art
            .as_deref()
            .and_then(|id| self.cover_url(id, 300));
        let mut playlist = wire::playlist(
            &detail.id,
            &detail.name,
            detail.owner.as_deref(),
            detail.public.unwrap_or(false),
            detail.song_count.unwrap_or(0).max(0) as u32,
            cover,
            &self.username,
        );
        if playlist.track_count == 0 {
            playlist.track_count = tracks.len() as u32;
        }
        if playlist.cover.is_none() {
            playlist.cover = tracks.iter().find_map(|track| track.cover.clone());
        }
        Ok(PlaylistDetail {
            playlist,
            tracks,
            continuation: None,
        })
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self.playlist(playlist_id).await?.tracks)
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = self.playlist_tracks(playlist_id).await?;
        Ok(distinct_covers(&tracks, wanted))
    }

    async fn track_radio(&self, track_id: &str) -> Result<Vec<Track>> {
        let similar = self
            .client
            .get_similar_songs2(track_id, Some(RADIO_COUNT))
            .await;
        if let Ok(songs) = similar {
            let songs: Vec<Track> = songs.into_iter().map(|song| self.song(song)).collect();
            if !songs.is_empty() {
                return Ok(songs);
            }
        }
        let random = self
            .client
            .get_random_songs(Some(20), None, None, None, None)
            .await
            .context("cannot load a radio fallback")?;
        Ok(random
            .into_iter()
            .map(|song| self.song(song))
            .filter(|track| track.id.as_deref() != Some(track_id))
            .collect())
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let found = self
            .client
            .search3(query, None, None, None, None, Some(50), None, None)
            .await
            .context("cannot search")?;
        Ok(found.song.into_iter().map(|song| self.song(song)).collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let found = self
            .client
            .search3(query, None, None, Some(30), None, None, None, None)
            .await
            .context("cannot search albums")?;
        Ok(found
            .album
            .into_iter()
            .map(|album| self.convert_album(album))
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let needle = query.to_lowercase();
        Ok(self
            .playlists(u32::MAX)
            .await?
            .into_iter()
            .filter(|playlist| playlist.name.to_lowercase().contains(&needle))
            .collect())
    }

    async fn home(&self) -> Result<HomeFeed> {
        let random = self
            .client
            .get_random_songs(Some(HOME_SONGS), None, None, None, None)
            .await
            .map(|songs| {
                songs
                    .into_iter()
                    .map(|song| self.song(song))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let newest = self.album_list(AlbumListType::Newest, HOME_ALBUMS).await;
        let frequent = self.album_list(AlbumListType::Frequent, HOME_ALBUMS).await;

        let mut sections = Vec::new();
        if !newest.is_empty() {
            sections.push(GenreSection {
                title: "Newest albums".to_owned(),
                items: newest.into_iter().map(GenreItem::Album).collect(),
            });
        }
        if !frequent.is_empty() {
            sections.push(GenreSection {
                title: "Most played albums".to_owned(),
                items: frequent.into_iter().map(GenreItem::Album).collect(),
            });
        }

        Ok(HomeFeed {
            listen_again: random.iter().take(10).cloned().collect(),
            quick_picks: Some(random.into_iter().take(15).collect()),
            sections,
        })
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        let genres = self
            .client
            .get_genres()
            .await
            .context("cannot load the genres")?;
        Ok(genres.into_iter().map(source_genre).collect())
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        let albums = self
            .client
            .get_album_list2(
                AlbumListType::ByGenre,
                Some(50),
                None,
                None,
                None,
                Some(genre_id),
                None,
            )
            .await
            .with_context(|| format!("cannot load the genre {genre_id}"))?;
        let items: Vec<GenreItem> = albums
            .into_iter()
            .map(|album| GenreItem::Album(self.convert_album(album)))
            .collect();
        Ok(GenreDetail {
            name: genre_id.to_owned(),
            sections: match items.is_empty() {
                true => Vec::new(),
                false => vec![GenreSection {
                    title: genre_id.to_owned(),
                    items,
                }],
            },
        })
    }
}

impl SubsonicClient {
    async fn change_saved(
        &self,
        saved: bool,
        ids: &[&str],
        album_ids: &[&str],
        artist_ids: &[&str],
    ) -> Result<()> {
        match saved {
            true => self.client.star(ids, album_ids, artist_ids).await?,
            false => self.client.unstar(ids, album_ids, artist_ids).await?,
        }
        Ok(())
    }

    async fn album_list(
        &self,
        kind: opensubsonic::api::lists::AlbumListType,
        size: i32,
    ) -> Vec<Album> {
        self.client
            .get_album_list2(kind, Some(size), None, None, None, None, None)
            .await
            .map(|albums| {
                albums
                    .into_iter()
                    .map(|album| self.convert_album(album))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Opens the audio of a track and answers once the response headers are in; the body is
    /// still on its way.
    pub async fn open_stream(&self, track_id: &str) -> Result<reqwest::Response> {
        let url = self
            .client
            .stream_url(track_id, None, None)
            .context("cannot build the stream url")?;
        self.http
            .get(url)
            .send()
            .await
            .context("cannot stream the track")?
            .error_for_status()
            .context("the server refused the stream")
    }

    /// The length the server records for a track, when it has one.
    pub async fn duration(&self, track_id: &str) -> Option<Duration> {
        let song = self.client.get_song(track_id).await.ok()?;
        let seconds = song.duration?;
        Some(Duration::from_secs(u64::try_from(seconds).unwrap_or(0)))
    }
}

fn source_genre(source: SourceGenre) -> Genre {
    let name = source.name;
    Genre {
        id: name.clone(),
        name,
        cover: None,
    }
}
