use std::collections::{HashMap, HashSet};

use crate::session::Session;
use anyhow::{Context as _, Result};

use crate::{auth::sign_in_error, collection, wire};
use music::{Contributor, Playlist, UserDetail, UserProfile};
use spotifyapis::proto::extended_metadata::{ExtensionKind, IdentityProfile};
use spotifyapis::proto::user_profile_view::PublicPlaylist;

const USER_PREFIX: &str = "spotify:user:";
const PLAYLIST_PREFIX: &str = "spotify:playlist:";
const PLAYLISTS: u32 = 50;
const PROFILE_BATCH: usize = 100;

pub(super) fn username(uri: &str) -> &str {
    uri.strip_prefix(USER_PREFIX).unwrap_or(uri)
}

pub async fn profile(session: &Session, user_id: &str) -> Result<UserDetail> {
    let id = username(user_id);
    let found = session
        .api()
        .profile(id, PLAYLISTS)
        .await
        .map_err(sign_in_error)
        .with_context(|| format!("cannot read the profile of {id}"))?;
    let playlists = found
        .public_playlists
        .into_iter()
        .filter_map(listed)
        .collect();

    Ok(UserDetail {
        name: if found.name.is_empty() {
            id.to_owned()
        } else {
            found.name
        },
        avatar: (!found.image_url.is_empty()).then_some(found.image_url),
        followers: found.followers_count,
        following: found.following_count,
        playlists,
        id: id.to_owned(),
    })
}

/// Uses the username when the signed-in profile is unavailable.
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

/// Loads compact names and avatars in metadata batches.
pub async fn contributors(session: &Session, ids: HashSet<String>) -> HashMap<String, Contributor> {
    let uris: Vec<_> = ids
        .into_iter()
        .filter(|id| !id.is_empty())
        .map(|id| format!("{USER_PREFIX}{id}"))
        .collect();
    let kind = ExtensionKind::UserProfile;
    collection::project_batch(session, &uris, &[kind], PROFILE_BATCH, true, |data, uri| {
        Some(contributor(uri, collection::take(data, kind, uri)?))
    })
    .await
    .unwrap_or_default()
    .into_values()
    .map(|profile| (profile.id.clone(), profile))
    .collect()
}

/// Projects the compact profile shared by playlist owners and contributors.
pub(super) fn contributor(uri: &str, profile: IdentityProfile) -> Contributor {
    let id = username(uri);
    let name = profile
        .name
        .as_ref()
        .map(|name| name.value.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(id)
        .to_owned();
    let avatar = profile
        .images
        .iter()
        .filter_map(|image| Some((image.width, wire::playlist_image(&image.url)?)))
        .max_by_key(|(width, _)| *width)
        .map(|(_, url)| url);
    Contributor {
        id: id.to_owned(),
        name,
        avatar,
    }
}

/// Resolves the displayed owner from the same profiles used for contributor credits.
pub(super) async fn playlist_owners<'a>(
    session: &Session,
    playlists: impl IntoIterator<Item = &'a mut Playlist>,
) {
    let playlists: Vec<_> = playlists.into_iter().collect();
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
        } else if playlist.owner.is_empty() {
            playlist.owner.clone_from(&playlist.owner_id);
        }
    }
}

fn listed(found: PublicPlaylist) -> Option<Playlist> {
    let id = found
        .uri
        .strip_prefix(PLAYLIST_PREFIX)
        .filter(|id| !id.is_empty() && !found.name.is_empty())?;
    let owner_id = username(&found.owner_uri);

    Some(Playlist {
        revision: None,
        id: id.to_owned(),
        name: found.name,
        owner: if found.owner_name.is_empty() {
            owner_id.to_owned()
        } else {
            found.owner_name
        },
        owner_id: owner_id.to_owned(),
        owned: false,
        collaborative: false,
        blend: false,
        public: true,
        cover: wire::fetchable(&found.image_url).then_some(found.image_url),
        track_count: 0,
        modified_at: None,
        read_only: false,
    })
}
