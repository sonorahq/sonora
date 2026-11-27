//! The listens a scrobbling service has not taken yet. They wait in `state.sqlite`, so a listen
//! made offline, while a service is down or just before a quit still reaches it later.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::{Connection, params};
use scrobble::{Account, Play, Rejected, Service};
use storage::Database;
use tokio::sync::mpsc;

use super::{Verdict, judge};
use crate::Io;

/// The most listens kept for one service. Past it the oldest go first, so an account that stays
/// unreachable for months cannot grow the database without end.
const LIMIT: i64 = 5000;
/// How long the sender waits after a failed attempt before it tries again. Each failure in a row
/// doubles the wait, up to `LONGEST_WAIT`.
const FIRST_WAIT: Duration = Duration::from_secs(60);
const LONGEST_WAIT: Duration = Duration::from_secs(30 * 60);

/// What the sender of one service is asked to do.
pub(super) enum Submit {
    /// Keep this listen, then send everything waiting to this account.
    Play(Account, Play),
    /// Send everything waiting to this account.
    Account(Account),
    /// Stop sending, since the service was switched off. Its listens keep waiting.
    Hold,
    /// Drop every waiting listen and stop sending, since the account was unlinked.
    Forget,
}

/// The backlog table in `state.sqlite`, one row per waiting listen.
#[derive(Clone)]
pub(super) struct Backlog {
    database: Database,
}

impl Backlog {
    pub(super) fn new(database: Database) -> Self {
        Self { database }
    }

    fn open(&self) -> Result<Connection> {
        self.database
            .open()
            .context("cannot open the scrobble backlog")
    }

    /// Keeps a listen at the end of a service's backlog and drops the oldest past [`LIMIT`].
    fn push(&self, service: &str, play: &Play) -> Result<()> {
        let play = serde_json::to_string(play).context("cannot encode a listen")?;
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO scrobble_backlog (service, play) VALUES (?, ?)",
                params![service, play],
            )
            .context("cannot keep a listen")?;
        connection
            .execute(
                "DELETE FROM scrobble_backlog WHERE service = ?1 AND id NOT IN (
                    SELECT id FROM scrobble_backlog WHERE service = ?1 ORDER BY id DESC LIMIT ?2
                )",
                params![service, LIMIT],
            )
            .context("cannot trim the scrobble backlog")?;
        Ok(())
    }

    /// The oldest listens waiting for a service, at most `count`, with their row ids. A row that
    /// no longer decodes is deleted on the way, since no later attempt could send it either.
    fn oldest(&self, service: &str, count: usize) -> Result<Vec<(i64, Play)>> {
        let connection = self.open()?;
        let limit = i64::try_from(count).unwrap_or(i64::MAX);
        loop {
            let mut query = connection
                .prepare(
                    "SELECT id, play FROM scrobble_backlog WHERE service = ? ORDER BY id LIMIT ?",
                )
                .context("cannot prepare a scrobble backlog read")?;
            let rows = query
                .query_map(params![service, limit], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("cannot read the scrobble backlog")?;

            let mut plays = Vec::new();
            let mut broken = Vec::new();
            for row in rows {
                let (id, play) = row.context("cannot read a waiting listen")?;
                match serde_json::from_str(&play) {
                    Ok(play) => plays.push((id, play)),
                    Err(error) => {
                        log::warn!(
                            "scrobble: dropping a waiting listen that no longer reads: {error}"
                        );
                        broken.push(id);
                    }
                }
            }
            remove(&connection, &broken)?;
            if broken.is_empty() || !plays.is_empty() {
                return Ok(plays);
            }
        }
    }

    /// Deletes listens the service has taken or rejected for good.
    fn remove(&self, ids: &[i64]) -> Result<()> {
        remove(&self.open()?, ids)
    }

    /// Deletes every listen waiting for a service.
    fn forget(&self, service: &str) -> Result<()> {
        self.open()?
            .execute(
                "DELETE FROM scrobble_backlog WHERE service = ?",
                params![service],
            )
            .context("cannot clear the scrobble backlog")?;
        Ok(())
    }
}

