use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::params;
use storage::Database;

use crate::{Change, Cx, Event, Job, SessionEvent, Store, join, version};

const ENDPOINT: &str = "https://sonora-stats.nolight.dev/install";
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/sonorahq/sonora)"
);
const REPORTED: &str = "reported_usage";
const LOGGED_IN: &str = "logged_in_once";
const PATIENCE: Duration = Duration::from_secs(10);

/// The flags table usage reads and writes, behind a handle cheap to send to a blocking thread.
#[derive(Clone)]
struct Flags {
    database: Database,
}

impl Flags {
    fn new(database: Database) -> Self {
        Self { database }
    }

    fn open(&self) -> Result<rusqlite::Connection> {
        self.database.open().context("cannot open flags")
    }

    fn read(&self) -> Result<(bool, bool)> {
        let connection = self.open()?;
        let mut query = connection
            .prepare("SELECT key, value FROM flags WHERE key IN (?, ?)")
            .context("cannot prepare a flag read")?;
        let rows = query
            .query_map(params![REPORTED, LOGGED_IN], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .context("cannot read flags")?;

        let mut settled = false;
        let mut logged_in = false;
        for row in rows {
            let (key, value) = row.context("cannot decode a flag")?;
            match key.as_str() {
                REPORTED => settled = true,
                LOGGED_IN => logged_in = value != 0,
                _ => {}
            }
        }
        Ok((settled, logged_in))
    }

    fn write(&self, key: &str, value: bool) -> Result<()> {
        self.open()?
            .execute(
                "INSERT INTO flags (key, value) VALUES (?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, i64::from(value)],
            )
            .context("cannot save a flag")?;
        Ok(())
    }
}

pub struct Usage {
    loaded: bool,
    settled: bool,
    logged_in: bool,
    consented: bool,
    flags: Flags,
    http: reqwest::Client,
    load: Option<Job>,
    task: Option<Job>,
    mark: Option<Job>,
}

impl Usage {
    pub fn new(database: Database, cx: &mut Cx) -> Self {
        let mut usage = Self {
            loaded: false,
            settled: false,
            logged_in: false,
            consented: true,
            flags: Flags::new(database),
            http: reqwest::Client::new(),
            load: None,
            task: None,
            mark: None,
        };
        usage.recall(cx);
        usage
    }

    pub fn asking(&self) -> bool {
        self.loaded && !self.settled && !self.logged_in
    }

    pub fn consented(&self) -> bool {
        self.consented
    }

    pub fn consent(&mut self, consented: bool) {
        self.consented = consented;
    }

    pub fn report(&mut self, cx: &mut Cx) {
        if !self.asking() {
            return;
        }
        let consented = self.consented;
        self.settled = true;

        let flags = self.flags.clone();
        let http = self.http.clone();
        self.task = Some(cx.spawn(|link| async move {
            let sent = async move {
                if consented {
                    send(&http).await?;
                }
                tokio::task::spawn_blocking(move || flags.write(REPORTED, consented)).await?
            }
            .await;
            if let Err(error) = sent {
                log::warn!("usage: cannot report usage: {error:#}");
            }
            link.post(|store| store.usage.task = None);
        }));
    }

    fn connected(&mut self, cx: &mut Cx) {
        if self.logged_in {
            return;
        }
        self.logged_in = true;

        let flags = self.flags.clone();
        self.mark = Some(cx.spawn(|link| async move {
            let saved = join(tokio::task::spawn_blocking(move || {
                flags.write(LOGGED_IN, true)
            }))
            .await;
            if let Err(error) = saved {
                log::warn!("usage: cannot remember the first sign in: {error:#}");
            }
            link.post(|store| store.usage.mark = None);
        }));
    }

    fn recall(&mut self, cx: &mut Cx) {
        let flags = self.flags.clone();
        self.load = Some(cx.spawn(|link| async move {
            let flags = join(tokio::task::spawn_blocking(move || flags.read())).await;
            link.post(move |store| {
                let this = &mut store.usage;
                this.load = None;
                match flags {
                    Ok((settled, logged_in)) => {
                        this.settled = settled;
                        this.logged_in = logged_in;
                    }
                    Err(error) => {
                        log::warn!("usage: cannot read flags: {error:#}");
                        this.settled = true;
                    }
                }
                this.loaded = true;
            });
        }));
    }
}

impl Store {
    pub(crate) fn react_usage(&mut self, change: &Change) {
        if let Change::Event(Event::Session(SessionEvent::SignedIn)) = change {
            self.usage.connected(&mut self.cx);
        }
    }
}

async fn send(http: &reqwest::Client) -> Result<()> {
    http.post(ENDPOINT)
        .header(reqwest::header::USER_AGENT, AGENT)
        .timeout(PATIENCE)
        .json(&serde_json::json!({ "version": version(), "os": std::env::consts::OS }))
        .send()
        .await
        .context("cannot reach the usage endpoint")?
        .error_for_status()
        .context("the usage endpoint refused the report")?;
    Ok(())
}
