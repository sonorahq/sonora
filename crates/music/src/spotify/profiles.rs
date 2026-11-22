use std::collections::{HashMap, HashSet};

use crate::spotify::session::Session;
use anyhow::{Context as _, Result};

use crate::spotify::proto::metadata::{ExtensionKind, IdentityProfile};
use crate::spotify::proto::profile::{Profile, PublicPlaylist};
use crate::spotify::{collection, transport, wire};
use crate::{Contributor, Playlist, UserDetail, UserProfile, escape};

const USER_PREFIX: &str = "spotify:user:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const PLAYLISTS: u32 = 50;
const PROFILE_BATCH: usize = 100;

pub fn username(uri: &str) -> String {
    uri.strip_prefix(USER_PREFIX).unwrap_or(uri).to_owned()
}

pub async fn profile(session: &Session, user_id: &str) -> Result<UserDetail> {
    let id = username(user_id);
    let path = format!(
        "/user-profile-view/v3/profile/{}?playlist_limit={PLAYLISTS}&artist_limit=0",
        escape::component(&id)
    );
    let headers = transport::proto_headers(transport::PROTOBUF, None);
    let found: Profile = transport::get(session, &path, headers)
        .await
        .with_context(|| format!("cannot read the profile of {id}"))?;
    let playlists = found.public_playlists.iter().filter_map(listed).collect();

    Ok(UserDetail {
        name: if found.name.is_empty() {
            id.clone()
        } else {
            found.name
        },
        avatar: (!found.image_url.is_empty()).then_some(found.image_url),
        followers: found.followers_count,
        following: found.following_count,
        playlists,
        id,
    })
}

/// The signed-in account, named and pictured from its own profile. Signing in calls this, so a
/// profile Spotify will not hand over falls back to the bare username rather than failing.
pub async fn own(session: &Session) -> UserProfile {
    let id = session.username();
    let found = contributors(session, HashSet::from([id.to_owned()]))
        .await
        .remove(id);
    if let Some(profile) = &found {
        session.remember_display_name(&profile.name);
    }
    let (display_name, avatar) = found
        .map(|profile| (profile.name, profile.avatar))
        .unwrap_or_else(|| (id.to_owned(), None));
    UserProfile {
        display_name,
        avatar,
        id: id.to_owned(),
    }
}

/// Loads compact names and avatars in batches using the shared metadata cache.
pub async fn contributors(session: &Session, ids: HashSet<String>) -> HashMap<String, Contributor> {
    let uris: Vec<_> = ids
        .into_iter()
        .filter(|id| !id.is_empty())
        .map(|id| format!("{USER_PREFIX}{id}"))
        .collect();
    let mut contributors = HashMap::new();
    for batch in uris.chunks(PROFILE_BATCH) {
        match collection::project(
            session,
            batch,
            ExtensionKind::UserProfile,
            PROFILE_BATCH,
            |uri, profile| Some(contributor(uri, profile)),
        )
        .await
        {
            Ok(profiles) => contributors.extend(
                profiles
                    .into_values()
                    .map(|profile| (profile.id.clone(), profile)),
            ),
            Err(error) => log::debug!("profiles: cannot read contributor batch: {error:#}"),
        }
    }
    contributors
}

/// Projects the compact profile shared by playlist owners and contributors.
pub(super) fn contributor(uri: &str, profile: IdentityProfile) -> Contributor {
    let id = username(uri);
    let name = profile
        .name
        .as_ref()
        .map(|name| name.value.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(&id)
        .to_owned();
    let avatar = profile
        .images
        .iter()
        .filter_map(|image| Some((image.width, wire::playlist_image(&image.url)?)))
        .max_by_key(|(width, _)| *width)
        .map(|(_, url)| url);
    Contributor { id, name, avatar }
}

/// Resolves the displayed owner from the same profiles used for contributor credits.
pub(super) async fn playlist_owners(session: &Session, playlists: &mut [Playlist]) {
    let own_name = session.display_name();
    let ids = playlists
        .iter()
        .filter(|playlist| own_name.is_none() || playlist.owner_id != session.username())
        .map(|playlist| playlist.owner_id.clone())
        .collect();
    let known = contributors(session, ids).await;
    for playlist in playlists {
        if let Some(name) = own_name.filter(|_| playlist.owner_id == session.username()) {
            name.clone_into(&mut playlist.owner);
        } else if let Some(owner) = known.get(&playlist.owner_id) {
            playlist.owner.clone_from(&owner.name);
        }
    }
}

fn listed(found: &PublicPlaylist) -> Option<Playlist> {
    let id = found
        .uri
        .strip_prefix(PLAYLIST_PREFIX)
        .filter(|id| !id.is_empty() && !found.name.is_empty())?;
    let owner_id = username(&found.owner_uri);

    Some(Playlist {
        id: id.to_owned(),
        name: found.name.clone(),
        owner: if found.owner_name.is_empty() {
            owner_id.clone()
        } else {
            found.owner_name.clone()
        },
        owner_id,
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover: wire::fetchable(&found.image_url).then(|| found.image_url.clone()),
        track_count: 0,
        modified_at: None,
        read_only: false,
    })
}
