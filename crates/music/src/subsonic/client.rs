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
use crate::smart::SmartPlaylistUpdate;
use crate::subsonic::auth::Signature;
use crate::subsonic::wire;
use crate::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistProfile, Codec, Genre, GenreDetail,
    GenreItem, GenreSection, HomeFeed, MediaKind, MusicApi, Playlist, PlaylistDetail, Report,
    SUGGESTIONS, SavedArtist, SmartPlaylist, SmartRules, Track, Transcode, UserProfile,
    distinct_covers, iso_8601_to_epoch,
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
/// The first Navidrome declared to take smart playlists through its own API. The create,
/// edit and read contract is the same across the 0.64 releases (their playlist REST adapters
/// differ only in delete error mapping, which these calls do not use), and the live exercise
/// ran against 0.64.2. An earlier Navidrome may answer them, but only a version whose
/// contract was verified is declared.
const NAVIDROME_SMART: (u32, u32) = (0, 64);
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
    /// The server's Navidrome release, asked the first time a Navidrome-only call needs it.
    navidrome: Arc<OnceCell<Option<(u32, u32)>>>,
    /// The account's token for Navidrome's own API, signed in once and reused: the sign-in
    /// endpoint is rate-limited, so a run of playlist calls shares one token and signs in
    /// again only after the server refuses it.
    token: Arc<tokio::sync::Mutex<Option<String>>>,
}

/// What the server records about a track that playback wants before the decoder can tell.
#[derive(Clone, Copy, Debug, Default)]
pub struct Details {
    pub duration: Option<Duration>,
    /// The track's ReplayGain, which OpenSubsonic servers such as Navidrome report and plain
    /// Subsonic servers do not.
    pub loudness: Option<Loudness>,
}

