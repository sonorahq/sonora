use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use futures::{StreamExt as _, stream};
use opensubsonic::api::lists::AlbumListType;
use opensubsonic::data::{AlbumId3, AlbumWithSongsId3, Child, Genre as SourceGenre};
use opensubsonic::{Auth, Client};
use tokio::sync::OnceCell;
use tokio::task::JoinSet;

use crate::cover::{self, CoverLimits};
use crate::engine::Loudness;
use crate::escape;
use crate::subsonic::auth::{Library, Signature};
use crate::subsonic::wire;
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistProfile, Codec, Genre, GenreDetail,
    GenreItem, GenreSection, HomeFeed, MediaKind, MusicApi, Playlist, PlaylistDetail, Report,
    SUGGESTIONS, SavedArtist, Track, Transcode, UserProfile, distinct_covers, iso_8601_to_epoch,
};

const PORTRAIT_LIMIT: usize = 24;
const RADIO_COUNT: i32 = 25;
/// How many similar artists lend their albums to a thin rail, and how many albums each
/// lends.
const SIMILAR_ARTISTS: usize = 6;
const SIMILAR_RELEASES: usize = 2;
const HOME_SONGS: i32 = 25;
const HOME_ALBUMS: i32 = 12;
/// How many albums playing an artist reads at once.
const ALBUM_READS: usize = 4;
const LIBRARY_PAGE: i32 = 500;
const API_VERSION: &str = "1.16.1";
const CLIENT_NAME: &str = "sonora";
/// The first Navidrome that takes a playlist picture through its own API.
const NAVIDROME_COVERS: (u32, u32) = (0, 61);
/// Navidrome takes up to 10 MB by default, and shows a cover at most this large.
const PLAYLIST_COVER: CoverLimits = CoverLimits {
    side: 1500,
    bytes: 4 * 1024 * 1024,
};
/// The OpenSubsonic extension that takes a playback position and state.
const PLAYBACK_REPORT: &str = "playbackReport";
/// The edge of a list cover, in pixels.
const COVER: i32 = 300;
/// The edge of a large cover, wide enough for the fullscreen player on a 2x display. The server
/// only scales down, so a smaller original comes back as it is. Asking for the original instead
/// would send whole multi-megabyte scans to every album card.
const COVER_LARGE: i32 = 1200;

#[derive(Clone)]
pub struct SubsonicClient {
    client: Client,
    username: String,
    /// The password Navidrome's own API signs in with, which a playlist picture goes through.
    password: String,
    /// The fixed signature as a query, which every hand-built Subsonic url starts with.
    signed: String,
    /// The server address with no trailing slash.
    server: String,
    http: reqwest::Client,
    /// The cover art endpoint with the fixed signature already on it. The library client signs
    /// every url afresh, which would give one cover a new url on every conversion and defeat
    /// every image cache between here and the screen.
    covers: String,
    /// Whether the server takes `reportPlayback`, asked the first time a report goes out.
    playback_report: Arc<OnceCell<bool>>,
    /// The library every scoped request is limited to, chosen in settings. `None` reads the
    /// whole server, as every account did before libraries could be chosen. It is fixed for
    /// the client's life: a change rebuilds the session rather than mutating requests in
    /// flight.
    library: Option<i64>,
}

/// What the server records about a track that playback wants before the decoder can tell.
#[derive(Clone, Copy, Debug, Default)]
pub struct Details {
    pub duration: Option<Duration>,
    /// The track's ReplayGain, which OpenSubsonic servers such as Navidrome report and plain
    /// Subsonic servers do not.
    pub loudness: Option<Loudness>,
}

/// One page of the library-membership walk.
const SCOPE_PAGE: i32 = 500;
/// The most pages the walk reads before calling the library unverifiably large.
const SCOPE_PAGES: i32 = 100;

impl SubsonicClient {
    /// Every album ID in the chosen library, paged through the empty-query search the
    /// OpenSubsonic spec guarantees ("servers must support an empty query and return all the
    /// data"). This is the membership oracle for scoping what the API cannot scope itself:
    /// complete, exact IDs, no name matching. Only an empty page ends the walk: a short page
    /// can be a server cap rather than the end, and stopping there would truncate membership.
    /// A page that adds nothing means the server ignores offsets, and more than `SCOPE_PAGES`
    /// pages means the library is too large to verify - both error, because partial
    /// membership silently drops valid albums, the same failure as widening the scope.
    async fn scoped_album_ids(&self, folder: &str) -> Result<std::collections::HashSet<String>> {
        let mut ids = std::collections::HashSet::new();
        let mut offset = 0;
        for _ in 0..SCOPE_PAGES {
            let found = self
                .client
                .search3(
                    "",
                    Some(0),
                    None,
                    Some(SCOPE_PAGE),
                    Some(offset),
                    Some(0),
                    None,
                    Some(folder),
                )
                .await
                .context("cannot read the chosen library's contents")?;
            if found.album.is_empty() {
                return Ok(ids);
            }
            offset += found.album.len() as i32;
            let before = ids.len();
            ids.extend(found.album.into_iter().map(|album| album.id));
            if ids.len() == before {
                anyhow::bail!("the server repeats one page of the library's contents");
            }
        }
        anyhow::bail!("the chosen library is too large to verify")
    }

