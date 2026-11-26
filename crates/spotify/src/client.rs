use std::collections::HashMap;
use std::sync::Arc;

use crate::session::Session;
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use music::connect::Connect;
use music::{MediaKind, MusicApi, escape};

use crate::{albums, artists, collection, connect::sequence, playlists, profiles, search, wire};
use music::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, CoverSize, Genre,
    GenreDetail, HomeFeed, Playlist, PlaylistDetail, RadioSeed, SavedArtist, Track, UserDetail,
    UserProfile,
};

pub struct SpotifyClient {
    session: Session,
    connect: Arc<Connect>,
}

impl SpotifyClient {
    pub fn new(session: Session) -> Self {
        let connect = Arc::new(crate::connect::open(session.clone()));
        Self { session, connect }
    }

    pub(super) fn connect_handle(&self) -> Arc<Connect> {
        self.connect.clone()
    }
}

#[async_trait]
impl MusicApi for SpotifyClient {
    fn connect(&self) -> Option<Arc<Connect>> {
        Some(self.connect_handle())
    }

    fn cover_at(&self, cover: &str, size: CoverSize) -> Option<String> {
        wire::cover_at(cover, size)
    }

    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let kind = match kind {
            MediaKind::Track => "track",
            MediaKind::Album => "album",
            MediaKind::Artist => "artist",
            MediaKind::Playlist => "playlist",
        };
        Some(format!(
            "https://open.spotify.com/{kind}/{}",
            escape::component(id)
        ))
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(profiles::own(&self.session).await)
    }

    async fn user(&self, user_id: &str) -> Result<UserDetail> {
        profiles::profile(&self.session, user_id).await
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        artists::artist(&self.session, artist_id).await
    }

    async fn artist_catalogue(&self, artist_id: &str, known: &[Track]) -> Result<ArtistCatalogue> {
        artists::catalogue(&self.session, artist_id, known.to_vec(), None).await
    }

    async fn artist_catalogue_with_preview(
        &self,
        id: &str,
        known: &[Track],
        preview: tokio::sync::mpsc::Sender<ArtistCatalogue>,
    ) -> Result<ArtistCatalogue> {
        artists::catalogue(&self.session, id, known.to_vec(), Some(preview)).await
    }

    async fn artist_tracks(&self, artist_id: &str) -> Result<Vec<Track>> {
        artists::tracks(&self.session, artist_id).await
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        artists::profile(&self.session, artist_id).await
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        artists::images(&self.session, &ids).await
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let items = collection::saved_items(
            &self.session,
            collection::COLLECTION,
            collection::TRACK_PREFIX,
        )
        .await?;
        collection::hydrate_saved_tracks(&self.session, &items).await
    }

    async fn saved_collection_paged(&self) -> Option<music::SavedCollectionPages> {
        Some(collection::saved_collection(self.session.clone()))
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        collection::set_saved(
            &self.session,
            collection::COLLECTION,
            &format!("spotify:track:{track_id}"),
            saved,
        )
        .await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let uri = format!("{}{track_id}", collection::TRACK_PREFIX);
        collection::metadata(&self.session, std::slice::from_ref(&uri))
            .await?
            .remove(&uri)
            .context("track metadata is missing")
    }

    async fn tracks_by_id(&self, ids: &[String]) -> Result<HashMap<String, Track>> {
        collection::available_tracks(&self.session, ids).await
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let items =
            collection::saved_items(&self.session, collection::COLLECTION, albums::ALBUM_PREFIX)
                .await?;
        albums::hydrate_saved_albums(&self.session, items).await
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        collection::set_saved(
            &self.session,
            collection::COLLECTION,
            &format!("spotify:album:{album_id}"),
            saved,
        )
        .await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        artists::saved_artists(&self.session).await
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        collection::set_saved(
            &self.session,
            collection::ARTISTS,
            &format!("spotify:artist:{artist_id}"),
            saved,
        )
        .await
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        albums::album(&self.session, album_id).await
    }

    async fn song_album(&self, album_id: &str) -> Result<Option<Album>> {
        albums::read_album(&self.session, album_id, albums::album_from)
            .await
            .map(Some)
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        let uris = albums::read_album(&self.session, album_id, |_, album| {
            albums::track_uris(album).collect::<Vec<_>>()
        })
        .await?;
        collection::ordered_tracks(&self.session, &uris, collection::TRACK_DETAILS).await
    }

    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        artists::album_catalogue(&self.session, album_id, artist_id).await
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        playlists::playlist(&self.session, playlist_id, None).await
    }

    async fn playlist_with_preview(
        &self,
        playlist_id: &str,
        preview: tokio::sync::mpsc::Sender<music::PlaylistPreview>,
    ) -> Result<PlaylistDetail> {
        playlists::playlist(&self.session, playlist_id, Some(preview)).await
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        self.playlist_cover_batch(&[playlist_id.to_owned()], wanted)
            .await
            .pop()
            .expect("one result for one playlist")
    }

    async fn playlist_cover_batch(
        &self,
        ids: &[String],
        wanted: usize,
    ) -> Vec<Result<Vec<String>>> {
        playlists::covers(&self.session, ids, wanted).await
    }

    async fn playlist_memberships(
        &self,
        requests: &[(String, usize)],
    ) -> Vec<Result<music::PlaylistMembership>> {
        playlists::memberships(&self.session, requests).await
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        playlists::playlist_tracks(&self.session, playlist_id).await
    }

    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        Ok((
            sequence::radio(&self.session, "track", track_id).await?,
            None,
        ))
    }

    async fn collection_radio(
        &self,
        seed: RadioSeed,
        id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        Ok((
            sequence::radio(&self.session, sequence::radio_kind(seed), id).await?,
            None,
        ))
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        search::tracks(&self.session, query).await
    }

    async fn search_with_preview(
        &self,
        query: &str,
        preview: tokio::sync::mpsc::Sender<music::SearchPart>,
    ) -> music::Found {
        search::with_preview(&self.session, query, &preview).await
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        search::albums(&self.session, query).await
    }

    async fn search_artists(&self, query: &str) -> Result<Vec<SavedArtist>> {
        search::artists(&self.session, query).await
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        search::playlists(&self.session, query).await
    }

    async fn home(&self) -> Result<HomeFeed> {
        super::feeds::home(&self.session).await
    }

    async fn home_paged(&self) -> Result<music::Feed> {
        Ok(super::feeds::home_paged(self.session.clone()))
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        super::feeds::landing(&self.session).await
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        super::feeds::page(&self.session, genre_id, None, None).await
    }

    async fn genre_with_preview(
        &self,
        id: &str,
        title: Option<&str>,
        preview: tokio::sync::mpsc::Sender<GenreDetail>,
    ) -> Result<GenreDetail> {
        super::feeds::page(&self.session, id, title, Some(preview)).await
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        playlists::create(&self.session, name).await
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        playlists::rename(&self.session, playlist_id, name).await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        self.remove_playlist_from_library(playlist_id).await
    }

    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        playlists::remove_from_library(&self.session, playlist_id).await
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        playlists::add_to_library(&self.session, playlist_id).await
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        playlists::set_public(&self.session, playlist_id, public).await
    }

    async fn set_playlist_cover(&self, playlist_id: &str, image: Option<&[u8]>) -> Result<()> {
        self.edit_playlist_appearance(playlist_id, None, image)
            .await
            .map(drop)
    }

    async fn edit_playlist_appearance(
        &self,
        playlist_id: &str,
        name: Option<&str>,
        image: Option<&[u8]>,
    ) -> Result<Option<String>> {
        playlists::edit_appearance(&self.session, playlist_id, name, image).await
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.add_tracks_to_playlist(playlist_id, &[track_id.to_owned()])
            .await
    }

    async fn add_tracks_to_playlist(&self, playlist_id: &str, track_ids: &[String]) -> Result<()> {
        playlists::add_tracks(&self.session, playlist_id, track_ids).await
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        playlists::remove_track(&self.session, playlist_id, track_id).await
    }

    async fn reorder_playlist(
        &self,
        playlist_id: &str,
        seen: &[String],
        wanted: &[String],
    ) -> Result<()> {
        playlists::reorder(&self.session, playlist_id, seen, wanted).await
    }

    /// Spotify ignores pin writes through its collection sets, so a pin changed here stays a
    /// local one while Spotify's own pins are still read and followed.
    async fn set_pinned(&self, _uri: &str, _pinned: bool) -> Result<music::PinOutcome> {
        Ok(music::PinOutcome::Accepted)
    }

    async fn pin_targets(&self) -> Result<Option<Vec<music::PinTarget>>> {
        collection::pins(&self.session).await.map(Some)
    }

    fn pin_uri(&self, kind: music::PinTargetKind, id: &str) -> Option<String> {
        collection::pin_uri(kind, id)
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        playlists::library(&self.session).await
    }

    async fn playlist_snapshots(&self, ids: Vec<String>) -> Option<Vec<Playlist>> {
        Some(playlists::summaries(&self.session, ids).await)
    }
}
