//! Yandex Music's mobile api at `api.music.yandex.net`, the one the Android app talks to. Every
//! call carries the account's OAuth token, and every answer wraps what was asked for in a
//! `result`, or says why not in an `error`.
//!
//! The endpoints follow the community clients, chiefly
//! [yandex-music-rs](https://github.com/vyfor/yandex-music-rs) and MarshalX's
//! `yandex-music-api`, since Yandex documents none of them.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::StreamExt as _;
use serde_json::{Value, json};

use crate::yandex::{decrypt, wire};
use crate::{
    Album, AlbumDetail, Artist, ArtistCatalogue, ArtistProfile, ArtistRef, CoverSize, Encoding,
    MediaKind, MusicApi, Offer, Playlist, PlaylistDetail, Quality, SavedArtist, SignInFailure,
    SignInProblem, Throttled, Track, UserProfile, distinct_covers, escape,
};

pub(crate) const API: &str = "https://api.music.yandex.net";
/// What the Android app calls itself, which some endpoints want before they answer in full.
const CLIENT: &str = "YandexMusicAndroid/24023621";
const AGENT: &str = "Yandex-Music-API";
/// Where the website shows a track, an album, an artist or a playlist.
const SITE: &str = "https://music.yandex.ru";

/// How many tracks one `tracks` lookup asks for, and how many lookups run at once.
const LOOKUP: usize = 250;
const LOOKUPS_AT_ONCE: usize = 4;
/// How many releases an artist page asks for at once.
const DISCOGRAPHY: u32 = 200;
/// How many results a search asks for.
const SEARCH_PAGE: u32 = 30;

/// The key `get-file-info` asks its query to be signed with, the Android app's own.
const SIGN_KEY: &[u8] = b"p93jhgh689SBReK6ghtw62";
/// Every codec a file may come in, plain containers first. Yandex picks one for the level.
const CODECS: [&str; 7] = [
    "flac",
    "aac",
    "he-aac",
    "mp3",
    "flac-mp4",
    "aac-mp4",
    "he-aac-mp4",
];
/// Files come encrypted; asking for them in the clear gets the lossless ones refused.
const TRANSPORT: &str = "encraw";

/// The levels Yandex serves, best first: lossless, `nq` and `lq`.
pub(crate) const QUALITIES: &[Offer] = &[
    Offer {
        quality: Quality::Lossless,
        encoding: Encoding::Flac {
            depth: 24,
            rate: 48_000,
        },
        up_to: true,
    },
    Offer {
        quality: Quality::High,
        encoding: Encoding::Aac { kbps: 256 },
        up_to: true,
    },
    Offer {
        quality: Quality::Low,
        encoding: Encoding::Aac { kbps: 64 },
        up_to: true,
    },
];

/// A track's file as it starts to arrive, with what decrypts it and what it is.
pub struct Opened {
    pub response: reqwest::Response,
    /// The hex key the file is encrypted with, or `None` for a file sent in the clear.
    pub key: Option<String>,
    /// The codec `get-file-info` names, such as `flac` or `aac-mp4`.
    pub codec: Option<String>,
    /// The bitrate in kilobits a second, zero for a lossless file.
    pub bitrate: Option<u64>,
}

#[derive(Clone)]
pub struct YandexClient {
    pub(crate) http: reqwest::Client,
    profile: UserProfile,
}

impl YandexClient {
    /// Signs in with an OAuth token: reads the account it belongs to, which also proves the
    /// token is still good.
    pub async fn connect(token: &str) -> Result<Self> {
        let mut client = Self::unopened(token)?;
        let status = client
            .get("/account/status")
            .await
            .context("cannot read the account")?;
        client.profile = wire::profile(&status);
        if client.profile.id.is_empty() {
            return Err(SignInFailure(SignInProblem::Credentials))
                .context("yandex answered with no account for that token");
        }
        Ok(client)
    }

