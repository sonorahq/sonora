use std::collections::{HashMap, HashSet};

use crate::{MediaKind, MusicApi, escape};
use anyhow::{Context as _, Result};
use async_trait::async_trait;
use librespot_core::Session;
use librespot_protocol::playlist4_external::SelectedListContent as RootList;
use protobuf::Message as _;

use crate::spotify::{
    albums, artists, collection, collection2, pathfinder, playlists, profiles, radio, search, wire,
};
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, Genre, GenreDetail,
    HomeFeed, Playlist, PlaylistDetail, PlaylistEntry, SUGGESTIONS, SavedArtist, Track, UserDetail,
    UserProfile,
};

const MADE_FOR_YOU: &str = "0JQ5DAt0tbjZptfcdMSKl3";

pub struct LibrespotClient {
    session: Session,
}

impl LibrespotClient {
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The account's own display name, or `None` when the lookup fails. Signing in calls this,
    /// so a profile Spotify will not hand over must not take the session down with it.
    async fn display_name(&self, username: &str) -> Option<String> {
        let body = self
            .session
            .spclient()
            .get_user_profile(&escape::component(username), None, None)
            .await
            .inspect_err(|error| log::debug!("profiles: cannot read {username}: {error}"))
            .ok()?;

        let profile: wire::Named = serde_json::from_slice(&body).ok()?;
        profile.label().map(str::to_owned)
    }
}

#[async_trait]
impl MusicApi for LibrespotClient {
    fn alive(&self) -> bool {
        !self.session.is_invalid()
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
        let username = self.session.username();
        let display_name = self
            .display_name(&username)
            .await
            .unwrap_or_else(|| username.clone());
        Ok(UserProfile {
            display_name,
            id: username,
        })
    }

    async fn user(&self, user_id: &str) -> Result<UserDetail> {
        profiles::profile(&self.session, user_id).await
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        artists::artist(&self.session, artist_id).await
    }

    async fn artist_catalogue(&self, artist_id: &str, known: &[Track]) -> Result<ArtistCatalogue> {
        artists::catalogue(&self.session, artist_id, known.to_vec()).await
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        artists::profile(&self.session, artist_id).await
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        artists::images(&self.session, &ids).await
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        collection::saved_tracks(&self.session).await
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        collection2::set_track_saved(&self.session, track_id, saved).await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        collection::track(&self.session, track_id).await
    }

    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>> {
        pathfinder::track(&self.session, track_id).await
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        albums::saved_albums(&self.session).await
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        collection2::set_album_saved(&self.session, album_id, saved).await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        artists::saved_artists(&self.session).await
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        collection2::set_artist_saved(&self.session, artist_id, saved).await
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        albums::album(&self.session, album_id).await
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        albums::album_tracks(&self.session, album_id).await
    }

    /// The artist's own releases without the album the page is already showing. Similar
    /// artists stay out: the internal queries are persisted by hash, so nothing here can
    /// ask for a relationship the official client never registered.
    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        let Some(artist_id) = artist_id else {
            return Ok(AlbumCatalogue::default());
        };
        let mut also_like = artists::discography(&self.session, artist_id)
            .await
            .with_context(|| format!("cannot load more from artist {artist_id}"))?;
        also_like.retain(|album| album.id != album_id);
        also_like.truncate(SUGGESTIONS);
        Ok(AlbumCatalogue {
            also_like,
            similar: Vec::new(),
        })
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let mut detail = playlists::playlist(&self.session, playlist_id).await?;
        let owner = detail.playlist.owner_id.clone();
        if !owner.is_empty() {
            let names =
                profiles::display_names(&self.session, HashSet::from([owner.clone()])).await;
            if let Some(name) = names.get(&owner) {
                detail.playlist.owner = name.clone();
            }
        }
        Ok(detail)
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        playlists::covers(&self.session, playlist_id, wanted).await
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        playlists::playlist_tracks(&self.session, playlist_id).await
    }

    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        Ok((radio::track_radio(&self.session, track_id).await?, None))
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        search::search(&self.session, query).await
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        pathfinder::search_albums(&self.session, query).await
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        pathfinder::search_playlists(&self.session, query).await
    }

    async fn home(&self) -> Result<HomeFeed> {
        let mut sections = pathfinder::genre(&self.session, MADE_FOR_YOU)
            .await?
            .sections;
        playlists::name_blanks(&self.session, &mut sections).await;

        Ok(HomeFeed {
            sections,
            ..HomeFeed::default()
        })
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        pathfinder::genres(&self.session).await
    }

    async fn genre(&self, genre_id: &str) -> Result<GenreDetail> {
        pathfinder::genre(&self.session, genre_id).await
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        playlists::create(&self.session, name).await
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        playlists::rename(&self.session, playlist_id, name).await
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        playlists::delete(&self.session, playlist_id).await
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

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        playlists::add_track(&self.session, playlist_id, track_id).await
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        playlists::remove_track(&self.session, playlist_id, track_id).await
    }

    async fn set_pinned(&self, uri: &str, pinned: bool) -> Result<crate::PinOutcome> {
        pathfinder::set_pinned(&self.session, uri, pinned).await
    }

    async fn pin_targets(&self) -> Result<Option<Vec<crate::PinTarget>>> {
        pathfinder::library(&self.session).await.map(Some)
    }

    /// Pages the rootlist and folds every page into one list before reading it, so a folder
    /// whose group markers straddle a page boundary still comes back whole. A playlist uri
    /// already taken is skipped, and its meta item with it, keeping the two lists aligned.
    async fn playlists(&self) -> Result<Vec<PlaylistEntry>> {
        let mut rootlist = RootList::new();
        let mut offset = 0;
        let mut seen = HashSet::new();
        loop {
            let body = playlists::rootlist_page(&self.session, offset, 300).await?;
            let page =
                RootList::parse_from_bytes(&body).context("cannot decode the rootlist protobuf")?;
            let count = page.contents.items.len();

            let contents = rootlist.contents.mut_or_insert_default();
            for (index, item) in page.contents.items.iter().enumerate() {
                let taken = item.uri().starts_with(wire::PLAYLIST_PREFIX)
                    && !seen.insert(item.uri().to_owned());
                if taken {
                    continue;
                }
                contents.items.push(item.clone());
                contents.meta_items.push(
                    page.contents
                        .meta_items
                        .get(index)
                        .cloned()
                        .unwrap_or_default(),
                );
            }

            offset += count;
            if count == 0 || !page.contents.truncated() {
                break;
            }
        }
        log::debug!(
            "playlists: the rootlist holds {} items and {} meta items",
            rootlist.contents.items.len(),
            rootlist.contents.meta_items.len()
        );
        let mut entries = wire::playlists_from(&rootlist);

        let (owners, ids): (HashSet<String>, Vec<String>) = {
            let playlists = PlaylistEntry::playlists(&entries);
            (
                playlists
                    .iter()
                    .map(|playlist| playlist.owner_id.clone())
                    .filter(|owner| !owner.is_empty())
                    .collect(),
                playlists
                    .iter()
                    .map(|playlist| playlist.id.clone())
                    .collect(),
            )
        };
        let (names, stamps) = tokio::join!(
            profiles::display_names(&self.session, owners),
            playlists::modified(&self.session, ids)
        );

        for playlist in PlaylistEntry::playlists_mut(&mut entries) {
            playlist.owned = playlist.owner_id == self.session.username();
            if let Some(name) = names.get(&playlist.owner_id) {
                playlist.owner = name.clone();
            }
            playlist.modified_at = stamps.get(&playlist.id).copied();
        }

        Ok(entries)
    }
}
