//! The system CDM, as the one process-wide resource it actually is.
//!
//! The underlying host is a single global instance. Every call here is serialized behind one
//! lock, and its session accumulates content keys rather than replacing them, so more than one
//! track can be licensed at a time. That is what makes preloading the next track possible.

#[cfg(feature = "cdm")]
mod host {
    use std::sync::{Mutex, MutexGuard};

    use anyhow::{Context as _, Result, bail};
    use cdm_host::CdmHost;

    /// The one CDM this process has, opened on first use and kept for the rest of the session.
    /// Re-opening would re-initialize the same native instance and take the keys of whatever is
    /// playing with it, so it is opened once.
    static CDM: Mutex<Option<CdmHost>> = Mutex::new(None);

    /// A handle to the process CDM. Every call takes the same lock, in the order it arrives.
    pub struct Cdm;

    fn held() -> Result<MutexGuard<'static, Option<CdmHost>>> {
        CDM.lock()
            .map_err(|_| anyhow::anyhow!("the widevine cdm is poisoned"))
    }

    impl Cdm {
        /// Opens the CDM [`crate::find`] settles on, or hands back the one already open.
        pub fn open() -> Result<Self> {
            let mut cdm = held()?;
            if cdm.is_some() {
                return Ok(Self);
            }
            let Some(found) = crate::find() else {
                bail!("no widevine module was found, so there is no cdm to open");
            };
            let path = found.path;
            log::info!(
                "widevine: opening the {:?} cdm at {}",
                found.origin,
                path.display()
            );
            // Deliberately `open` and not `install_or_open`: the file is read where it already
            // is, and the only copy Sonora ever makes is the one it fetched on request.
            let opened = CdmHost::open(&path)
                .with_context(|| format!("cannot open the widevine cdm at {}", path.display()))?;
            *cdm = Some(opened);
            Ok(Self)
        }

        /// The license challenge for `init`, a `pssh` box. What comes back goes to the
        /// provider's license endpoint untouched.
        pub fn challenge(&self, init: &[u8]) -> Result<Vec<u8>> {
            let cdm = held()?;
            let cdm = cdm.as_ref().context("the widevine cdm is not open")?;
            cdm.challenge(init).context("cannot build a challenge")
        }

        /// Hands the license back to the CDM, which loads the content keys it carries. An
        /// earlier track's keys stay loaded beside them.
        pub fn accept(&self, license: &[u8]) -> Result<()> {
            let cdm = held()?;
            let cdm = cdm.as_ref().context("the widevine cdm is not open")?;
            cdm.update(license)
                .context("the widevine cdm refused the license")
        }

        /// Decrypts one CENC sample. `subs` is empty when the whole sample is encrypted.
        pub fn decrypt(
            &self,
            sample: &[u8],
            key_id: &[u8],
            iv: &[u8; 16],
            subs: &[(u32, u32)],
        ) -> Result<Vec<u8>> {
            let cdm = held()?;
            let cdm = cdm.as_ref().context("the widevine cdm is not open")?;
            cdm.decrypt(sample, key_id, iv, subs)
                .context("cannot decrypt a sample")
        }
    }
}

#[cfg(not(feature = "cdm"))]
mod host {
    use anyhow::{Result, bail};

    /// A handle to a CDM this build has no host for.
    pub struct Cdm;

    impl Cdm {
        pub fn open() -> Result<Self> {
            bail!("this build carries no widevine host")
        }

        pub fn challenge(&self, _init: &[u8]) -> Result<Vec<u8>> {
            bail!("this build carries no widevine host")
        }

        pub fn accept(&self, _license: &[u8]) -> Result<()> {
            bail!("this build carries no widevine host")
        }

        pub fn decrypt(
            &self,
            _sample: &[u8],
            _key_id: &[u8],
            _iv: &[u8; 16],
            _subs: &[(u32, u32)],
        ) -> Result<Vec<u8>> {
            bail!("this build carries no widevine host")
        }
    }
}

pub use host::Cdm;
