mod auth;
mod client;
mod playback;
mod stream;
mod wire;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::deezer::client::{DeezerClient, Session};
use crate::deezer::playback::Factory;
use crate::{
    InputSource, MusicProvider, PromptSink, ProviderSession, SignIn, SignInPrompt, UserProfile,
    credentials,
};

const GUEST_ID: &str = "deezer-guest";

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Saved {
    Arl { arl: String },
    Guest,
}

pub struct DeezerProvider {
    credentials: PathBuf,
}

impl DeezerProvider {
    pub fn new() -> Self {
        let cache = credentials::dir("deezer");
        Self {
            credentials: cache.join(credentials::FILE),
        }
    }

    fn save(&self, saved: &Saved) -> Result<()> {
        let body = serde_json::to_vec(saved).context("cannot encode deezer credentials")?;
        credentials::write(&self.credentials, &body)
    }

    fn saved(&self) -> Option<Saved> {
        let body = std::fs::read(&self.credentials).ok()?;
        match serde_json::from_slice(&body) {
            Ok(saved) => Some(saved),
            Err(error) => {
                log::warn!("deezer: cannot read the stored credentials: {error}");
                None
            }
        }
    }

    fn wrap(&self, session: Session, user: UserProfile) -> ProviderSession {
        let session = Arc::new(session);
        let authenticated = !user.id.is_empty();
        ProviderSession {
            profile: user.clone(),
            api: Arc::new(DeezerClient::new(session.clone(), user)),
            playback: Arc::new(Factory::new(session)),
            authenticated,
            playcounts: false,
        }
    }

    async fn connect(&self, secret: &str) -> Result<ProviderSession> {
        let arl = auth::arl(secret)?;
        let session = Session::new(Some(arl.clone()));
        let user = session.identify().await?;
        self.save(&Saved::Arl { arl })?;
        Ok(self.wrap(session, user))
    }

    fn guest(&self) -> ProviderSession {
        let session = Arc::new(Session::new(None));
        ProviderSession {
            profile: UserProfile {
                id: GUEST_ID.to_string(),
                display_name: "Deezer".to_string(),
            },
            api: Arc::new(DeezerClient::new(
                session.clone(),
                UserProfile {
                    id: String::new(),
                    display_name: "Deezer".to_string(),
                },
            )),
            playback: Arc::new(Factory::new(session)),
            authenticated: false,
            playcounts: false,
        }
    }
}

impl Default for DeezerProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MusicProvider for DeezerProvider {
    fn name(&self) -> &'static str {
        "Deezer"
    }

    fn slug(&self) -> &'static str {
        "deezer"
    }

    fn sign_in_options(&self) -> Vec<SignIn> {
        vec![SignIn::Anonymous, SignIn::Secret]
    }

    fn stored(&self) -> bool {
        self.credentials.exists()
    }

    async fn restore(&self) -> Result<Option<ProviderSession>> {
        match self.saved() {
            Some(Saved::Arl { arl }) => {
                let session = Session::new(Some(arl));
                match session.identify().await {
                    Ok(user) => Ok(Some(self.wrap(session, user))),
                    Err(error) => {
                        log::warn!("deezer: stored ARL was refused: {error:#}");
                        Ok(None)
                    }
                }
            }
            Some(Saved::Guest) => Ok(Some(self.guest())),
            None => Ok(None),
        }
    }

    async fn sign_in(
        &self,
        method: SignIn,
        prompt: PromptSink,
        mut input: InputSource,
    ) -> Result<ProviderSession> {
        match method {
            SignIn::Anonymous | SignIn::Default => {
                self.save(&Saved::Guest)?;
                Ok(self.guest())
            }
            SignIn::Secret => {
                prompt(SignInPrompt::Secret);
                let secret = input.recv().await.context("sign-in was cancelled")?;
                self.connect(&secret).await
            }
            SignIn::Path(_) => Err(anyhow::anyhow!("deezer does not sign in with a folder path")),
        }
    }

    fn sign_out(&self) {
        credentials::remove(&self.credentials);
    }
}
