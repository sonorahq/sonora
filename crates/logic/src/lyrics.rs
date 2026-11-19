//! Asking every lyrics source at once and putting what comes back in order.

use std::sync::Arc;

use music::{LyricsHit, LyricsProvider, LyricsQuery};
use tokio::task::JoinSet;

/// Hits sent to a lookup, with the provider that has now answered. Hits held from an earlier
/// lookup come with no provider.
pub type Reply = (Option<&'static str>, Vec<LyricsHit>);

/// Where `source` stands in the user's priority. Sources missing from it come last.
pub fn rank_in(priority: &[String], source: &str) -> usize {
    priority
        .iter()
        .position(|name| name == source)
        .unwrap_or(priority.len())
}

/// Ranks `hits` for `query` by the listener's source order and puts each sheet in the shape it
/// is shown in.
pub fn ordered(
    query: &LyricsQuery,
    hits: Vec<LyricsHit>,
    priority: &[String],
    worded_first: bool,
) -> Vec<LyricsHit> {
    let mut ranked = music::lyrics::rank_with_priority(query, hits, priority, worded_first);
    music::lyrics::reshape(&mut ranked);
    ranked
}

/// Whether a source still to answer could take the place of `shown`, the hit on display. The
/// best kind of lyrics can only lose to a source ranked higher, anything else to whichever
/// source answers next.
pub fn awaits_better(
    query: &LyricsQuery,
    shown: &LyricsHit,
    pending: &[&str],
    priority: &[String],
    worded_first: bool,
) -> bool {
    let place = match music::lyrics::best_kind(query, &shown.lyrics, worded_first) {
        true => rank_in(priority, shown.source),
        false => usize::MAX,
    };
    pending.iter().any(|name| rank_in(priority, name) < place)
}

/// Asks every provider at once and sends each one's name and hits for the queried track as
/// they arrive. A provider that fails sends its name with no hits, so the receiver knows it is
/// done. Returns the providers that replied, leaving out any that failed.
pub async fn gather(
    providers: Vec<Arc<dyn LyricsProvider>>,
    query: LyricsQuery,
    sender: tokio::sync::mpsc::UnboundedSender<Reply>,
) -> anyhow::Result<Vec<&'static str>> {
    let mut tasks = JoinSet::new();
    for provider in providers {
        let query = query.clone();
        tasks.spawn(async move {
            let name = provider.name();
            let found = provider
                .search(&query)
                .await
                .inspect_err(|error| log::warn!("lyrics: {name} did not answer: {error:#}"))
                .ok()
                .map(|mut found| {
                    // Hits for another track are dropped here, before the lookup holds and
                    // copies them.
                    found.retain(|hit| music::lyrics::matched(&query, hit));
                    log::debug!(
                        "lyrics: {name} returned {} matching answer(s) for {}",
                        found.len(),
                        query.title
                    );
                    found
                });
            (name, found)
        });
    }
    let mut answered = Vec::new();
    while !tasks.is_empty() {
        tokio::select! {
            // Dropping the foreground lookup (including on a provider change) must also
            // stop its network requests. Dropping this JoinSet aborts the remaining tasks.
            _ = sender.closed() => break,
            found = tasks.join_next() => {
                if let Some(Ok((name, found))) = found {
                    if found.is_some() {
                        answered.push(name);
                    }
                    sender.send((Some(name), found.unwrap_or_default())).ok();
                }
            }
        }
    }
    Ok(answered)
}
