//! Fetching the module from Google's component update service.
//!
//! This is the request Chrome's own component updater makes for the Widevine component, and
//! the one Kodi's InputStream Helper makes for the same file. The answer names the current
//! version and a `.crx3` to download: a signed header in front of an ordinary zip, with the
//! module under `_platform_specific/<os>_<arch>/` and Google's terms in `LICENSE`. The
//! archive's sha256 is checked against the one the service announced before anything is read
//! out of it.
//!
//! Kodi shows those terms and installs only on the user's word, so the download and the
//! install are two steps here: [`offer`] downloads and hands back the terms as an [`Offer`],
//! and [`Offer::install`] puts the module into the store. [`fetch`] runs both for a module the
//! user accepted before, when a newer version is out.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::source::{ARCH, Found, LIBRARY, OS, Origin, prefer, remember, store, version};

/// The update service every Chrome asks for its components.
const UPDATE_URL: &str = "https://update.googleapis.com/service/update2/json";

/// The Widevine component's id on that service.
const APP_ID: &str = "oimompecagnajdejgnnjijobebaeigek";

/// What the service is told the client runs; anything else is answered with nothing, so this
/// is the same version Kodi sends.
const UPDATER_VERSION: &str = "151.0.7922.173";

/// The version the request claims to have, old enough that any answer is an update.
const HAVE: &str = "1.4.9.1088";

/// Google's terms, at the archive root.
const LICENSE: &str = "LICENSE";

/// The files kept beside the module, from the archive root.
const KEPT: [&str; 2] = [LICENSE, "manifest.json"];

/// What the update service offers for this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub size: u64,
    pub sha256: String,
    pub urls: Vec<String>,
}

/// A module downloaded and checked, waiting for the user's word on Google's terms.
pub struct Offer {
    release: Release,
    archive: Vec<u8>,
    license: String,
}

impl Offer {
    pub fn version(&self) -> &str {
        &self.release.version
    }

    /// Google's terms for the module, as the archive carries them.
    pub fn license(&self) -> &str {
        &self.license
    }

    /// Puts the module into the store under its version and answers with the module the
    /// process uses from here on. Older versions in the store are removed once the new one is
    /// complete, except the one this process has already opened. Blocking: file writes.
    pub fn install(self) -> Result<Found> {
        let folder = store().join(&self.release.version);
        let path = folder.join(LIBRARY);
        let zip = crx(&self.archive)?;
        let module = entry(zip, &format!("_platform_specific/{OS}_{ARCH}/{LIBRARY}"))?;
        std::fs::create_dir_all(&folder)
            .with_context(|| format!("cannot create {}", folder.display()))?;
        for name in KEPT {
            if let Ok(text) = entry(zip, name) {
                std::fs::write(folder.join(name), text)
                    .with_context(|| format!("cannot write {name} beside the module"))?;
            }
        }
        let mut file = tempfile::NamedTempFile::new_in(&folder)
            .with_context(|| format!("cannot stage {}", path.display()))?;
        file.write_all(&module)
            .with_context(|| format!("cannot write {}", path.display()))?;
        file.persist(&path)
            .map_err(std::io::Error::from)
            .with_context(|| format!("cannot move {} into place", path.display()))?;
        log::info!("widevine: {} is in the store", self.release.version);

        let found = prefer(found(path));
        prune(&self.release.version, &found.path);
        Ok(found)
    }
}

/// Downloads the version the service offers and reads Google's terms out of it, without
/// installing anything.
pub async fn offer() -> Result<Offer> {
    ensure!(
        !ARCH.is_empty(),
        "google builds no widevine module for this processor"
    );
    let http = client()?;
    let release = latest(&http).await?;
    log::info!(
        "widevine: fetching {} ({} bytes) from google",
        release.version,
        release.size
    );
    let archive = download(&http, &release).await?;
    let license = entry(crx(&archive)?, LICENSE)
        .map(|text| String::from_utf8_lossy(&text).trim().to_string())
        .context("the module archive carries no license")?;
    Ok(Offer {
        release,
        archive,
        license,
    })
}