    /// A client for `token` that has not reached Yandex, for playing what is already kept.
    pub fn offline(token: &str) -> Result<Self> {
        Self::unopened(token)
    }

    fn unopened(token: &str) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        let mut authorization = reqwest::header::HeaderValue::from_str(&format!("OAuth {token}"))
            .context("the token holds characters a header cannot")?;
        authorization.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, authorization);
        headers.insert(
            "X-Yandex-Music-Client",
            reqwest::header::HeaderValue::from_static(CLIENT),
        );
        let http = reqwest::Client::builder()
            .user_agent(AGENT)
            .default_headers(headers)
            .build()
            .context("cannot build the yandex http client")?;
        Ok(Self {
            http,
            profile: UserProfile {
                id: String::new(),
                display_name: String::new(),
                avatar: None,
            },
        })
    }

    fn uid(&self) -> &str {
        &self.profile.id
    }

    pub(crate) async fn get(&self, path: &str) -> Result<Value> {
        let response = self
            .http
            .get(format!("{API}{path}"))
            .send()
            .await
            .context("cannot reach yandex music")?;
        answer(response).await
    }

    async fn post(&self, path: &str, form: &[(&str, &str)]) -> Result<Value> {
        let response = self
            .http
            .post(format!("{API}{path}"))
            .form(form)
            .send()
            .await
            .context("cannot reach yandex music")?;
        answer(response).await
    }

    /// Full tracks for `ids`, each a track id or `track:album`, in the order asked for. Big
    /// lists are looked up a slice at a time, a few slices at once.
    async fn tracks(&self, ids: &[String]) -> Result<Vec<Track>> {
        let lookups: Vec<_> = ids
            .chunks(LOOKUP)
            .map(|chunk| {
                let (client, joined) = (self.clone(), chunk.join(","));
                async move {
                    let found = client
                        .post(
                            "/tracks",
                            &[("track-ids", &joined), ("with-positions", "true")],
                        )
                        .await?;
                    Ok(found
                        .as_array()
                        .map(|list| wire::tracks(list))
                        .unwrap_or_default())
                }
            })
            .collect();
        let pages: Vec<Result<Vec<Track>>> = futures::stream::iter(lookups)
            .buffered(LOOKUPS_AT_ONCE)
            .collect()
            .await;
        let mut tracks = Vec::with_capacity(ids.len());
        for page in pages {
            tracks.extend(page?);
        }
        Ok(tracks)
    }

    /// Likes or unlikes items of one `kind` (`track`, `album`, `artist` or `playlist`).
    async fn like(&self, kind: &str, id: &str, liked: bool) -> Result<()> {
        let action = if liked { "add-multiple" } else { "remove" };
        let path = format!(
            "/users/{}/likes/{kind}s/{action}",
            escape::component(self.uid())
        );
        self.post(&path, &[(&format!("{kind}-ids"), id)])
            .await
            .with_context(|| format!("cannot change the like on {kind} {id}"))?;
        Ok(())
    }

    /// A playlist as the api answers it, with its tracks in full.
    async fn playlist_value(&self, playlist_id: &str) -> Result<Value> {
        let (owner, kind) = wire::split_playlist(playlist_id)
            .with_context(|| format!("{playlist_id} is not a yandex playlist id"))?;
        self.get(&format!(
            "/users/{}/playlists/{}?richTracks=true",
            escape::component(owner),
            escape::component(kind)
        ))
        .await
        .with_context(|| format!("cannot load the playlist {playlist_id}"))
    }

    /// The kind of one of the account's own playlists, refusing someone else's.
    fn own_kind<'a>(&self, playlist_id: &'a str) -> Result<&'a str> {
        match wire::split_playlist(playlist_id) {
            Some((owner, kind)) if owner == self.uid() => Ok(kind),
            Some(_) => bail!("only your own playlists can be changed"),
            None => bail!("{playlist_id} is not a yandex playlist id"),
        }
    }

    /// The tracks of a playlist answer. A rich answer carries every track; a short one only
    /// names them, and the rest is looked up.
    async fn playlist_entries(&self, value: &Value) -> Result<Vec<Track>> {
        let entries = wire::list(value, "tracks");
        if entries.iter().all(|entry| entry.get("track").is_some()) {
            return Ok(entries.iter().filter_map(wire::entry).collect());
        }
        let ids: Vec<String> = entries.iter().filter_map(short_id).collect();
        self.tracks(&ids).await
    }

    /// Applies one change to an owned playlist, at the revision it was read at.
    async fn change(&self, playlist_id: &str, revision: u64, diff: Value) -> Result<()> {
        let kind = self.own_kind(playlist_id)?;
        let path = format!(
            "/users/{}/playlists/{}/change-relative",
            escape::component(self.uid()),
            escape::component(kind)
        );
        let diff = json!([diff]).to_string();
        self.post(
            &path,
            &[("diff", &diff), ("revision", &revision.to_string())],
        )
        .await
        .context("cannot change the playlist")?;
        Ok(())
    }

    async fn search_kind(&self, query: &str, kind: &str) -> Result<Value> {
        let found = self
            .get(&format!(
                "/search?text={}&type={kind}&page=0&page-size={SEARCH_PAGE}&nocorrect=false",
                escape::component(query)
            ))
            .await
            .context("cannot search yandex music")?;
        Ok(found[format!("{kind}s")]["results"].clone())
    }

    async fn brief(&self, artist_id: &str) -> Result<Value> {
        self.get(&format!(
            "/artists/{}/brief-info",
            escape::component(artist_id)
        ))
        .await
        .with_context(|| format!("cannot load the artist {artist_id}"))
    }

    /// The station seeded by a track, from its start or after the track `queue` names.
    async fn station(&self, track_id: &str, queue: Option<&str>) -> Result<Vec<Track>> {
        let mut path = format!(
            "/rotor/station/track:{}/tracks?settings2=true",
            escape::component(track_id)
        );
        if let Some(queue) = queue {
            path.push_str(&format!("&queue={}", escape::component(queue)));
        }
        let station = self.get(&path).await?;
        Ok(wire::list(&station, "sequence")
            .iter()
            .filter_map(|item| wire::track(&item["track"]))
            .collect())
    }

    async fn similar(&self, track_id: &str) -> Result<Vec<Track>> {
        let similar = self
            .get(&format!("/tracks/{}/similar", escape::component(track_id)))
            .await?;
        Ok(wire::tracks(wire::list(&similar, "similarTracks")))
    }

    /// What `get-file-info` calls `quality`, the level a file is asked for at.
    pub(crate) fn served(quality: Quality) -> &'static str {
        match quality.within(QUALITIES) {
            Quality::HiRes | Quality::Lossless => "lossless",
            Quality::High | Quality::Normal => "nq",
            Quality::Low => "lq",
        }
    }

    /// Asks where a track's file is at `quality` and starts downloading it.
    pub async fn open_stream(&self, track_id: &str, quality: Quality) -> Result<Opened> {
        let served = Self::served(quality);
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        let sign = sign(&ts, track_id, served);
        let path = format!(
            "/get-file-info?ts={ts}&trackId={}&quality={served}&codecs={}&transports={TRANSPORT}&sign={}",
            escape::component(track_id),
            escape::component(&CODECS.join(",")),
            escape::component(&sign),
        );
        let info = self
            .get(&path)
            .await
            .with_context(|| format!("cannot find the file of {track_id}"))?;
        let info = &info["downloadInfo"];
        let url = wire::text(info, &["url"])
            .or_else(|| {
                wire::list(info, "urls")
                    .first()
                    .and_then(|url| url.as_str())
            })
            .with_context(|| format!("yandex named no file for {track_id}"))?;
        let response = self
            .http
            .get(url)
            .send()
            .await
            .context("cannot reach the yandex storage")?
            .error_for_status()
            .context("the yandex storage refused the file")?;
        let key = wire::text(info, &["key"])
            .filter(|_| wire::text(info, &["transport"]) != Some("raw"))
            .map(str::to_owned);
        Ok(Opened {
            response,
            key,
            codec: wire::text(info, &["codec"]).map(str::to_owned),
            bitrate: wire::number(info, &["bitrate"]),
        })
    }
}

