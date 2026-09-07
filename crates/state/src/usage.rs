use std::time::Duration;

use anyhow::{Context as _, Result};
use gpui::{Context, Entity, Task};
use rusqlite::params;
use storage::Database;

use crate::{Io, Session, SessionEvent, join};

const ENDPOINT: &str = "https://sonora-stats.nolight.dev/install";
const RUNNING: &str = env!("CARGO_PKG_VERSION");
const AGENT: &str = concat!(
    "sonora/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/nolight132/sonora)"
);
const REPORTED: &str = "reported_usage";
const LOGGED_IN: &str = "logged_in_once";
const PATIENCE: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct Store {
    database: Database,
}

impl Store {
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
    store: Store,
    http: reqwest::Client,
    io: Io,
    load: Option<Task<()>>,
    task: Option<Task<()>>,
    mark: Option<Task<()>>,
}

impl Usage {
    pub fn new(
        session: Entity<Session>,
        database: Database,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => this.connected(cx),
            SessionEvent::Reconnected | SessionEvent::SignedOut | SessionEvent::LocalChanged => {}
        })
        .detach();

        let mut usage = Self {
            loaded: false,
            settled: false,
            logged_in: false,
            consented: true,
            store: Store::new(database),
            http: reqwest::Client::new(),
            io,
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

    pub fn consent(&mut self, consented: bool, cx: &mut Context<Self>) {
        self.consented = consented;
        cx.notify();
    }

    pub fn report(&mut self, cx: &mut Context<Self>) {
        if !self.asking() {
            return;
        }
        let consented = self.consented;
        self.settled = true;
        cx.notify();

        let store = self.store.clone();
        let http = self.http.clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let sent = join(io.spawn(async move {
                if consented {
                    send(&http).await?;
                }
                tokio::task::spawn_blocking(move || store.write(REPORTED, consented)).await?
            }))
            .await;
            if let Err(error) = sent {
                log::warn!("usage: cannot report usage: {error:#}");
            }
            this.update(cx, |this, _| this.task = None).ok();
        }));
    }

    fn connected(&mut self, cx: &mut Context<Self>) {
        if self.logged_in {
            return;
        }
        self.logged_in = true;
        cx.notify();

        let store = self.store.clone();
        let io = self.io.clone();
        self.mark = Some(cx.spawn(async move |this, cx| {
            let saved = join(io.spawn(async move {
                tokio::task::spawn_blocking(move || store.write(LOGGED_IN, true)).await?
            }))
            .await;
            if let Err(error) = saved {
                log::warn!("usage: cannot remember the first sign in: {error:#}");
            }
            this.update(cx, |this, _| this.mark = None).ok();
        }));
    }

    fn recall(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let io = self.io.clone();
        self.load = Some(cx.spawn(async move |this, cx| {
            let flags = join(
                io.spawn(async move { tokio::task::spawn_blocking(move || store.read()).await? }),
            )
            .await;
            this.update(cx, |this, cx| {
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
                cx.notify();
            })
            .ok();
        }));
    }
}

async fn send(http: &reqwest::Client) -> Result<()> {
    http.post(ENDPOINT)
        .header(reqwest::header::USER_AGENT, AGENT)
        .timeout(PATIENCE)
        .json(&serde_json::json!({ "version": RUNNING, "os": std::env::consts::OS }))
        .send()
        .await
        .context("cannot reach the usage endpoint")?
        .error_for_status()
        .context("the usage endpoint refused the report")?;
    Ok(())
}
