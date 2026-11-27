//! The listens a scrobbling service has not taken yet. They wait in `state.sqlite`, so a listen
//! made offline, while a service is down or just before a quit still reaches it later.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// Send everything waiting to this account.
    Account(Account),
    /// Send everything waiting now, to the account last handed over. A kept listen asks this, so
    /// it can never hand back an account the row has since held or forgotten.
    Wake,
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

/// One service's end of the backlog: it keeps the service's listens and wakes the task that sends
/// them. Keeping a listen never waits on a send still in flight, so a stalled service cannot cost
/// a listen made meanwhile.
pub(super) struct Sender {
    io: Io,
    service: Arc<dyn Service>,
    backlog: Backlog,
    verdicts: mpsc::UnboundedSender<Verdict>,
    asked: mpsc::UnboundedSender<Submit>,
    /// Counts the requests other than a wake, so a drain in progress can tell one is waiting.
    turn: Arc<AtomicU64>,
}

impl Sender {
    /// Starts the task that sends the service's backlog oldest first whenever a listen is kept,
    /// the account is handed over again or the wait after a failure runs out. The task ends when
    /// the sender is dropped.
    pub(super) fn new(
        io: &Io,
        service: Arc<dyn Service>,
        backlog: Backlog,
        verdicts: mpsc::UnboundedSender<Verdict>,
    ) -> Self {
        let (asked, heard) = mpsc::unbounded_channel::<Submit>();
        let turn = Arc::new(AtomicU64::new(0));
        io.spawn(send(
            service.clone(),
            backlog.clone(),
            verdicts.clone(),
            heard,
            turn.clone(),
        ));
        Self {
            io: io.clone(),
            service,
            backlog,
            verdicts,
            asked,
            turn,
        }
    }

    /// Keeps a listen, then wakes the task to send everything waiting. `account` is used only when
    /// the listen cannot be kept and goes out once on its own.
    pub(super) fn play(&self, account: Account, play: Play) {
        let (service, backlog) = (self.service.clone(), self.backlog.clone());
        let (verdicts, asked) = (self.verdicts.clone(), self.asked.clone());
        self.io.spawn(async move {
            let id = service.id();
            let copy = play.clone();
            match blocking(move || backlog.push(id, &copy)).await {
                Ok(()) => {
                    asked.send(Submit::Wake).ok();
                }
                Err(error) => {
                    log::warn!(
                        "scrobble: cannot keep a listen for {id}, sending it once: {error:#}"
                    );
                    let answer = service
                        .scrobble(&account, std::slice::from_ref(&play))
                        .await;
                    if let Err(error) = &answer {
                        log::warn!("scrobble: cannot submit the track to {id}: {error:#}");
                    }
                    judge(&verdicts, id, &account, &answer);
                }
            }
        });
    }

    /// Passes a request to the task. Anything but a wake also stops a drain in progress before
    /// its next slice, so a hold or an unlink takes effect without waiting out the backlog.
    pub(super) fn ask(&self, submit: Submit) {
        if !matches!(submit, Submit::Wake) {
            self.turn.fetch_add(1, Ordering::SeqCst);
        }
        self.asked.send(submit).ok();
    }
}

/// The task behind a [`Sender`]. It drains the backlog after each request and, after a failure,
/// once the wait runs out. A new account starts the wait over.
async fn send(
    service: Arc<dyn Service>,
    backlog: Backlog,
    verdicts: mpsc::UnboundedSender<Verdict>,
    mut heard: mpsc::UnboundedReceiver<Submit>,
    turn: Arc<AtomicU64>,
) {
    let id = service.id();
    // how many of the requests counted in `turn` this task has taken
    let mut seen = 0;
    let mut account: Option<Account> = None;
    let mut wait: Option<Duration> = None;
    loop {
        let next = match wait {
            Some(wait) => match tokio::time::timeout(wait, heard.recv()).await {
                Ok(Some(next)) => Some(next),
                Ok(None) => break,
                Err(_) => None,
            },
            None => match heard.recv().await {
                Some(next) => Some(next),
                None => break,
            },
        };
        match next {
            Some(Submit::Account(given)) => {
                seen += 1;
                account = Some(given);
                wait = None;
            }
            Some(Submit::Wake) => wait = None,
            Some(Submit::Hold) => {
                seen += 1;
                account = None;
            }
            Some(Submit::Forget) => {
                seen += 1;
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
        let turned = || turn.load(Ordering::SeqCst) != seen;
        let drained = drain(service.as_ref(), given, &backlog, turned).await;
        // an empty backlog asked nothing, so it says nothing about the key either
        let heard_back = !matches!(drained, Ok(false));
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
        if heard_back {
            judge(&verdicts, id, given, &answer);
        }
    }
}

/// Sends a service's backlog oldest first, in slices of its batch size, until it is empty or an
/// attempt fails, and says whether the service was asked anything. A rejected slice of several
/// listens goes again one at a time, so only the listen the service refuses is dropped. The
/// failure that stops the drain is returned and its slice keeps waiting. It also stops before a
/// slice once `turned` says a new request waits.
async fn drain(
    service: &dyn Service,
    account: &Account,
    backlog: &Backlog,
    turned: impl Fn() -> bool,
) -> Result<bool> {
    let id = service.id();
    let batch = service.batch().max(1);
    let mut asked = false;
    // how many listens still go one at a time after a rejected slice
    let mut alone = 0;
    loop {
        if turned() {
            return Ok(asked);
        }
        let reader = backlog.clone();
        let size = match alone > 0 {
            true => 1,
            false => batch,
        };
        let waiting = blocking(move || reader.oldest(id, size)).await?;
        if waiting.is_empty() {
            return Ok(asked);
        }
        asked = true;
        let (ids, plays): (Vec<i64>, Vec<Play>) = waiting.into_iter().unzip();
        match service.scrobble(account, &plays).await {
            Ok(()) => {}
            Err(error) if Rejected::in_error(&error) && plays.len() > 1 => {
                log::warn!("scrobble: {id} rejected a batch, sending it one at a time: {error:#}");
                alone = plays.len();
                continue;
            }
            Err(error) if Rejected::in_error(&error) => {
                log::warn!("scrobble: {id} rejected a listen, dropping it: {error:#}")
            }
            Err(error) => return Err(error),
        }
        alone = alone.saturating_sub(plays.len());
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

/// Deletes listens by row id in one transaction, so a batch goes all at once.
fn remove(connection: &Connection, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let transaction = connection
        .unchecked_transaction()
        .context("cannot start a scrobble backlog delete")?;
    {
        let mut statement = transaction
            .prepare("DELETE FROM scrobble_backlog WHERE id = ?")
            .context("cannot prepare a scrobble backlog delete")?;
        for id in ids {
            statement
                .execute(params![id])
                .context("cannot delete a sent listen")?;
        }
    }
    transaction
        .commit()
        .context("cannot delete the sent listens")
}