/// The `get-file-info` signature: an HMAC-SHA256 of the query's values run together, in
/// base64 with its last character dropped, as the Android app sends it.
fn sign(ts: &str, track_id: &str, quality: &str) -> String {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, SIGN_KEY);
    let message = format!("{ts}{track_id}{quality}{}{TRANSPORT}", CODECS.concat());
    let mut sign = STANDARD.encode(ring::hmac::sign(&key, message.as_bytes()));
    sign.pop();
    sign
}

/// What a codec `get-file-info` names is, at its bitrate in kilobits a second. FLAC is left
/// for the decoder to tell, since nothing here says its depth or its rate.
pub(crate) fn encoding(codec: Option<&str>, kbps: Option<u64>) -> Option<Encoding> {
    let kbps = kbps.unwrap_or_default() as u32;
    Some(match codec? {
        "flac" | "flac-mp4" => return None,
        "aac" | "he-aac" | "aac-mp4" | "he-aac-mp4" => Encoding::Aac { kbps },
        "mp3" => Encoding::Mp3 { kbps },
        _ => Encoding::Other,
    })
}

/// The key for an encrypted file, read from the hex `get-file-info` names it by.
pub(crate) fn file_key(hex: Option<&str>) -> Result<Option<decrypt::Key>> {
    hex.map(decrypt::key).transpose()
}

