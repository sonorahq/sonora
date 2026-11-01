use super::{
    PLAYLIST_PREFIX, TRACK_PREFIX, add_items, changes, edit, fetch_rootlist, item, remove_items,
    rootlist_edit, snapshot,
};
use anyhow::{Context as _, Result, ensure};
use futures::{StreamExt as _, TryStreamExt as _, stream};
use librespot_core::{Session, SpotifyId};
use librespot_protocol::playlist4_external::{Item, SelectedListContent};
use std::collections::{HashMap, HashSet};

const FOLDER: &str = "Rated";
const START: &str = "spotify:start-group:";
const END: &str = "spotify:end-group";
/// The maximum number of rating playlists fetched at once.
const READS: usize = 4;

/// A playlist in the Rated folder, with its contents.
struct Rated {
    id: String,
    rating: u8,
    content: SelectedListContent,
}

/// The rootlist, plus the Rated folder's end index and playlists when it exists.
type Account = (SelectedListContent, Option<(usize, Vec<Rated>)>);

/// The Rated folder's end index, and its playlists as (id, half-stars).
type Folder = (usize, Vec<(String, u8)>);

/// Reads every rating. A track in several rating playlists takes the most recently added one.
pub(in crate::spotify) async fn load(
    session: &Session,
) -> Result<(HashMap<String, u8>, HashSet<String>)> {
    let (_, Some((_, rated))) = read(session).await? else {
        return Ok((HashMap::new(), HashSet::new()));
    };
    let playlists = rated.iter().map(|playlist| playlist.id.clone()).collect();
    let mut entries = HashMap::new();
    for playlist in rated {
        for item in &playlist.content.contents.items {
            if let Some(id) = item.uri().strip_prefix(TRACK_PREFIX) {
                let entry = entries.entry(id.to_owned()).or_insert((i64::MIN, 0));
                if item.attributes.timestamp() >= entry.0 {
                    *entry = (item.attributes.timestamp(), playlist.rating);
                }
            }
        }
    }
    Ok((
        entries
            .into_iter()
            .map(|(id, (_, rating))| (id, rating))
            .collect(),
        playlists,
    ))
}

/// Sets or clears a rating. The new entry is added before old ones are removed, so a failure
/// never loses the rating.
pub(in crate::spotify) async fn set(
    session: &Session,
    track_id: &str,
    rating: Option<u8>,
) -> Result<()> {
    ensure!(
        rating.is_none_or(|value| (1..=10).contains(&value)),
        "invalid half-star rating"
    );
    SpotifyId::from_base62(track_id).context("cannot read the rated track id")?;
    let (mut root, mut folder) = read(session).await?;
    if folder.is_none() && rating.is_some() {
        let group = format!("{:016x}", fastrand::u64(..));
        let op = add_items(vec![
            item(&format!("{START}{group}:{FOLDER}")),
            item(&format!("{END}:{group}")),
        ]);
        rootlist_edit(session, &changes(root.revision.as_deref(), op)).await?;
        (root, folder) = read(session).await?;
    }
    let Some((end, rated)) = folder else {
        return Ok(());
    };

    let uri = format!("{TRACK_PREFIX}{track_id}");
    let target = rated
        .iter()
        .find(|playlist| Some(playlist.rating) == rating);
    if let Some(rating) = rating
        && target.is_none_or(|playlist| matching(playlist, &uri).next().is_none())
    {
        let (id, revision) = match target {
            Some(playlist) => (playlist.id.clone(), playlist.content.revision.clone()),
            None => {
                let id = super::create_unlisted(session, &format!("{:.1}", f64::from(rating) / 2.))
                    .await?;
                let mut entry = item(&format!("{PLAYLIST_PREFIX}{id}"));
                entry.attributes.mut_or_insert_default().set_public(false);
                let mut op = add_items(vec![entry]);
                let add = op.add.mut_or_insert_default();
                add.add_last = None;
                add.set_from_index(end as i32);
                rootlist_edit(session, &changes(root.revision.as_deref(), op)).await?;
                (id, None)
            }
        };
        let mut entry = item(&uri);
        let now = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
        entry.attributes.mut_or_insert_default().set_timestamp(now);
        edit(
            session,
            &id,
            &changes(revision.as_deref(), add_items(vec![entry])),
        )
        .await?;
    }
    for playlist in &rated {
        if target.is_some_and(|target| target.id == playlist.id) {
            continue;
        }
        let items: Vec<Item> = matching(playlist, &uri).cloned().collect();
        if items.is_empty() {
            continue;
        }
        let revision = playlist.content.revision.as_deref();
        edit(
            session,
            &playlist.id,
            &changes(revision, remove_items(items)),
        )
        .await?;
    }
    Ok(())
}