impl SubsonicClient {
    pub fn new(
        server: String,
        username: String,
        password: String,
        signature: &Signature,
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
            navidrome: Arc::default(),
            token: Arc::default(),
        })
    }

    /// The server's Navidrome release, asked once per client: `Some(version)` when the ping
    /// names a Navidrome, `None` for any other server or one that cannot be asked.
    async fn navidrome_release(&self) -> Option<(u32, u32)> {
        *self
            .navidrome
            .get_or_init(|| async {
                let url = format!("{}/rest/ping?{}", self.server, self.signed);
                let answer: serde_json::Value = match self.http.get(url).send().await {
                    Ok(response) => response.json().await.unwrap_or_default(),
                    Err(error) => {
                        log::info!("subsonic: cannot ask what the server is: {error:#}");
                        return None;
                    }
                };
                let response = &answer["subsonic-response"];
                match response["type"].as_str() == Some("navidrome") {
                    true => response["serverVersion"].as_str().and_then(release),
                    false => None,
                }
            })
            .await
    }

    /// Whether the server is a Navidrome that takes playlist pictures. Only its own API does,
    /// and only from 0.61 on. A server that cannot be asked counts as not taking them.
    pub async fn takes_playlist_covers(&self) -> bool {
        self.navidrome_release()
            .await
            .is_some_and(|version| version >= NAVIDROME_COVERS)
    }

    /// Whether the server is a Navidrome that creates and edits smart playlists through its
    /// own API. Only its own API does: the Subsonic playlist calls carry no rules.
    pub async fn takes_smart_playlists(&self) -> bool {
        self.navidrome_release()
            .await
            .is_some_and(|version| version >= NAVIDROME_SMART)
    }

    /// Signs in to Navidrome's own API and answers the token its requests carry.
    async fn navidrome_login(&self) -> Result<String> {
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

    /// The account's token for Navidrome's own API, from the shared sign-in.
    async fn navidrome_token(&self) -> Result<String> {
        let mut cached = self.token.lock().await;
        if let Some(token) = &*cached {
            return Ok(token.clone());
        }
        let token = self.navidrome_login().await?;
        *cached = Some(token.clone());
        Ok(token)
    }

    /// Sends a request to Navidrome's own API with an optional json body. A reused token can
    /// lapse while the client is open, so a 401 signs in once more and retries the call.
    async fn navidrome_send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<reqwest::Response> {
        let build = |token: &str| {
            let request = self
                .http
                .request(method.clone(), format!("{}{path}", self.server))
                .header("X-ND-Authorization", format!("Bearer {token}"));
            match &body {
                Some(body) => request.json(body),
                None => request,
            }
        };
        let token = self.navidrome_token().await?;
        let response = build(&token)
            .send()
            .await
            .context("cannot reach navidrome")?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        let fresh = {
            let mut cached = self.token.lock().await;
            if let Some(current) = cached.as_ref().filter(|current| *current != &token) {
                current.clone()
            } else {
                let fresh = self.navidrome_login().await?;
                *cached = Some(fresh.clone());
                fresh
            }
        };
        build(&fresh).send().await.context("cannot reach navidrome")
    }

    /// The refusal a Navidrome playlist call answers with, spelled for the listener: a
    /// forbidden edit is another owner's playlist, a conflict a playlist the server keeps
    /// read-only, and an unprocessable body rules the server could not read.
    async fn playlist_refusal(response: reqwest::Response) -> anyhow::Error {
        let status = response.status();
        let reason = response.text().await.unwrap_or_default();
        let reason = reason.trim();
        let detail = match reason.is_empty() {
            true => String::new(),
            false => format!(": {reason}"),
        };
        match status.as_u16() {
            401 => anyhow::anyhow!("navidrome refused the account's sign-in{detail}"),
            403 => anyhow::anyhow!("navidrome refuses the change for this account{detail}"),
            404 => anyhow::anyhow!("navidrome no longer has the playlist{detail}"),
            409 => anyhow::anyhow!("navidrome keeps the playlist read-only{detail}"),
            422 => anyhow::anyhow!("navidrome could not read the rules{detail}"),
            _ => anyhow::anyhow!("navidrome refused the playlist with {status}{detail}"),
        }
    }

    /// The playlist as Navidrome's own API answers it: its rules, its owner and whether a
    /// file sync can overwrite an edit. A `rules` of null marks an ordinary playlist.
    fn read_smart_playlist(&self, value: &serde_json::Value) -> Result<SmartPlaylist> {
        let rules =
            match value["rules"].is_null() {
                true => None,
                false => Some(SmartRules::from_wire(&value["rules"]).map_err(|problem| {
                    anyhow::anyhow!("navidrome's rules do not read: {problem}")
                })?),
            };
        let owner = value["ownerName"].as_str().unwrap_or_default().to_owned();
        Ok(SmartPlaylist {
            id: value["id"]
                .as_str()
                .context("navidrome's playlist answer names no id")?
                .to_owned(),
            name: value["name"].as_str().unwrap_or_default().to_owned(),
            comment: value["comment"].as_str().unwrap_or_default().to_owned(),
            public: value["public"].as_bool().unwrap_or(false),
            // Navidrome signs an account in under any spelling of its name, so ownership
            // compares without case.
            owned: !owner.is_empty() && owner.eq_ignore_ascii_case(&self.username),
            rules,
            synced: value["sync"].as_bool().unwrap_or(false)
                && value["path"].as_str().is_some_and(|path| !path.is_empty()),
            track_count: value["songCount"].as_u64().unwrap_or(0) as u32,
            owner,
        })
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
            .get_starred2(None)
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
                    None,
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

    /// Reads one playlist's rules and edit bookkeeping from Navidrome's own API, which the
    /// Subsonic playlist calls have no answer for.
    async fn smart_playlist(&self, playlist_id: &str) -> Result<SmartPlaylist> {
        let response = self
            .navidrome_send(
                reqwest::Method::GET,
                &format!("/api/playlist/{}", escape::component(playlist_id)),
                None,
            )
            .await?;
        if !response.status().is_success() {
            return Err(Self::playlist_refusal(response).await);
        }
        let value: serde_json::Value = response
            .json()
            .await
            .context("navidrome answered no json for the playlist")?;
        Ok(self.read_smart_playlist(&value)?)
    }

    /// Creates a smart playlist from rules, going through Navidrome's own API since the
    /// Subsonic create call carries none. The server owns the new playlist's id and owner.
    async fn create_smart_playlist(&self, name: &str, rules: &SmartRules) -> Result<String> {
        let problems = rules.problems();
        if !problems.is_empty() {
            anyhow::bail!("the rules are not ready to save: {}", describe(&problems));
        }
        let response = self
            .navidrome_send(
                reqwest::Method::POST,
                "/api/playlist",
                Some(serde_json::json!({
                    "name": name,
                    "rules": rules.to_wire(),
                })),
            )
            .await?;
        if !response.status().is_success() {
            return Err(Self::playlist_refusal(response).await);
        }
        let value: serde_json::Value = response
            .json()
            .await
            .context("navidrome answered no json for the new playlist")?;
        value["id"]
            .as_str()
            .map(str::to_owned)
            .context("navidrome answered no id for the new playlist")
    }

    /// Writes the fields `update` names through Navidrome's own API. The server changes only
    /// the fields the body carries, so a save never sends a field the edit did not touch, and
    /// rules the editor cannot show come back unchanged inside the tree.
    async fn update_smart_playlist(
        &self,
        playlist_id: &str,
        update: SmartPlaylistUpdate,
    ) -> Result<SmartPlaylist> {
        if update.is_empty() {
            anyhow::bail!("the edit changes nothing");
        }
        if let Some(rules) = &update.rules {
            let problems = rules.problems();
            if !problems.is_empty() {
                anyhow::bail!("the rules are not ready to save: {}", describe(&problems));
            }
        }
        let response = self
            .navidrome_send(
                reqwest::Method::PUT,
                &format!("/api/playlist/{}", escape::component(playlist_id)),
                Some(update.to_wire()),
            )
            .await?;
        if !response.status().is_success() {
            return Err(Self::playlist_refusal(response).await);
        }
        let value: serde_json::Value = response
            .json()
            .await
            .context("navidrome answered no json for the saved playlist")?;
        Ok(self.read_smart_playlist(&value)?)
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
            .get_starred2(None)
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

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let starred = self
            .client
            .get_starred2(None)
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
            .get_artists(None)
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
            .get_random_songs(Some(20), None, None, None, None)
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
            .playlists()
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
                None,
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

/// The rule problems of one save, joined for an error message.
fn describe(problems: &[crate::RuleProblem]) -> String {
    problems
        .iter()
        .map(|problem| format!("{}: {}", problem.path, problem.problem))
        .collect::<Vec<_>>()
        .join("; ")
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
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::*;
    use crate::smart::{Condition, Operand, Operator, Rule, RuleGroup};

    /// A canned Navidrome: one thread per request, routing on method and path, keeping what
    /// it was asked so a test can check the exact payload that went out.
    struct Server {
        url: String,
        requests: Arc<Mutex<Vec<(String, String, String)>>>,
    }

    impl Server {
        fn start(playlists: Vec<Value>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
            let url = format!("http://{}", listener.local_addr().expect("the address"));
            let requests: Arc<Mutex<Vec<(String, String, String)>>> = Arc::default();
            let store: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(playlists));
            let thread = {
                let requests = requests.clone();
                let store = store.clone();
                std::thread::spawn(move || {
                    for stream in listener.incoming() {
                        let Ok(mut stream) = stream else { return };
                        let mut reader = BufReader::new(stream.try_clone().expect("a clone"));
                        let mut request_line = String::new();
                        if reader.read_line(&mut request_line).is_err() {
                            return;
                        }
                        let mut length = 0;
                        let mut authorization = String::new();
                        loop {
                            let mut header = String::new();
                            if reader.read_line(&mut header).is_err() || header == "\r\n" {
                                break;
                            }
                            if let Some((name, value)) = header.trim().split_once(':') {
                                match name.to_ascii_lowercase().as_str() {
                                    "content-length" => length = value.trim().parse().unwrap_or(0),
                                    "x-nd-authorization" => authorization = value.trim().to_owned(),
                                    _ => {}
                                }
                            }
                        }
                        let mut body = vec![0; length];
                        if reader.read_exact(&mut body).is_err() {
                            return;
                        }
                        let parts: Vec<&str> = request_line.split_whitespace().collect();
                        let (method, path) = (
                            parts.first().copied().unwrap_or_default().to_owned(),
                            parts.get(1).copied().unwrap_or_default().to_owned(),
                        );
                        let body = String::from_utf8_lossy(&body).to_string();
                        requests.lock().expect("the log").push((
                            method.clone(),
                            path.clone(),
                            body.clone(),
                        ));
                        let (status, answer) = route(&method, &path, &body, &authorization, &store);
                        let answer = answer.to_string();
                        write!(
                            stream,
                            "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                            answer.len()
                        )
                        .expect("a response");
                    }
                })
            };
            let _ = thread;
            Server { url, requests }
        }

        fn client(&self) -> SubsonicClient {
            let signature = crate::subsonic::auth::sign("judith", "secret");
            SubsonicClient::new(
                self.url.clone(),
                "judith".into(),
                "secret".into(),
                &signature,
            )
            .expect("a client")
        }

        fn requests(&self) -> Vec<(String, String, String)> {
            self.requests.lock().expect("the log").clone()
        }
    }

    fn route(
        method: &str,
        path: &str,
        body: &str,
        authorization: &str,
        store: &Arc<Mutex<Vec<Value>>>,
    ) -> (u16, Value) {
        if path == "/auth/login" {
            let credentials: Value = serde_json::from_str(body).unwrap_or_default();
            return match credentials["password"].as_str() {
                Some("secret") => (200, json!({"token": "the-token", "id": "u1"})),
                _ => (401, json!({"error": "invalid credentials"})),
            };
        }
        if path.starts_with("/rest/ping") {
            return (
                200,
                json!({"subsonic-response": {
                    "status": "ok",
                    "type": "navidrome",
                    "serverVersion": "0.64.2"
                }}),
            );
        }
        if authorization != "Bearer the-token" {
            return (401, json!({"error": "unauthorized"}));
        }
        let playlists = store.lock().expect("the store");
        if method == "POST" && path == "/api/playlist" {
            let asked: Value = serde_json::from_str(body).unwrap_or_default();
            return (200, json!({"id": "pl-new", "name": asked["name"]}));
        }
        if let Some(id) = path.strip_prefix("/api/playlist/") {
            let found = playlists
                .iter()
                .find(|playlist| playlist["id"] == id)
                .cloned();
            return match (method, found) {
                (_, None) => (404, json!({"error": "not found"})),
                ("GET", Some(playlist)) => (200, playlist),
                ("PUT", Some(playlist))
                    if playlist["ownerName"]
                        .as_str()
                        .is_some_and(|owner| owner.eq_ignore_ascii_case("judith")) =>
                {
                    let mut updated = playlist.clone();
                    let changes: Value = serde_json::from_str(body).unwrap_or_default();
                    if let Some(fields) = changes.as_object() {
                        for (key, value) in fields {
                            updated[key] = value.clone();
                        }
                    }
                    (200, updated)
                }
                ("PUT", Some(_)) => (403, json!({"error": "permission denied"})),
                _ => (405, json!({"error": "method not allowed"})),
            };
        }
        (404, json!({"error": "no route"}))
    }

    fn smart_playlist_json() -> Value {
        json!({
            "id": "pl-1",
            "name": "Quiet mornings",
            "comment": "",
            "public": false,
            "ownerId": "u1",
            "ownerName": "Judith",
            "songCount": 12,
            "sync": false,
            "path": "",
            "rules": {
                "all": [
                    {"contains": {"title": "love"}},
                    {"gt": {"playcount": 3}}
                ],
                "sort": "dateloved",
                "order": "desc",
                "limit": 50
            }
        })
    }

    #[tokio::test]
    async fn the_capability_reads_the_ping_once() {
        let server = Server::start(Vec::new());
        let client = server.client();
        assert!(client.takes_smart_playlists().await);
        assert!(client.takes_playlist_covers().await);
        let pings = server
            .requests()
            .into_iter()
            .filter(|(_, path, _)| path.starts_with("/rest/ping"))
            .count();
        assert_eq!(pings, 1);
    }

    #[tokio::test]
    async fn a_smart_playlist_reads_with_its_rules() {
        let server = Server::start(vec![smart_playlist_json()]);
        let client = server.client();
        let playlist = client.smart_playlist("pl-1").await.expect("the playlist");
        assert_eq!(playlist.name, "Quiet mornings");
        assert!(playlist.owned, "ownership compares without case");
        assert!(!playlist.synced);
        assert_eq!(playlist.track_count, 12);
        let rules = playlist.rules.expect("the rules");
        assert_eq!(rules.sort.as_deref(), Some("dateloved"));
        assert_eq!(rules.limit, Some(50));
        assert_eq!(rules.group.rules().len(), 2);
        let (_, _, body) = &server.requests()[1];
        assert_eq!(server.requests()[0].1, "/auth/login");
        assert!(body.is_empty(), "a read sends no body: {body}");
    }

    #[tokio::test]
    async fn a_run_of_calls_signs_in_once() {
        let server = Server::start(vec![smart_playlist_json()]);
        let client = server.client();
        client.smart_playlist("pl-1").await.expect("the read");
        client
            .smart_playlist("pl-1")
            .await
            .expect("the second read");
        let logins = server
            .requests()
            .iter()
            .filter(|(_, path, _)| path == "/auth/login")
            .count();
        assert_eq!(
            logins, 1,
            "navidrome rate-limits its sign-in, so calls share one token"
        );
        assert_eq!(server.requests()[1].1, "/api/playlist/pl-1");
        assert_eq!(server.requests()[2].1, "/api/playlist/pl-1");
    }

    #[tokio::test]
    async fn concurrent_expired_requests_share_one_refresh() {
        let server = Server::start(vec![smart_playlist_json()]);
        let client = server.client();
        *client.token.lock().await = Some("expired-token".into());
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let client = client.clone();
            tasks.spawn(async move { client.smart_playlist("pl-1").await });
        }
        while let Some(result) = tasks.join_next().await {
            result.expect("the task").expect("the refreshed read");
        }
        let logins = server
            .requests()
            .iter()
            .filter(|(_, path, _)| path == "/auth/login")
            .count();
        assert_eq!(logins, 1, "expired requests must share their refresh");
    }

    #[tokio::test]
    async fn a_failed_refresh_does_not_retry_login() {
        let server = Server::start(vec![smart_playlist_json()]);
        let mut client = server.client();
        client.password = "incorrect".into();
        *client.token.lock().await = Some("expired-token".into());
        assert!(client.smart_playlist("pl-1").await.is_err());
        let logins = server
            .requests()
            .iter()
            .filter(|(_, path, _)| path == "/auth/login")
            .count();
        assert_eq!(logins, 1, "failed authentication must not loop");
    }

    #[tokio::test]
    async fn a_synced_playlist_is_marked_for_the_warning() {
        let mut synced = smart_playlist_json();
        synced["sync"] = json!(true);
        synced["path"] = json!("/music/playlists/quiet.nsp");
        let server = Server::start(vec![synced]);
        let playlist = server
            .client()
            .smart_playlist("pl-1")
            .await
            .expect("the playlist");
        assert!(playlist.synced);
    }

    #[tokio::test]
    async fn an_ordinary_playlist_reads_without_rules() {
        let mut plain = smart_playlist_json();
        plain["rules"] = Value::Null;
        let server = Server::start(vec![plain]);
        let playlist = server
            .client()
            .smart_playlist("pl-1")
            .await
            .expect("the playlist");
        assert_eq!(playlist.rules, None);
    }

    #[tokio::test]
    async fn a_create_sends_name_and_rules_nothing_else() {
        let server = Server::start(Vec::new());
        let client = server.client();
        let rules = SmartRules::from_wire(&json!({
            "all": [{"is": {"loved": true}}]
        }))
        .expect("the rules");
        let id = client
            .create_smart_playlist("Favorites", &rules)
            .await
            .expect("the id");
        assert_eq!(id, "pl-new");
        let (method, path, body) = &server.requests()[1];
        assert_eq!((method.as_str(), path.as_str()), ("POST", "/api/playlist"));
        let sent: Value = serde_json::from_str(body).expect("json");
        assert_eq!(
            sent,
            json!({"name": "Favorites", "rules": {"all": [{"is": {"loved": true}}]}})
        );
    }

    #[tokio::test]
    async fn an_update_sends_only_the_changed_fields() {
        let server = Server::start(vec![smart_playlist_json()]);
        let client = server.client();
        let update = SmartPlaylistUpdate {
            public: Some(true),
            ..SmartPlaylistUpdate::default()
        };
        let playlist = client
            .update_smart_playlist("pl-1", update)
            .await
            .expect("the save");
        assert!(playlist.public);
        let (method, path, body) = &server.requests()[1];
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("PUT", "/api/playlist/pl-1")
        );
        let sent: Value = serde_json::from_str(body).expect("json");
        assert_eq!(sent, json!({"public": true}), "no rules re-sent: {sent}");
    }

    #[tokio::test]
    async fn a_rule_update_round_trips_through_the_server_answer() {
        let server = Server::start(vec![smart_playlist_json()]);
        let client = server.client();
        let rules = SmartRules::from_wire(&json!({
            "any": [{"is": {"loved": true}}],
            "sort": "random"
        }))
        .expect("the rules");
        let playlist = client
            .update_smart_playlist(
                "pl-1",
                SmartPlaylistUpdate {
                    rules: Some(rules),
                    ..SmartPlaylistUpdate::default()
                },
            )
            .await
            .expect("the save");
        let rules = playlist.rules.expect("the rules back");
        assert!(matches!(rules.group, RuleGroup::Any(_)));
        assert_eq!(rules.sort.as_deref(), Some("random"));
    }

    #[tokio::test]
    async fn editing_another_owners_playlist_is_forbidden() {
        let mut foreign = smart_playlist_json();
        foreign["ownerName"] = json!("someone-else");
        let server = Server::start(vec![foreign]);
        let client = server.client();
        let error = client
            .update_smart_playlist(
                "pl-1",
                SmartPlaylistUpdate {
                    name: Some("mine now".into()),
                    ..SmartPlaylistUpdate::default()
                },
            )
            .await
            .expect_err("the refusal");
        assert!(
            format!("{error:#}").contains("refuses the change for this account"),
            "the error reads for the listener: {error:#}"
        );
        let playlist = client.smart_playlist("pl-1").await.expect("the read");
        assert!(!playlist.owned);
    }

    #[tokio::test]
    async fn a_gone_playlist_says_so() {
        let server = Server::start(Vec::new());
        let error = server
            .client()
            .smart_playlist("pl-missing")
            .await
            .expect_err("the refusal");
        assert!(format!("{error:#}").contains("no longer has the playlist"));
    }

    #[tokio::test]
    async fn broken_rules_never_leave_the_client() {
        let server = Server::start(Vec::new());
        let client = server.client();
        let mut rules = SmartRules::new(RuleGroup::All(Vec::new()));
        let error = client
            .create_smart_playlist("Empty", &rules)
            .await
            .expect_err("the refusal");
        assert!(format!("{error:#}").contains("not ready to save"));
        rules.order = Some("sideways".into());
        rules.group.rules_mut().push(Rule::Condition(Condition {
            operator: Operator::Is,
            field: "loved".into(),
            value: Operand::Boolean(true),
        }));
        let error = client
            .update_smart_playlist(
                "pl-1",
                SmartPlaylistUpdate {
                    rules: Some(rules),
                    ..SmartPlaylistUpdate::default()
                },
            )
            .await
            .expect_err("the refusal");
        assert!(format!("{error:#}").contains("not ready to save"));
        // The checks run before any sign-in: no request carried broken rules.
        assert!(server.requests().is_empty());
    }

    #[tokio::test]
    async fn a_refused_sign_in_reads_as_credentials() {
        let server = Server::start(Vec::new());
        let signature = crate::subsonic::auth::sign("judith", "wrong");
        let client = SubsonicClient::new(
            server.url.clone(),
            "judith".into(),
            "wrong".into(),
            &signature,
        )
        .expect("a client");
        let error = client
            .smart_playlist("pl-1")
            .await
            .expect_err("the refusal");
        assert!(format!("{error:#}").contains("refused the sign-in"));
    }
}