/// Reads an answer: its `result` when it has one, or the reason it was refused.
async fn answer(response: reqwest::Response) -> Result<Value> {
    let status = response.status();
    let body: Value = response
        .json()
        .await
        .with_context(|| format!("yandex answered {status} with something other than json"))?;
    let error = &body["error"];
    let reason = wire::text(error, &["message", "name"])
        .or_else(|| error.as_str())
        .unwrap_or_default()
        .to_owned();
    match status.as_u16() {
        401 | 403 => {
            return Err(SignInFailure(SignInProblem::Credentials))
                .with_context(|| format!("yandex refused the token: {reason}"));
        }
        429 => return Err(Throttled).context("yandex is refusing for now"),
        _ => {}
    }
    if !status.is_success() || !error.is_null() {
        bail!("yandex answered {status}: {reason}");
    }
    Ok(body.get("result").cloned().unwrap_or(body))
}

/// The `track:album` id a short playlist entry or a like names its track by.
fn short_id(entry: &Value) -> Option<String> {
    let track = wire::id(&entry["id"])?;
    Some(match wire::id(&entry["albumId"]) {
        Some(album) => format!("{track}:{album}"),
        None => track,
    })
}

#[async_trait]
impl MusicApi for YandexClient {
    fn cover_at(&self, cover: &str, size: CoverSize) -> Option<String> {
        let size = match size {
            CoverSize::Large => wire::LARGE,
            CoverSize::Full => wire::FULL,
        };
        wire::resize(cover, size)
    }

    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String> {
        let path = match kind {
            MediaKind::Track => format!("track/{}", escape::component(id)),
            MediaKind::Album => format!("album/{}", escape::component(id)),
            MediaKind::Artist => format!("artist/{}", escape::component(id)),
            MediaKind::Playlist => {
                let (owner, kind) = wire::split_playlist(id)?;
                format!(
                    "users/{}/playlists/{}",
                    escape::component(owner),
                    escape::component(kind)
                )
            }
        };
        Some(format!("{SITE}/{path}"))
    }

    async fn profile(&self) -> Result<UserProfile> {
        Ok(self.profile.clone())
    }