/// Puts the version the service offers into the store without asking, for a module the user
/// accepted before. Answers with what is there when the store has that version already.
pub async fn fetch() -> Result<Found> {
    ensure!(
        !ARCH.is_empty(),
        "google builds no widevine module for this processor"
    );
    let http = client()?;
    let release = latest(&http).await?;
    let path = store().join(&release.version).join(LIBRARY);
    if path.is_file() {
        log::debug!("widevine: {} is in the store already", release.version);
        return Ok(remember(found(path)));
    }
    log::info!(
        "widevine: fetching {} ({} bytes) from google",
        release.version,
        release.size
    );
    let archive = download(&http, &release).await?;
    Offer {
        release,
        archive,
        license: String::new(),
    }
    .install()
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .build()
        .context("cannot build an http client")
}

/// Asks the service what it has for this platform.
pub async fn latest(http: &reqwest::Client) -> Result<Release> {
    let request = json!({
        "request": {
            "@os": "",
            "@updater": "",
            "acceptformat": "crx3,download,puff,run,xz,zucc",
            "apps": [{
                "appid": APP_ID,
                "installsource": "ondemand",
                "updatecheck": {},
                "version": HAVE,
            }],
            "dedup": "cr",
            "ismachine": false,
            "arch": ARCH,
            "os": { "arch": ARCH, "platform": OS },
            "protocol": "4.0",
            "updaterversion": UPDATER_VERSION,
        }
    });
    let text = http
        .post(UPDATE_URL)
        .json(&request)
        .send()
        .await
        .context("cannot reach google's update service")?
        .error_for_status()
        .context("google's update service refused the request")?
        .text()
        .await
        .context("cannot read the update service's answer")?;
    release(&text)
}

/// Reads the offer out of the service's answer, which is JSON behind a `)]}'` guard line.
fn release(text: &str) -> Result<Release> {
    let json = text.split_once('\n').map_or(text, |(_, rest)| rest);
    let answer: Value = serde_json::from_str(json)
        .context("the update service answered with something other than json")?;
    let check = &answer["response"]["apps"][0]["updatecheck"];
    let status = check["status"].as_str().unwrap_or("");
    ensure!(
        status == "ok",
        "the update service offers no module for this platform ({status})"
    );
    let version = check["nextversion"]
        .as_str()
        .context("the offer names no version")?
        .to_string();
    let operation = &check["pipelines"][0]["operations"][0];
    let size = operation["size"].as_u64().unwrap_or(0);
    let sha256 = operation["out"]["sha256"]
        .as_str()
        .context("the offer carries no checksum")?
        .to_ascii_lowercase();
    let mut urls: Vec<String> = operation["urls"]
        .as_array()
        .context("the offer names nowhere to download from")?
        .iter()
        .filter_map(|url| url["url"].as_str())
        .map(str::to_string)
        .collect();
    // https first: the service lists a plain http mirror ahead of it.
    urls.sort_by_key(|url| !url.starts_with("https://"));
    ensure!(!urls.is_empty(), "the offer names nowhere to download from");
    Ok(Release {
        version,
        size,
        sha256,
        urls,
    })
}

/// Downloads the archive from the first mirror that answers and checks it against the
/// announced sha256.
async fn download(http: &reqwest::Client, release: &Release) -> Result<Vec<u8>> {
    let mut last = None;
    for url in &release.urls {
        match http
            .get(url)
            .send()
            .await
            .and_then(|response| response.error_for_status())
        {
            Ok(response) => {
                let bytes = response
                    .bytes()
                    .await
                    .context("cannot read the module archive")?;
                let digest = Sha256::digest(&bytes);
                let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
                ensure!(
                    hex == release.sha256,
                    "the module archive does not match the checksum google announced"
                );
                return Ok(bytes.to_vec());
            }
            Err(error) => {
                log::warn!("widevine: {url} did not answer: {error:#}");
                last = Some(error);
            }
        }
    }
    let error = last.context("the offer names nowhere to download from")?;
    Err(anyhow::Error::new(error).context("cannot download the module archive"))
}