    pub fn new(
        server: String,
        username: String,
        password: String,
        signature: &Signature,
        library: Option<i64>,
    ) -> Result<Self> {
        let server = server.trim_end_matches('/').to_owned();
        let client = Client::new(&server, Auth::token(&username, password.clone()))
            .context("cannot parse the subsonic server address")?
            .with_client_name(CLIENT_NAME);
        let signed = format!(
            "u={}&t={}&s={}&v={API_VERSION}&c={CLIENT_NAME}&f=json",
            escape::component(&username),
            escape::component(&signature.token),
            escape::component(&signature.salt),
        );
        let covers = format!("{server}/rest/getCoverArt?{signed}");
        Ok(Self {
            client,
            username,
            password,
            signed,
            server,
            http: reqwest::Client::new(),
            covers,
            playback_report: Arc::default(),
            library,
        })
    }

    /// Whether the server is a Navidrome that takes playlist pictures. Only its own API does,
    /// and only from 0.61 on. A server that cannot be asked counts as not taking them.
    pub async fn takes_playlist_covers(&self) -> bool {
        let url = format!("{}/rest/ping?{}", self.server, self.signed);
        let answer: serde_json::Value = match self.http.get(url).send().await {
            Ok(response) => response.json().await.unwrap_or_default(),
            Err(error) => {
                log::info!("subsonic: cannot ask what the server is: {error:#}");
                return false;
            }
        };
        let response = &answer["subsonic-response"];
        response["type"].as_str() == Some("navidrome")
            && response["serverVersion"]
                .as_str()
                .and_then(release)
                .is_some_and(|version| version >= NAVIDROME_COVERS)
    }

    /// Signs in to Navidrome's own API and answers the token its requests carry.
    async fn navidrome_token(&self) -> Result<String> {
        let answer: serde_json::Value = self
            .http
            .post(format!("{}/auth/login", self.server))
            .json(&serde_json::json!({
                "username": self.username,
                "password": self.password,
            }))
            .send()
            .await
            .context("cannot reach navidrome's sign-in")?
            .error_for_status()
            .context("navidrome refused the sign-in")?
            .json()
            .await
            .context("navidrome's sign-in answered no json")?;
        answer["token"]
            .as_str()
            .map(str::to_owned)
            .context("navidrome's sign-in answered no token")
    }

    /// Whether the server lists the `playbackReport` extension, asked once per client. A plain
    /// Subsonic server has no extension list and answers with an error, which counts as no.
    async fn reports_playback(&self) -> bool {
        *self
            .playback_report
            .get_or_init(|| async {
                match self.client.get_open_subsonic_extensions().await {
                    Ok(extensions) => extensions
                        .iter()
                        .any(|extension| extension.name == PLAYBACK_REPORT),
                    Err(error) => {
                        log::info!("subsonic: the server lists no extensions: {error:#}");
                        false
                    }
                }
            })
            .await
    }

    fn cover_url(&self, id: &str, size: i32) -> Option<String> {
        Some(format!(
            "{}&id={}&size={size}",
            self.covers,
            escape::component(id)
        ))
    }

    fn cover(&self, art: Option<&str>, fallback: &str) -> Option<String> {
        self.cover_url(art.filter(|id| !id.is_empty()).unwrap_or(fallback), COVER)
    }

    fn cover_large(&self, art: Option<&str>, fallback: &str) -> Option<String> {
        self.cover_url(
            art.filter(|id| !id.is_empty()).unwrap_or(fallback),
            COVER_LARGE,
        )
    }

    /// The library scope as the query value the API takes, for the calls that take it.
    fn folder(&self) -> Option<String> {
        self.library.map(|id| id.to_string())
    }