    async fn artist(&self, artist_id: &str) -> Result<Artist> {
        let brief = self.brief(artist_id).await?;
        let artist = &brief["artist"];
        let own = ArtistRef {
            name: wire::text(artist, &["name"]).unwrap_or_default().to_owned(),
            id: Some(artist_id.to_owned()),
        };
        Ok(Artist {
            name: own.name.clone(),
            cover_large: wire::portrait(artist, wire::LARGE),
            biography: wire::text(&artist["description"], &["text"]).map(str::to_owned),
            monthly_listeners: wire::number(&brief["stats"], &["lastMonthListeners"]),
            top_tracks: wire::tracks(wire::list(&brief, "popularTracks")),
            albums: wire::list(&brief, "albums")
                .iter()
                .filter_map(wire::album)
                .map(|album| album.credit(&own))
                .collect(),
        })
    }

    async fn artist_catalogue(&self, artist_id: &str, _known: &[Track]) -> Result<ArtistCatalogue> {
        let path = format!(
            "/artists/{}/direct-albums?page=0&page-size={DISCOGRAPHY}&sort-by=year",
            escape::component(artist_id)
        );
        let (albums, brief) = tokio::join!(self.get(&path), self.brief(artist_id));
        let albums = albums.context("cannot load the discography")?;
        Ok(ArtistCatalogue {
            albums: wire::list(&albums, "albums")
                .iter()
                .filter_map(wire::album)
                .collect(),
            top_tracks: Vec::new(),
            appears_on: brief
                .map(|brief| {
                    wire::list(&brief, "alsoAlbums")
                        .iter()
                        .filter_map(wire::album)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile> {
        let brief = self.brief(artist_id).await?;
        let artist = &brief["artist"];
        Ok(ArtistProfile {
            name: wire::text(artist, &["name"]).unwrap_or_default().to_owned(),
            cover_large: wire::portrait(artist, wire::LARGE),
            biography: wire::text(&artist["description"], &["text"]).map(str::to_owned),
        })
    }

    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let found = self
            .post("/artists", &[("artist-ids", &ids.join(","))])
            .await
            .context("cannot load the artists")?;
        Ok(found
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(wire::saved_artist)
            .filter_map(|artist| Some((artist.id, artist.cover?)))
            .collect())
    }

    async fn saved_tracks(&self) -> Result<Vec<Track>> {
        let likes = self
            .get(&format!(
                "/users/{}/likes/tracks",
                escape::component(self.uid())
            ))
            .await
            .context("cannot load the liked tracks")?;
        let entries = wire::list(&likes["library"], "tracks");
        let ids: Vec<String> = entries.iter().filter_map(short_id).collect();
        let added: HashMap<String, i64> = entries
            .iter()
            .filter_map(|entry| {
                let at = crate::iso_8601_to_epoch(wire::text(entry, &["timestamp"]))?;
                Some((wire::id(&entry["id"])?, at))
            })
            .collect();
        let mut tracks = self.tracks(&ids).await?;
        for track in &mut tracks {
            track.added_at = track.id.as_ref().and_then(|id| added.get(id).copied());
        }
        Ok(tracks)
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()> {
        self.like("track", track_id, saved).await
    }

    async fn track(&self, track_id: &str) -> Result<Track> {
        let found = self
            .get(&format!("/tracks/{}", escape::component(track_id)))
            .await
            .with_context(|| format!("cannot load the track {track_id}"))?;
        found
            .as_array()
            .and_then(|list| list.first())
            .and_then(wire::track)
            .with_context(|| format!("cannot read the yandex track {track_id}"))
    }

    async fn track_playcount(&self, _track_id: &str) -> Result<Option<u64>> {
        Ok(None)
    }

    async fn playlists(&self) -> Result<Vec<Playlist>> {
        let uid = escape::component(self.uid());
        let (own_path, liked_path) = (
            format!("/users/{uid}/playlists/list"),
            format!("/users/{uid}/likes/playlists"),
        );
        let (own, liked) = tokio::join!(self.get(&own_path), self.get(&liked_path));
        let own = own.context("cannot load the playlists")?;
        let liked = liked
            .inspect_err(|error| log::warn!("yandex: cannot load the liked playlists: {error:#}"))
            .unwrap_or_default();
        let mut seen = HashSet::new();
        Ok([own, liked]
            .iter()
            .flat_map(|list| list.as_array().map(Vec::as_slice).unwrap_or_default())
            .filter_map(|value| wire::playlist(value, self.uid()))
            .filter(|playlist| seen.insert(playlist.id.clone()))
            .collect())
    }

    async fn create_playlist(&self, name: &str) -> Result<String> {
        let created = self
            .post(
                &format!("/users/{}/playlists/create", escape::component(self.uid())),
                &[("title", name), ("visibility", "private")],
            )
            .await
            .context("cannot create the playlist")?;
        wire::playlist(&created, self.uid())
            .map(|playlist| playlist.id)
            .context("the created playlist came back with no id")
    }

    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()> {
        let kind = self.own_kind(playlist_id)?;
        self.post(
            &format!(
                "/users/{}/playlists/{}/name",
                escape::component(self.uid()),
                escape::component(kind)
            ),
            &[("value", name)],
        )
        .await
        .context("cannot rename the playlist")?;
        Ok(())
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let kind = self.own_kind(playlist_id)?;
        self.post(
            &format!(
                "/users/{}/playlists/{}/delete",
                escape::component(self.uid()),
                escape::component(kind)
            ),
            &[],
        )
        .await
        .context("cannot delete the playlist")?;
        Ok(())
    }

    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()> {
        self.like("playlist", playlist_id, false).await
    }

    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()> {
        self.like("playlist", playlist_id, true).await
    }

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()> {
        let kind = self.own_kind(playlist_id)?;
        let visibility = if public { "public" } else { "private" };
        self.post(
            &format!(
                "/users/{}/playlists/{}/visibility",
                escape::component(self.uid()),
                escape::component(kind)
            ),
            &[("value", visibility)],
        )
        .await
        .context("cannot change the playlist's visibility")?;
        Ok(())
    }

    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        self.add_tracks_to_playlist(playlist_id, &[track_id.to_owned()])
            .await
    }

    /// Inserts every track at the end in one change. The api wants each track's album too,
    /// so the tracks are looked up first.
    async fn add_tracks_to_playlist(&self, playlist_id: &str, track_ids: &[String]) -> Result<()> {
        let (playlist, tracks) =
            tokio::join!(self.playlist_value(playlist_id), self.tracks(track_ids));
        let (playlist, tracks) = (playlist?, tracks?);
        let albums: HashMap<&str, &str> = tracks
            .iter()
            .filter_map(|track| Some((track.id.as_deref()?, track.album_id.as_deref()?)))
            .collect();
        let entries: Vec<Value> = track_ids
            .iter()
            .map(|id| match albums.get(id.as_str()) {
                Some(album) => json!({ "id": id, "albumId": album }),
                None => json!({ "id": id }),
            })
            .collect();
        let at = wire::number(&playlist, &["trackCount"])
            .unwrap_or(wire::list(&playlist, "tracks").len() as u64);
        let revision = wire::number(&playlist, &["revision"]).unwrap_or_default();
        self.change(
            playlist_id,
            revision,
            json!({ "op": "insert", "at": at, "tracks": entries }),
        )
        .await
    }

    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()> {
        let playlist = self.playlist_value(playlist_id).await?;
        let entries = wire::list(&playlist, "tracks");
        let at = entries
            .iter()
            .position(|entry| {
                wire::id(&entry["id"]).as_deref() == Some(track_id)
                    || wire::id(&entry["track"]["id"]).as_deref() == Some(track_id)
            })
            .with_context(|| format!("the playlist does not hold the track {track_id}"))?;
        let album = wire::id(&entries[at]["albumId"]).or_else(|| {
            wire::list(&entries[at]["track"], "albums")
                .first()
                .and_then(|album| wire::id(&album["id"]))
        });
        let revision = wire::number(&playlist, &["revision"]).unwrap_or_default();
        self.change(
            playlist_id,
            revision,
            json!({
                "op": "delete",
                "from": at,
                "to": at + 1,
                "tracks": [{ "id": track_id, "albumId": album }],
            }),
        )
        .await
    }

    async fn saved_albums(&self) -> Result<Vec<Album>> {
        let liked = self
            .get(&format!(
                "/users/{}/likes/albums?rich=true",
                escape::component(self.uid())
            ))
            .await
            .context("cannot load the liked albums")?;
        Ok(liked
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(wire::liked_album)
            .collect())
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()> {
        self.like("album", album_id, saved).await
    }

    async fn saved_artists(&self) -> Result<Vec<SavedArtist>> {
        let liked = self
            .get(&format!(
                "/users/{}/likes/artists?with-timestamps=true",
                escape::component(self.uid())
            ))
            .await
            .context("cannot load the liked artists")?;
        Ok(liked
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(wire::saved_artist)
            .collect())
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()> {
        self.like("artist", artist_id, saved).await
    }

    async fn album(&self, album_id: &str) -> Result<AlbumDetail> {
        let value = self
            .get(&format!(
                "/albums/{}/with-tracks",
                escape::component(album_id)
            ))
            .await
            .with_context(|| format!("cannot load the album {album_id}"))?;
        let album =
            wire::album(&value).with_context(|| format!("cannot read the album {album_id}"))?;
        Ok(AlbumDetail {
            album,
            tracks: wire::volumes(&value),
        })
    }

    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>> {
        Ok(self.album(album_id).await?.tracks)
    }

    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail> {
        let value = self.playlist_value(playlist_id).await?;
        let playlist = wire::playlist(&value, self.uid())
            .with_context(|| format!("cannot read the playlist {playlist_id}"))?;
        let tracks = self.playlist_entries(&value).await?;
        Ok(PlaylistDetail {
            playlist,
            tracks,
            continuation: None,
        })
    }

    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let value = self.playlist_value(playlist_id).await?;
        self.playlist_entries(&value).await
    }

    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>> {
        let tracks = self.playlist_tracks(playlist_id).await?;
        Ok(distinct_covers(&tracks, wanted))
    }

    /// The track's station, a stretch at a time, each continuing after the last track of the
    /// one before. Where the station will not answer, the similar tracks stand in for it.
    async fn track_radio(
        &self,
        track_id: &str,
        from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)> {
        match self.station(track_id, from).await {
            Ok(tracks) if !tracks.is_empty() => {
                let next = tracks.last().and_then(|track| track.id.clone());
                Ok((tracks, next))
            }
            Ok(_) if from.is_some() => Ok((Vec::new(), None)),
            result => {
                if let Err(error) = result {
                    log::warn!("yandex: the station of {track_id} did not answer: {error:#}");
                }
                let similar = self
                    .similar(track_id)
                    .await
                    .context("cannot load the similar tracks")?;
                Ok((similar, None))
            }
        }
    }

    async fn search(&self, query: &str) -> Result<Vec<Track>> {
        let found = self.search_kind(query, "track").await?;
        Ok(found
            .as_array()
            .map(|list| wire::tracks(list))
            .unwrap_or_default())
    }

    async fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let found = self.search_kind(query, "album").await?;
        Ok(found
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(wire::album)
            .collect())
    }

    async fn search_playlists(&self, query: &str) -> Result<Vec<Playlist>> {
        let found = self.search_kind(query, "playlist").await?;
        Ok(found
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|value| wire::playlist(value, self.uid()))
            .collect())
    }
}