/// The zip inside a `.crx3`: a magic, a format version, the length of the signed header, the
/// header, then the archive.
fn crx(archive: &[u8]) -> Result<&[u8]> {
    ensure!(
        archive.starts_with(b"Cr24"),
        "the module archive is not a crx"
    );
    let format = u32_at(archive, 4)?;
    ensure!(format == 3, "the module archive is crx{format}, not crx3");
    let header = u32_at(archive, 8)? as usize;
    archive
        .get(12 + header..)
        .context("the module archive ends inside its header")
}

/// One file out of a zip, by its full name in the archive. Stored and deflated entries are
/// understood, which is every entry Google writes.
fn entry(zip: &[u8], wanted: &str) -> Result<Vec<u8>> {
    const END: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const CENTRAL: [u8; 4] = [0x50, 0x4b, 0x01, 0x02];
    const LOCAL: [u8; 4] = [0x50, 0x4b, 0x03, 0x04];

    let end = (0..zip.len().saturating_sub(21))
        .rev()
        .take(u16::MAX as usize + 1)
        .find(|&at| zip[at..].starts_with(&END))
        .context("cannot find the zip directory")?;
    let count = u16_at(zip, end + 10)? as usize;
    let mut at = u32_at(zip, end + 16)? as usize;
    for _ in 0..count {
        ensure!(
            zip.get(at..at + 4) == Some(&CENTRAL),
            "the zip directory is damaged"
        );
        let method = u16_at(zip, at + 10)?;
        let packed = u32_at(zip, at + 20)? as usize;
        let size = u32_at(zip, at + 24)? as usize;
        let name = u16_at(zip, at + 28)? as usize;
        let extra = u16_at(zip, at + 30)? as usize;
        let comment = u16_at(zip, at + 32)? as usize;
        let local = u32_at(zip, at + 42)? as usize;
        let entry = zip
            .get(at + 46..at + 46 + name)
            .context("the zip directory is damaged")?;
        if entry == wanted.as_bytes() {
            ensure!(
                zip.get(local..local + 4) == Some(&LOCAL),
                "the zip entry is damaged"
            );
            let name = u16_at(zip, local + 26)? as usize;
            let extra = u16_at(zip, local + 28)? as usize;
            let start = local + 30 + name + extra;
            let data = zip
                .get(start..start + packed)
                .context("the zip entry is truncated")?;
            return match method {
                0 => Ok(data.to_vec()),
                8 => inflate(data, size),
                other => bail!("the zip entry uses compression method {other}"),
            };
        }
        at += 46 + name + extra + comment;
    }
    bail!("{wanted} is not in the archive")
}

fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>> {
    use std::io::Read as _;
    let mut out = Vec::with_capacity(size);
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .context("cannot inflate the zip entry")?;
    Ok(out)
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    let slice = bytes.get(at..at + 2).context("the archive is truncated")?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    let slice = bytes.get(at..at + 4).context("the archive is truncated")?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn found(path: PathBuf) -> Found {
    Found {
        path,
        origin: Origin::Fetched,
    }
}

/// Removes every other version folder from the store, keeping `keep` and the folder of the
/// module this process has open, which may be older.
fn prune(keep: &str, open: &Path) {
    let Ok(entries) = std::fs::read_dir(store()) else {
        return;
    };
    let held = open.parent();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if version(name).is_none() || name == keep || held == Some(path.as_path()) {
            continue;
        }
        if let Err(error) = std::fs::remove_dir_all(&path) {
            log::warn!("widevine: cannot remove the old {name} module: {error:#}");
        }
    }
}