    /// The libraries the server lists for this account, named where they have names. Anything
    /// the account cannot read is the server's to leave out.
    pub async fn music_folders(&self) -> Result<Vec<Library>> {
        let mut folders = self
            .client
            .get_music_folders()
            .await
            .context("cannot load the libraries")?
            .into_iter()
            .map(|folder| Library {
                id: folder.id,
                name: folder.name,
            })
            .collect::<Vec<_>>();
        folders.sort_by_key(|folder| folder.name.clone().unwrap_or_default().to_lowercase());
        Ok(folders)
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
            release_type: wire::release_type(
                detail.release_types.as_deref(),
                detail.is_compilation,
            ),
            year,
            track_count: detail
                .song_count
                .map(|count| count.max(0) as u32)
                .unwrap_or(tracks as u32),
            release_date: match year {
                0 => String::new(),
                _ => year.to_string(),
            },
            label: wire::labels(detail.record_labels.as_deref()),
            copyrights: Vec::new(),
            added_at: iso_8601_to_epoch(detail.created.as_deref()),
        }
    }

    fn convert_playlist(&self, source: &opensubsonic::data::Playlist) -> Playlist {
        let cover = source
            .cover_art
            .as_deref()
            .and_then(|id| self.cover_url(id, COVER));
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

    /// Up to `SUGGESTIONS` of the artist's own albums without the album the page is already
    /// showing.
    async fn more_from_artist(&self, album_id: &str, artist_id: &str) -> Result<Vec<Album>> {
        let detail = self
            .client
            .get_artist(artist_id)
            .await
            .with_context(|| format!("cannot load more from artist {artist_id}"))?;
        Ok(detail
            .album
            .into_iter()
            .map(|album| self.convert_album(album))
            .filter(|album| album.id != album_id)
            .take(SUGGESTIONS)
            .collect())
    }

    /// Up to `SUGGESTIONS` artists the server lists as similar, for the rail's artists tab.
    async fn similar_artists(&self, artist_id: &str) -> Result<Vec<SavedArtist>> {
        let info = self
            .client
            .get_artist_info2(artist_id, Some(SUGGESTIONS as i32), None)
            .await
            .with_context(|| format!("cannot load artists similar to {artist_id}"))?;
        Ok(info
            .similar_artist
            .into_iter()
            .filter(|artist| !artist.name.is_empty())
            .map(|artist| SavedArtist {
                cover: self.artist_cover(
                    artist.cover_art.as_deref(),
                    artist.artist_image_url.as_deref(),
                    &artist.id,
                ),
                id: artist.id.clone(),
                name: artist.name.clone(),
                added_at: None,
            })
            .take(SUGGESTIONS)
            .collect())
    }

    /// A few albums each from the first similar artists, skipping the page's own album:
    /// the cross-artist half of the rail. One artist failing only shortens the rail.
    async fn similar_releases(&self, album_id: &str, similar: &[SavedArtist]) -> Vec<Album> {
        let mut tasks = JoinSet::new();
        for artist in similar.iter().take(SIMILAR_ARTISTS) {
            let client = self.clone();
            let album_id = album_id.to_owned();
            let artist_id = artist.id.clone();
            tasks.spawn(async move {
                let detail = match client.client.get_artist(&artist_id).await {
                    Ok(detail) => detail,
                    Err(error) => {
                        log::warn!("subsonic: cannot load similar artist {artist_id}: {error:#}");
                        return None;
                    }
                };
                Some(
                    detail
                        .album
                        .into_iter()
                        .map(|album| client.convert_album(album))
                        .filter(|album| album.id != album_id)
                        .take(SIMILAR_RELEASES)
                        .collect::<Vec<Album>>(),
                )
            });
        }
        let mut releases = Vec::new();
        while let Some(read) = tasks.join_next().await {
            if let Ok(Some(read)) = read {
                releases.extend(read);
            }
        }
        releases
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

        // getTopSongs and the by-id getArtist take no musicFolderId (the API has none; on
        // Navidrome top songs come from Last.fm), so under a chosen library both are
        // intersected with the library's complete album set, paged through the empty-query
        // search every OpenSubsonic server must answer. Top tracks keep only songs on albums
        // the library holds, and the album list keeps only the library's own copies, which
        // collapses the "same album from two libraries" duplicates the issue reports. A name
        // search is deliberately not used: it can miss miscredited albums and fabricate an
        // empty membership. If the library's contents cannot be read the page fails visibly
        // rather than quietly showing server-wide data. Unscoped accounts are untouched.
        let scoped = match self.folder() {
            Some(folder) => Some(self.scoped_album_ids(&folder).await?),
            None => None,
        };

        let mut top_tracks: Vec<Track> = match self.client.get_top_songs(&name, Some(20)).await {
            Ok(songs) => songs.into_iter().map(|song| self.song(song)).collect(),
            Err(_) => Vec::new(),
        };
        if top_tracks.is_empty()
            && let Ok(similar) = self
                .client
                .search3(
                    &name,
                    None,
                    None,
                    None,
                    None,
                    Some(10),
                    None,
                    self.folder().as_deref(),
                )
                .await
        {
            top_tracks = similar
                .song
                .into_iter()
                .map(|song| self.song(song))
                .collect();
        }
        if let Some(held) = &scoped {
            top_tracks.retain(|track| {
                track
                    .album_id
                    .as_deref()
                    .is_some_and(|album| held.contains(album))
            });
        }

        let albums = detail
            .album
            .into_iter()
            .filter(|album| scoped.as_ref().is_none_or(|held| held.contains(&album.id)))
            .map(|album| self.convert_album(album))
            .collect();

        Ok(Artist {
            name,
            cover_large,
            banner: None,
            biography,
            monthly_listeners: None,
            top_tracks,
            albums,
        })
    }

    /// Keeps top songs first, then album tracks newest first because Subsonic has no further
    /// popularity ranking. Albums that fail to load are omitted.
    async fn artist_tracks(&self, artist_id: &str) -> Result<Vec<Track>> {
        let artist = self.artist(artist_id).await?;
        let mut albums = artist.albums;
        albums.sort_by_key(|album| std::cmp::Reverse(album.year));
        let reads: Vec<_> = stream::iter(albums)
            .map(|album| async move { (self.album_tracks(&album.id).await, album.name) })
            .buffered(ALBUM_READS)
            .collect()
            .await;
        let mut tracks = artist.top_tracks;
        for (read, album) in reads {
            match read {
                Ok(songs) => tracks.extend(songs),
                Err(error) => log::warn!("subsonic: cannot read the album {album}: {error:#}"),
            }
        }
        Ok(crate::artist_songs(tracks, artist_id))
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

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let starred = self
            .client
            .get_starred2(self.folder().as_deref())
            .await
            .context("cannot load the starred songs")?;
        Ok(starred
            .song
            .into_iter()
            .map(|song| self.song(song))
            .collect())
    }

    /// Every song on the server. An empty `search3` query lists the whole library on
    /// OpenSubsonic servers, a page at a time.
    async fn all_tracks(&self) -> Result<Vec<Track>> {
        let mut tracks = Vec::new();
        let mut offset = 0i32;
        loop {
            let page = self
                .client
                .search3(
                    "",
                    Some(0),
                    None,
                    Some(0),
                    None,
                    Some(LIBRARY_PAGE),
                    Some(offset),
                    self.folder().as_deref(),
                )
                .await
                .context("cannot load the songs")?;
            let fetched = page.song.len();
            tracks.extend(page.song.into_iter().map(|song| self.song(song)));
            if fetched < LIBRARY_PAGE as usize {
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

    /// Sends the position and state through `reportPlayback`, with scrobbling left to `played`.
    /// A server without the extension only hears that the track is playing, through the
    /// now-playing form of `scrobble`, since it has nowhere to put a position.
    async fn report(&self, track_id: &str, report: Report, position: Duration) -> Result<()> {
        if !self.reports_playback().await {
            return match report {
                Report::Playing => self
                    .client
                    .scrobble(track_id, None, Some(false))
                    .await
                    .with_context(|| format!("cannot report {track_id} as playing")),
                Report::Paused | Report::Stopped => Ok(()),
            };
        }
        let state = match report {
            Report::Playing => "playing",
            Report::Paused => "paused",
            Report::Stopped => "stopped",
        };
        let millis = i64::try_from(position.as_millis()).unwrap_or(i64::MAX);
        self.client
            .report_playback(track_id, "song", millis, state, None, Some(true))
            .await
            .with_context(|| format!("cannot report {track_id} as {state}"))
    }

    /// Subsonic takes the start of the listen in milliseconds since the epoch.
    async fn played(&self, track_id: &str, at: SystemTime) -> Result<()> {
        let millis = at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        self.client
            .scrobble(
                track_id,
                Some(i64::try_from(millis).unwrap_or(i64::MAX)),
                Some(true),
            )
            .await
            .with_context(|| format!("cannot record a play of {track_id}"))
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        Ok(self
            .client
            .get_playlists(None)
            .await
            .context("cannot load the playlists")?
            .iter()
            .map(|playlist| self.convert_playlist(playlist))
            .collect())
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

    /// Goes through Navidrome's own API, since Subsonic has no way to upload a picture.
    /// Taking the picture off brings back the mosaic Navidrome draws.
    async fn set_playlist_cover(&self, playlist_id: &str, image: Option<&[u8]>) -> Result<()> {
        let token = self.navidrome_token().await?;
        let url = format!(
            "{}/api/playlist/{}/image",
            self.server,
            escape::component(playlist_id)
        );
        let request = match image {
            Some(image) => {
                let jpeg = cover::square_jpeg(image, PLAYLIST_COVER)?;
                let boundary = format!("sonora{:016x}", fastrand::u64(..));
                let mut body = format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"cover.jpg\"\r\nContent-Type: image/jpeg\r\n\r\n"
                )
                .into_bytes();
                body.extend_from_slice(&jpeg);
                body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
                self.http
                    .post(url)
                    .header(
                        "Content-Type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(body)
            }
            None => self.http.delete(url),
        };
        let response = request
            .header("X-ND-Authorization", format!("Bearer {token}"))
            .send()
            .await
            .context("cannot reach navidrome")?;
        let status = response.status();
        if !status.is_success() {
            let reason = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "navidrome refused the cover with {status}: {}",
                reason.trim()
            );
        }
        Ok(())
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

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let starred = self
            .client
            .get_starred2(self.folder().as_deref())
            .await
            .context("cannot load the starred albums")?;
        Ok(starred
            .album
            .into_iter()
            .map(|album| self.convert_album(album))
            .collect())
    }

    async fn all_albums(&self) -> Result<Vec<Album>> {
        let mut albums = Vec::new();
        let mut offset = 0i32;
        loop {
            let page = self
                .client
                .get_album_list2(
                    AlbumListType::AlphabeticalByName,
                    Some(LIBRARY_PAGE),
                    Some(offset),
                    None,
                    None,
                    None,
                    self.folder().as_deref(),
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

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let starred = self
            .client
            .get_starred2(self.folder().as_deref())
            .await
            .context("cannot load the starred artists")?;
        Ok(starred
            .artist
            .iter()
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

    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        let artists = self
            .client
            .get_artists(self.folder().as_deref())
            .await
            .context("cannot load the artists")?;
        Ok(artists
            .index
            .into_iter()
            .flat_map(|index| index.artist)
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
        let mut tracks: Vec<Track> = detail
            .song
            .into_iter()
            .map(|song| self.song(song))
            .collect();
        // a server answers in whatever order it keeps; an album is heard in disc and track order
        tracks.sort_by(|a, b| {
            (a.disc_number, a.track_number)
                .cmp(&(b.disc_number, b.track_number))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(AlbumDetail { album, tracks })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn album_catalogue(
        &self,
        album_id: &str,
        artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        let Some(artist_id) = artist_id else {
            return Ok(AlbumCatalogue::default());
        };
        let (more_by, similar) = tokio::join!(
            self.more_from_artist(album_id, artist_id),
            self.similar_artists(artist_id),
        );
        // Nothing read at all is an error rather than an empty rail, so the catalog does not
        // keep the empty answer for the rest of the session.
        let (more_by, similar) = match (more_by, similar) {
            (Err(error), Err(_)) => return Err(error.context("cannot read any recommendations")),
            pair => pair,
        };
        if let Err(error) = &more_by {
            log::warn!("subsonic: cannot read more from this artist: {error:#}");
        }
        if let Err(error) = &similar {
            log::warn!("subsonic: cannot read similar artists: {error:#}");
        }
        let (more_by, similar) = (more_by.unwrap_or_default(), similar.unwrap_or_default());
        let mut seen = HashSet::new();
        let mut liked: Vec<Album> = more_by
            .into_iter()
            .filter(|album| seen.insert(album.id.clone()))
            .collect();
        if liked.len() < SUGGESTIONS && !similar.is_empty() {
            for album in self.similar_releases(album_id, &similar).await {
                if liked.len() >= SUGGESTIONS {
                    break;
                }
                if seen.insert(album.id.clone()) {
                    liked.push(album);
                }
            }
        }
        Ok(AlbumCatalogue {
            also_like: liked,
            similar,
        })
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
            .and_then(|id| self.cover_url(id, COVER));
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

    async fn track_radio(
        &self,
        track_id: &str,
        _from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        let similar = self
            .client
            .get_similar_songs2(track_id, Some(RADIO_COUNT))
            .await;
        if let Ok(songs) = similar {
            let songs: Vec<Track> = songs.into_iter().map(|song| self.song(song)).collect();
            if !songs.is_empty() {
                return Ok((songs, None));
            }
        }
        let random = self
            .client
            .get_random_songs(Some(20), None, None, None, self.folder().as_deref())
            .await
            .context("cannot load a radio fallback")?;
        Ok((
            random
                .into_iter()
                .map(|song| self.song(song))
                .filter(|track| track.id.as_deref() != Some(track_id))
                .collect(),
            None,
        ))
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let found = self
            .client
            .search3(
                query,
                None,
                None,
                None,
                None,
                Some(50),
                None,
                self.folder().as_deref(),
            )
            .await
            .context("cannot search")?;
        Ok(found.song.into_iter().map(|song| self.song(song)).collect())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let found = self
            .client
            .search3(
                query,
                None,
                None,
                Some(30),
                None,
                None,
                None,
                self.folder().as_deref(),
            )
            .await
            .context("cannot search albums")?;
        Ok(found
            .album
            .into_iter()
            .map(|album| self.convert_album(album))
            .collect())
    }

    async fn search_artists(&self, query: &str) -> Result<Vec<SavedArtist>> {
        let found = self
            .client
            .search3(query, Some(30), None, Some(0), None, Some(0), None, None)
            .await
            .context("cannot search artists")?;
        Ok(found
            .artist
            .iter()
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

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let needle = query.to_lowercase();
        Ok(self
            .playlists()
            .await?
            .into_iter()
            .filter(|playlist| playlist.name.to_lowercase().contains(&needle))
            .collect())
    }

    async fn home(&self) -> Result<HomeFeed> {
        let random = self
            .client
            .get_random_songs(Some(HOME_SONGS), None, None, None, self.folder().as_deref())
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
            listen_again: random
                .iter()
                .take(10)
                .cloned()
                .map(GenreItem::Track)
                .collect(),
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
                self.folder().as_deref(),
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

    /// The cover of one album filed under the genre, since Subsonic genres have no picture.
    async fn genre_cover(&self, genre_id: &str) -> Result<Option<String>> {
        let albums = self
            .client
            .get_album_list2(
                AlbumListType::ByGenre,
                Some(1),
                None,
                None,
                None,
                Some(genre_id),
                self.folder().as_deref(),
            )
            .await
            .with_context(|| format!("cannot load the genre {genre_id}"))?;
        Ok(albums
            .into_iter()
            .find_map(|album| self.convert_album(album).cover))
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
            .get_album_list2(
                kind,
                Some(size),
                None,
                None,
                None,
                None,
                self.folder().as_deref(),
            )
            .await
            .map(|albums| {
                albums
                    .into_iter()
                    .map(|album| self.convert_album(album))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The account and server this client plays from. A track id only means something on its
    /// own server, so this tells apart two servers' tracks that share one.
    pub fn identity(&self) -> String {
        format!("{}@{}", self.username, self.server)
    }

    /// Opens the audio of a track, converted as `transcode` asks, and answers once the response
    /// headers are in. The body is still on its way.
    pub async fn open_stream(
        &self,
        track_id: &str,
        transcode: Transcode,
    ) -> Result<reqwest::Response> {
        let bitrate = transcode.bitrate.and_then(|kbps| i32::try_from(kbps).ok());
        let format = transcode.codec.map(Codec::id);
        let url = self
            .client
            .stream_url(track_id, bitrate, format)
            .context("cannot build the stream url")?;
        self.http
            .get(url)
            .send()
            .await
            .context("cannot stream the track")?
            .error_for_status()
            .context("the server refused the stream")
    }

    /// The lyrics the server holds for a track, by the OpenSubsonic `songLyrics` extension. A
    /// plain Subsonic server fails the call.
    pub(crate) async fn lyrics_by_song(
        &self,
        track_id: &str,
    ) -> Result<opensubsonic::data::LyricsList> {
        self.client
            .get_lyrics_by_song_id(track_id, None)
            .await
            .context("cannot fetch the structured lyrics")
    }

    /// The plain lyrics a Subsonic server finds for an artist and title, the only form a server
    /// without the `songLyrics` extension offers.
    pub(crate) async fn lyrics_by_name(
        &self,
        artist: &str,
        title: &str,
    ) -> Result<opensubsonic::data::Lyrics> {
        self.client
            .get_lyrics(Some(artist), Some(title))
            .await
            .context("cannot fetch the lyrics")
    }

    /// The length and ReplayGain the server records for a track, whichever it has. The track
    /// gain wins, and the album gain stands in when that is all the server knows.
    pub async fn details(&self, track_id: &str) -> Details {
        let Ok(song) = self.client.get_song(track_id).await else {
            return Details::default();
        };
        let duration = song
            .duration
            .map(|seconds| Duration::from_secs(u64::try_from(seconds).unwrap_or(0)));
        let loudness = song.replay_gain.and_then(|gain| {
            let (gain, peak) = match gain.track_gain {
                Some(track) => (track, gain.track_peak),
                None => (gain.album_gain?, gain.album_peak),
            };
            Some(Loudness::replay_gain(
                gain as f32,
                peak.map(|peak| peak as f32),
            ))
        });
        Details { duration, loudness }
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

/// The major and minor numbers of a version such as `0.61.0` or `0.61.0 (a1b2c3)`.
fn release(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split(|letter: char| !letter.is_ascii_digit());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// A stand-in Subsonic server that records the path and query of every request and answers
    /// each listing endpoint with an empty page. It exists to prove which calls carry the
    /// library scope, not to check conversion.
    struct Stub {
        server: String,
        requests: Arc<Mutex<Vec<String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Stub {
        async fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let recording = requests.clone();
            let task = tokio::spawn(async move {
                loop {
                    let Ok((socket, _)) = listener.accept().await else {
                        return;
                    };
                    let recording = recording.clone();
                    tokio::spawn(answer(socket, recording));
                }
            });
            Self {
                server: format!("http://127.0.0.1:{port}"),
                requests,
                task,
            }
        }

        fn client(&self, library: Option<i64>) -> SubsonicClient {
            SubsonicClient::new(
                self.server.clone(),
                "ada".to_owned(),
                "secret".to_owned(),
                &Signature::default(),
                library,
            )
            .unwrap()
        }

        /// The endpoints asked so far, each with its query.
        fn asked(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn answer(mut socket: tokio::net::TcpStream, recording: Arc<Mutex<Vec<String>>>) {
        let mut received = Vec::new();
        let mut chunk = [0u8; 4096];
        let request = loop {
            match socket.read(&mut chunk).await {
                Ok(0) => return,
                Ok(read) => {
                    received.extend_from_slice(&chunk[..read]);
                    if received.windows(4).any(|end| end == b"\r\n\r\n") {
                        break String::from_utf8_lossy(&received).into_owned();
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => continue,
                Err(_) => return,
            }
        };
        let Some(line) = request.lines().next() else {
            return;
        };
        let target = line.split_whitespace().nth(1).unwrap_or("/");
        recording.lock().unwrap().push(target.to_owned());
        let endpoint = target
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        let body = canned(endpoint, target);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.ok();
    }

    /// The least JSON that keeps one endpoint's caller reading: listing endpoints answer
    /// empty pages, and the artist calls answer a two-library fixture — one song and album
    /// inside library 7, one outside — so the scoped artist page can be checked end to end.
    /// The fixture answers. Listing endpoints answer empty pages. The artist calls answer a
    /// two-library story: getArtist (unscoped by-id, like the real API) lists the same album
    /// twice - the library-7 copy `al-in` and the other library's copy `al-out`; getTopSongs
    /// (unscopeable, like the real API) offers one song from each. The scoped empty-query
    /// membership walk answers 500 filler albums on page one and `al-in` only on page two,
    /// so a walk that stops early or matches by name would lose it. Asking as library 13 is
    /// a server that cannot read its library: the scoped search fails with a real Subsonic
    /// error body.
    fn canned(endpoint: &str, target: &str) -> String {
        if endpoint == "search3" && target.contains("musicFolderId=13") {
            return String::from(
                r#"{"subsonic-response": {"status": "failed", "version": "1.16.1", "error": {"code": 0, "message": "cannot read the library"}}}"#,
            );
        }
        if endpoint == "search3" && target.contains("musicFolderId=8") {
            let offset = target
                .split('?')
                .nth(1)
                .unwrap_or_default()
                .split('&')
                .filter_map(|part| part.split_once('='))
                .find(|(key, _)| *key == "albumOffset")
                .and_then(|(_, value)| value.parse::<usize>().ok())
                .unwrap_or(0);
            let albums = match offset {
                0 => vec![serde_json::json!({"id":"short-1","name":"One"})],
                1 => vec![serde_json::json!({"id":"al-in","name":"Morning View"})],
                _ => Vec::new(),
            };
            return serde_json::json!({"subsonic-response": {"status":"ok","version":"1.16.1",
                "searchResult3":{"song":[],"artist":[],"album":albums}}})
            .to_string();
        }
        let inner = match endpoint {
            "getArtist" => {
                r#""artist": {"id": "ar-1", "name": "Incubus", "album": [{"id": "al-in", "name": "Morning View"}, {"id": "al-out", "name": "Morning View"}]}"#.to_owned()
            }
            "getTopSongs" => {
                r#""topSongs": {"song": [{"id": "s-in", "isDir": false, "title": "Wish You Were Here", "albumId": "al-in"}, {"id": "s-out", "isDir": false, "title": "Wish You Were Here", "albumId": "al-out"}]}"#.to_owned()
            }
            "search3" if target.contains("query=&") => {
                let albums: Vec<String> = if target.contains("albumOffset=500") {
                    vec![String::from(r#"{"id": "al-in", "name": "Morning View"}"#)]
                } else if target.contains("albumOffset=501") {
                    Vec::new()
                } else {
                    (0..500)
                        .map(|n| format!(r#"{{"id": "al-fill-{n}", "name": "Filler {n}"}}"#))
                        .collect()
                };
                format!(
                    r#""searchResult3": {{"song": [], "album": [{}], "artist": []}}"#,
                    albums.join(", ")
                )
            }
            "getStarred2" => r#""starred2": {"song": [], "album": [], "artist": []}"#.to_owned(),
            "getAlbumList2" => r#""albumList2": {"album": []}"#.to_owned(),
            "getArtists" => r#""artists": {"index": []}"#.to_owned(),
            "search3" => r#""searchResult3": {"song": [], "album": [], "artist": []}"#.to_owned(),
            "getRandomSongs" => r#""randomSongs": {"song": []}"#.to_owned(),
            "getPlaylists" => r#""playlists": {"playlist": []}"#.to_owned(),
            "getGenres" => r#""genres": {"genre": []}"#.to_owned(),
            "getMusicFolders" => {
                r#""musicFolders": {"musicFolder": [{"id": 9, "name": "Everything"}, {"id": 7, "name": "Classical"}]}"#.to_owned()
            }
            _ => String::new(),
        };
        let mut whole =
            String::from(r#"{"subsonic-response": {"status": "ok", "version": "1.16.1""#);
        if !inner.is_empty() {
            whole.push_str(", ");
            whole.push_str(&inner);
        }
        whole.push_str("}}");
        whole
    }

    /// The endpoint names a scoped client asked about, in order.
    fn endpoints(asked: &[String]) -> Vec<String> {
        asked
            .iter()
            .map(|target| {
                target
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_owned()
            })
            .collect()
    }

    /// Whether one request's query limits it to the library.
    fn scoped_to(request: &str, folder: &str) -> bool {
        request
            .split('?')
            .nth(1)
            .unwrap_or("")
            .split('&')
            .any(|pair| pair == format!("musicFolderId={folder}"))
    }

    fn mentions_library(request: &str) -> bool {
        request.contains("musicFolderId=")
    }

    /// Every browsing, searching, starring, genre and home call the scope belongs on sends it.
    #[tokio::test]
    async fn a_scoped_client_limits_every_listing_call() {
        let stub = Stub::start().await;
        let client = stub.client(Some(7));

        client.saved_tracks().await.unwrap();
        client.saved_albums().await.unwrap();
        client.saved_artists().await.unwrap();
        client.all_tracks().await.unwrap();
        client.all_albums().await.unwrap();
        client.all_artists().await.unwrap();
        client.search("megalomaniac").await.unwrap();
        client.search_albums("megalomaniac").await.unwrap();
        client.home().await.unwrap();
        client.genre("Rock").await.unwrap();
        client.genre_cover("Rock").await.unwrap();
        client.track_radio("song-1", None).await.unwrap();

        let asked = stub.asked();
        let endpoints = endpoints(&asked);
        // home asks for random songs and two album lists; the radio fallback asks for more
        // random songs after the similar-songs call fails its parse.
        let expected = [
            ("getStarred2", 3),
            ("search3", 3),
            ("getAlbumList2", 5),
            ("getArtists", 1),
            ("getRandomSongs", 2),
            ("getSimilarSongs2", 1),
        ];
        for (endpoint, count) in expected {
            assert_eq!(
                endpoints.iter().filter(|seen| *seen == endpoint).count(),
                count,
                "{endpoint} should be asked {count} times: {asked:?}"
            );
        }
        // getSimilarSongs2 has no musicFolderId parameter in the API; everything else here
        // does. getTopSongs has none either, which is why the client does not try to scope it.
        for request in &asked {
            let endpoint = request
                .split('?')
                .next()
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("");
            if endpoint == "getSimilarSongs2" {
                continue;
            }
            assert!(
                scoped_to(request, "7"),
                "{endpoint} should carry the library scope: {request}"
            );
        }
    }

    /// A whole-server account sends no scope at all, so old servers never see a parameter
    /// they do not know.
    #[tokio::test]
    async fn an_unscoped_client_sends_no_library_parameter() {
        let stub = Stub::start().await;
        let client = stub.client(None);

        client.saved_tracks().await.unwrap();
        client.all_tracks().await.unwrap();
        client.all_albums().await.unwrap();
        client.all_artists().await.unwrap();
        client.search("megalomaniac").await.unwrap();
        client.search_albums("megalomaniac").await.unwrap();
        client.home().await.unwrap();
        client.genre("Rock").await.unwrap();
        client.genre_cover("Rock").await.unwrap();

        for request in stub.asked() {
            assert!(
                !mentions_library(&request),
                "no call should carry a library scope: {request}"
            );
        }
    }

    /// The calls the API cannot scope stay unscoped even under a chosen library: playlists and
    /// by-id reads have no musicFolderId parameter to take.
    #[tokio::test]
    async fn playlist_calls_stay_unscoped() {
        let stub = Stub::start().await;
        let client = stub.client(Some(7));

        client.playlists().await.unwrap();

        for request in stub.asked() {
            assert!(
                !mentions_library(&request),
                "playlists have no scope to carry: {request}"
            );
        }
    }

    /// The API cannot scope getArtist or getTopSongs, so under a chosen library the artist
    /// page intersects both with the library's complete album set: the other library's copy
    /// of the album and its top track drop out, the library's own survive - including the
    /// album that only appears on the second page of the membership walk.
    #[tokio::test]
    async fn a_scoped_artist_page_keeps_only_the_librarys_copies() {
        let stub = Stub::start().await;
        let client = stub.client(Some(7));

        let artist = client.artist("ar-1").await.unwrap();

        assert_eq!(artist.top_tracks.len(), 1);
        assert_eq!(artist.top_tracks[0].id.as_deref(), Some("s-in"));
        assert_eq!(artist.albums.len(), 1);
        assert_eq!(artist.albums[0].id, "al-in");
        let asked = stub.asked();
        assert!(
            asked.iter().any(|request| request.contains("search3")
                && request.contains("albumOffset=500")
                && scoped_to(request, "7")),
            "the membership walk should page past 500 albums: {asked:?}"
        );
        assert!(
            asked
                .iter()
                .any(|request| request.contains("albumOffset=501")),
            "the membership walk should stop only at an empty page: {asked:?}"
        );
    }

    /// When the library's contents cannot be read, a scoped artist page fails visibly
    /// instead of quietly standing in with server-wide data.
    #[tokio::test]
    async fn a_scoped_artist_page_fails_visibly_when_membership_cannot_be_read() {
        let stub = Stub::start().await;
        let client = stub.client(Some(13));

        let result = client.artist("ar-1").await;

        assert!(
            result.is_err(),
            "a scoped artist page must not fall back to server-wide data"
        );
    }

    #[tokio::test]
    async fn scoped_membership_advances_by_rows_returned_when_the_server_caps_pages() {
        let stub = Stub::start().await;
        let client = stub.client(Some(8));
        let ids = client.scoped_album_ids("8").await.unwrap();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("al-in"));
        let asked = stub.asked();
        assert!(
            asked
                .iter()
                .any(|request| request.contains("albumOffset=1&"))
        );
        assert!(
            asked
                .iter()
                .any(|request| request.contains("albumOffset=2&"))
        );
    }

    /// Discovery lists the server's libraries named and sorted for the picker.
    #[tokio::test]
    async fn discovery_lists_the_libraries() {
        let stub = Stub::start().await;
        let client = stub.client(Some(7));

        let libraries = client.music_folders().await.unwrap();
        assert_eq!(
            libraries,
            vec![
                Library {
                    id: 7,
                    name: Some("Classical".to_owned()),
                },
                Library {
                    id: 9,
                    name: Some("Everything".to_owned()),
                },
            ]
        );
        // Discovery itself has no scope parameter.
        assert!(!mentions_library(&stub.asked()[0]));
    }
}