/// Starts the task that sends one service its listens. Every listen goes into the backlog first,
/// and the backlog goes out oldest first whenever a listen arrives, the account is handed over
/// again or the wait after a failure runs out. It ends when the row drops its sender.
pub(super) fn sender(
    io: &Io,
    service: Arc<dyn Service>,
    backlog: Backlog,
    verdicts: mpsc::UnboundedSender<Verdict>,
) -> mpsc::UnboundedSender<Submit> {
    let (sender, mut asked) = mpsc::unbounded_channel::<Submit>();
    io.spawn(async move {
        let id = service.id();
        let mut account: Option<Account> = None;
        let mut wait: Option<Duration> = None;
        loop {
            let next = match wait {
                Some(wait) => match tokio::time::timeout(wait, asked.recv()).await {
                    Ok(Some(next)) => Some(next),
                    Ok(None) => break,
                    Err(_) => None,
                },
                None => match asked.recv().await {
                    Some(next) => Some(next),
                    None => break,
                },
            };
            match next {
                Some(Submit::Play(given, play)) => {
                    let kept = backlog.clone();
                    let copy = play.clone();
                    let stored = blocking(move || kept.push(id, &copy)).await;
                    if let Err(error) = stored {
                        log::warn!(
                            "scrobble: cannot keep a listen for {id}, sending it once: {error:#}"
                        );
                        let answer = service.scrobble(&given, std::slice::from_ref(&play)).await;
                        if let Err(error) = &answer {
                            log::warn!("scrobble: cannot submit the track to {id}: {error:#}");
                        }
                        judge(&verdicts, id, &given, &answer);
                    }
                    account = Some(given);
                }
                Some(Submit::Account(given)) => account = Some(given),
                Some(Submit::Hold) => account = None,
                Some(Submit::Forget) => {
                    account = None;
                    let kept = backlog.clone();
                    if let Err(error) = blocking(move || kept.forget(id)).await {
                        log::warn!("scrobble: {error:#}");
                    }
                }
                None => {}
            }

            let Some(given) = &account else {
                wait = None;
                continue;
            };
            let drained = drain(service.as_ref(), given, &backlog).await;
            // an empty backlog asked nothing, so it says nothing about the key either
            let heard = !matches!(drained, Ok(false));
            let answer = drained.map(|_| ());
            wait = match &answer {
                Ok(()) => None,
                Err(error) => {
                    let next = wait.map_or(FIRST_WAIT, |wait| (wait * 2).min(LONGEST_WAIT));
                    log::warn!(
                        "scrobble: cannot submit to {id}, trying again in {}s: {error:#}",
                        next.as_secs()
                    );
                    Some(next)
                }
            };
            if heard {
                judge(&verdicts, id, given, &answer);
            }
        }
    });
    sender
}

/// Sends a service's backlog oldest first, in slices of its batch size, until it is empty or an
/// attempt fails, and says whether the service was asked anything. A slice the service rejects
/// for good is dropped. The failure that stops the drain is returned and its slice keeps waiting.
async fn drain(service: &dyn Service, account: &Account, backlog: &Backlog) -> Result<bool> {
    let id = service.id();
    let batch = service.batch().max(1);
    let mut asked = false;
    loop {
        let reader = backlog.clone();
        let waiting = blocking(move || reader.oldest(id, batch)).await?;
        if waiting.is_empty() {
            return Ok(asked);
        }
        asked = true;
        let (ids, plays): (Vec<i64>, Vec<Play>) = waiting.into_iter().unzip();
        match service.scrobble(account, &plays).await {
            Ok(()) => {}
            Err(error) if Rejected::in_error(&error) => log::warn!(
                "scrobble: {id} rejected {} listens, dropping them: {error:#}",
                plays.len()
            ),
            Err(error) => return Err(error),
        }
        let remover = backlog.clone();
        blocking(move || remover.remove(&ids)).await?;
    }
}

/// Runs a database call off the async workers.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .context("the scrobble backlog task failed")?
}

fn remove(connection: &Connection, ids: &[i64]) -> Result<()> {
    let mut statement = connection
        .prepare("DELETE FROM scrobble_backlog WHERE id = ?")
        .context("cannot prepare a scrobble backlog delete")?;
    for id in ids {
        statement
            .execute(params![id])
            .context("cannot delete a sent listen")?;
    }
    Ok(())
}
