use std::collections::HashMap;

use gpui::{Context, Entity, Task};
use music::Track;

use crate::{Abort, Io, Network, Session, Shelf, join};

const BATCH: usize = 500;

/// Loads metadata in batches. Retain the task; dropping it cancels the current request.
pub(crate) fn load<E: 'static>(
    session: &Entity<Session>,
    tracks: &[Track],
    rows: impl Fn(&mut E) -> Option<&mut [Track]> + 'static,
    cx: &mut Context<E>,
) -> Option<Task<()>> {
    let shelf = Shelf::of(tracks.iter().find_map(|track| track.id.as_deref())?);
    let session = session.read(cx);
    if !session.capabilities_of(shelf).track_metadata || Network::lost(cx) {
        return None;
    }
    let client = session.client_of(shelf)?;
    let mut ids: Vec<_> = tracks.iter().filter_map(|track| track.id.clone()).collect();
    ids.sort_unstable();
    ids.dedup();
    let io = Io::global(cx);
    Some(cx.spawn(async move |this, cx| {
        for ids in ids.chunks(BATCH) {
            let client = client.clone();
            let ids = ids.to_vec();
            let request = io.spawn(async move { client.track_metadata(&ids).await });
            let _abort = Abort(request.abort_handle());
            let updates = match join(request).await {
                Ok(updates) => updates,
                Err(error) => {
                    log::warn!("metadata: cannot load track metadata: {error:#}");
                    return;
                }
            };
            if this
                .update(cx, |this, cx| {
                    if let Some(tracks) = rows(this) {
                        apply(tracks, &updates);
                        cx.notify();
                    }
                })
                .is_err()
            {
                return;
            }
        }
    }))
}

/// Updates matching rows without changing their order. Empty fields leave existing values intact.
fn apply(tracks: &mut [Track], updates: &[Track]) {
    let updates: HashMap<_, _> = updates
        .iter()
        .filter_map(|track| Some((track.id.as_deref()?, track)))
        .collect();
    for track in tracks {
        let Some(update) = track.id.as_deref().and_then(|id| updates.get(id)) else {
            continue;
        };
        for (field, value) in [
            (&mut track.genres, &update.genres),
            (&mut track.tags, &update.tags),
            (&mut track.moods, &update.moods),
        ] {
            if !value.is_empty() {
                field.clone_from(value);
            }
        }
        track.bpm = update.bpm.or(track.bpm);
    }
}