/// Reads the rootlist and the Rated folder's playlist contents with bounded concurrency.
/// Keeps folder order so equal timestamps resolve consistently when loading ratings.
async fn read(session: &Session) -> Result<Account> {
    let root = whole(fetch_rootlist(session).await?)?;
    let Some((end, playlists)) = folder(&root)? else {
        return Ok((root, None));
    };
    let rated = stream::iter(playlists)
        .map(|(id, rating)| async move {
            let content = whole(snapshot(session, &id).await?)?;
            Ok::<_, anyhow::Error>(Rated {
                id,
                rating,
                content,
            })
        })
        .buffered(READS)
        .try_collect()
        .await?;
    Ok((root, Some((end, rated))))
}

/// Rejects a truncated listing.
fn whole(content: SelectedListContent) -> Result<SelectedListContent> {
    ensure!(
        !content.contents.truncated(),
        "cannot read every rating at once"
    );
    Ok(content)
}

/// Finds the top-level Rated folder and the rating playlists directly inside it.
fn folder(root: &SelectedListContent) -> Result<Option<Folder>> {
    let contents = &root.contents;
    let mut depth = 0usize;
    let mut rated = None;
    let mut found = None;
    let mut playlists = Vec::new();
    for (index, item) in contents.items.iter().enumerate() {
        let uri = item.uri();
        let name = contents
            .meta_items
            .get(index)
            .map(|meta| meta.attributes.name());
        if let Some(group) = uri.strip_prefix(START) {
            let label = group.split_once(':').map(|(_, label)| label);
            let label = percent_encoding::percent_decode_str(label.unwrap_or_default());
            if depth == 0 && (label.decode_utf8_lossy() == FOLDER || name == Some(FOLDER)) {
                ensure!(found.is_none(), "more than one Rated folder exists");
                rated = Some(1);
            }
            depth += 1;
        } else if uri.starts_with(END) {
            if rated == Some(depth) {
                found = Some((index, std::mem::take(&mut playlists)));
                rated = None;
            }
            depth = depth.saturating_sub(1);
        } else if rated == Some(depth)
            && let Some(id) = uri.strip_prefix(PLAYLIST_PREFIX)
        {
            let name = name.context("cannot read rating playlist names")?;
            if let Some(rating) = parse_name(name) {
                playlists.push((id.to_owned(), rating));
            }
        }
    }
    ensure!(rated.is_none(), "cannot read the complete Rated folder");
    Ok(found)
}

/// Parses a rating playlist name such as `4` or `4.5` into half-stars.
fn parse_name(name: &str) -> Option<u8> {
    let rating = name.parse::<f64>().ok()? * 2.;
    ((1. ..=10.).contains(&rating) && rating.fract() == 0.).then_some(rating as u8)
}

/// Matches entries without allocating; only removals need owned copies.
fn matching<'a>(playlist: &'a Rated, uri: &'a str) -> impl Iterator<Item = &'a Item> {
    playlist
        .content
        .contents
        .items
        .iter()
        .filter(move |item| item.uri() == uri)
}
