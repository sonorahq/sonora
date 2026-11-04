use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};
use librespot_core::Session;
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::spotify::wire;
use crate::{Contributor, Playlist, UserDetail, UserProfile, escape};

const USER_PREFIX: &str = "spotify:user:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const PLAYLISTS: u32 = 50;

#[derive(Debug, Default, Deserialize)]
struct Profile {
    display_name: Option<String>,
    name: Option<String>,
    image_url: Option<String>,
    followers_count: Option<u64>,
    following_count: Option<u64>,
    #[serde(default)]
    public_playlists: Vec<Listed>,
}

#[derive(Debug, Default, Deserialize)]
struct Listed {
    uri: Option<String>,
    name: Option<String>,
    image_url: Option<String>,
    owner_name: Option<String>,
    owner_uri: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Circle {
    #[serde(default)]
    profiles: Vec<Profile>,
}

impl Profile {
    fn label(&self) -> Option<&str> {
        self.display_name
            .as_deref()
            .or(self.name.as_deref())
            .filter(|label| !label.is_empty())
    }

    fn avatar(&self) -> Option<String> {
        self.image_url
            .as_deref()
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
    }
}

pub fn username(uri: &str) -> String {
    uri.strip_prefix(USER_PREFIX).unwrap_or(uri).to_owned()
}

pub async fn profile(session: &Session, user_id: &str) -> Result<UserDetail> {
    let id = username(user_id);
    let found = fetch(session, &id, PLAYLISTS)
        .await
        .with_context(|| format!("cannot read the profile of {id}"))?;

    let (followers, following) = counts(session, &id, &found).await;
    let playlists = found.public_playlists.iter().filter_map(listed).collect();

    Ok(UserDetail {
        name: found.label().unwrap_or(&id).to_owned(),
        avatar: found.avatar(),
        followers,
        following,
        playlists,
        id,
    })
}

/// The signed-in account, named and pictured from its own profile. Signing in calls this, so a
/// profile Spotify will not hand over falls back to the bare username rather than failing.
pub async fn own(session: &Session) -> UserProfile {
    let id = session.username();
    let found = fetch(session, &id, 0).await.unwrap_or_default();
    UserProfile {
        display_name: found.label().unwrap_or(&id).to_owned(),
        avatar: found.avatar(),
        id,
    }
}

pub async fn contributors(session: &Session, ids: HashSet<String>) -> HashMap<String, Contributor> {
    let mut pending = JoinSet::new();
    for id in ids {
        let session = session.clone();
        pending.spawn(async move {
            let found = fetch(&session, &id, 0).await;
            (id, found)
        });
    }

    let mut known = HashMap::new();
    while let Some(joined) = pending.join_next().await {
        let Ok((id, Some(found))) = joined else {
            continue;
        };
        known.insert(
            id.clone(),
            Contributor {
                name: found.label().unwrap_or(&id).to_owned(),
                avatar: found.avatar(),
                id,
            },
        );
    }
    known
}

pub async fn display_names(
    session: &Session,
    usernames: HashSet<String>,
) -> HashMap<String, String> {
    contributors(session, usernames)
        .await
        .into_iter()
        .map(|(id, found)| (id, found.name))
        .collect()
}

async fn counts(session: &Session, username: &str, found: &Profile) -> (Option<u64>, Option<u64>) {
    let followers = async {
        match found.followers_count {
            Some(count) => Some(count),
            None => circle(session, username, true).await,
        }
    };
    let following = async {
        match found.following_count {
            Some(count) => Some(count),
            None => circle(session, username, false).await,
        }
    };

    tokio::join!(followers, following)
}

async fn circle(session: &Session, username: &str, followers: bool) -> Option<u64> {
    let client = session.spclient();
    let body = match followers {
        true => {
            client
                .get_user_followers(&escape::component(username))
                .await
        }
        false => {
            client
                .get_user_following(&escape::component(username))
                .await
        }
    }
    .inspect_err(|error| log::debug!("profiles: cannot count the circle of {username}: {error}"))
    .ok()?;

    let circle = serde_json::from_slice::<Circle>(&body)
        .inspect_err(|error| log::debug!("profiles: cannot decode the circle: {error}"))
        .ok()?;

    Some(circle.profiles.len() as u64)
}

async fn fetch(session: &Session, username: &str, playlists: u32) -> Option<Profile> {
    let body = session
        .spclient()
        .get_user_profile(&escape::component(username), Some(playlists), Some(0))
        .await
        .inspect_err(|error| log::debug!("profiles: cannot resolve {username}: {error}"))
        .ok()?;

    serde_json::from_slice::<Profile>(&body)
        .inspect_err(|error| log::debug!("profiles: cannot decode {username}: {error}"))
        .ok()
}

fn listed(found: &Listed) -> Option<Playlist> {
    let id = found
        .uri
        .as_deref()?
        .strip_prefix(PLAYLIST_PREFIX)
        .filter(|id| !id.is_empty())?;
    let name = found.name.as_deref().filter(|name| !name.is_empty())?;

    Some(Playlist {
        id: id.to_owned(),
        name: name.to_owned(),
        owner_id: found.owner_uri.as_deref().map(username).unwrap_or_default(),
        owner: found
            .owner_name
            .clone()
            .or_else(|| found.owner_uri.as_deref().map(username))
            .unwrap_or_default(),
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover: found
            .image_url
            .as_deref()
            .filter(|url| wire::fetchable(url))
            .map(str::to_owned),
        track_count: 0,
        duration: None,
        modified_at: None,
    })
}
