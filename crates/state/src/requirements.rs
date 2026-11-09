//! What the system around Sonora is missing for it to work fully: the ALSA plugin into the sound
//! server, an output device that opens, the browser engine for cookie sign-ins. Each is found
//! here and drawn as a card by `views`, which can put it away for the run or for good.

use music::SoundServer;

use crate::{AppSettings, Change, Cx, Job, Outputs, Session, Store, Topic};

/// One thing missing from the system, named by what a user installs to fix it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The sound server runs but ALSA has no plugin into it, so nothing played is heard.
    AudioBridge(SoundServer),
    /// The output device would not open for some other reason.
    AudioOutput,
    /// webkit2gtk is missing, so a provider that signs in through a browser window cannot.
    WebEngine,
}

impl Requirement {
    /// The name `settings.json` keeps a silenced requirement under.
    pub fn slug(self) -> &'static str {
        match self {
            Self::AudioBridge(_) => "audio-bridge",
            Self::AudioOutput => "audio-output",
            Self::WebEngine => "web-engine",
        }
    }
}

/// The requirements this machine is missing. The system is looked at once at startup, off the
/// main thread; the output is followed through `Outputs`, so a device that fails later or comes
/// back is noticed.
pub struct Requirements {
    bridge: Option<SoundServer>,
    output: bool,
    web_engine: bool,
    /// Put away until the next run.
    dismissed: Vec<&'static str>,
    task: Option<Job>,
}

impl Requirements {
    /// Starts looking at the system and takes the output's state from `outputs`. Whether the
    /// browser engine matters follows the provider in use, which [`Self::shown`] reads.
    pub fn new(outputs: &Outputs, cx: &mut Cx) -> Self {
        let task = cx.spawn(|link| async move {
            let looked =
                tokio::task::spawn_blocking(|| (music::missing_bridge(), !webview::supported()))
                    .await;
            let Ok((bridge, web_engine)) = looked else {
                return;
            };
            if let Some(server) = bridge {
                log::warn!("requirements: {server:?} runs without its ALSA plugin");
            }
            link.post(move |store| {
                let this = &mut store.requirements;
                this.task = None;
                this.bridge = bridge;
                this.web_engine = web_engine && cfg!(target_os = "linux");
            });
        });

        Self {
            bridge: None,
            output: outputs.failing(),
            web_engine: false,
            dismissed: Vec::new(),
            task: Some(task),
        }
    }

    /// The requirements to show, most pressing first. A failed output whose cause is already
    /// known to be the missing ALSA plugin is left to that card.
    pub fn shown(&self, settings: &AppSettings, session: &Session) -> Vec<Requirement> {
        let found = [
            self.bridge.map(Requirement::AudioBridge),
            (self.output && self.bridge.is_none()).then_some(Requirement::AudioOutput),
            (self.web_engine && session.wants_browser()).then_some(Requirement::WebEngine),
        ];
        found
            .into_iter()
            .flatten()
            .filter(|requirement| {
                let slug = requirement.slug();
                !self.dismissed.contains(&slug) && !settings.requirement_silenced(slug)
            })
            .collect()
    }

    /// Puts `requirement` away until the next run.
    pub fn dismiss(&mut self, requirement: Requirement) {
        self.dismissed.push(requirement.slug());
    }

    /// Puts `requirement` away for good.
    pub fn silence(&self, requirement: Requirement, settings: &mut AppSettings, cx: &mut Cx) {
        settings.silence_requirement(requirement.slug(), cx);
    }

    /// Follows the output after `Outputs` changed. An output that came back takes its card away
    /// and lets it show again should it fail later in the run.
    fn listen(&mut self, outputs: &Outputs) {
        let output = outputs.failing();
        if output == self.output {
            return;
        }
        match output {
            true => log::warn!("requirements: the audio output cannot be opened"),
            false => {
                let slug = Requirement::AudioOutput.slug();
                self.dismissed.retain(|dismissed| *dismissed != slug);
            }
        }
        self.output = output;
    }
}

impl Store {
    /// Puts `requirement` away for good.
    pub fn silence_requirement(&mut self, requirement: Requirement) {
        self.requirements
            .silence(requirement, &mut self.settings, &mut self.cx);
    }

    /// The requirements to show, most pressing first.
    pub fn requirements_shown(&self) -> Vec<Requirement> {
        self.requirements.shown(&self.settings, &self.session)
    }

    pub(crate) fn react_requirements(&mut self, change: &Change) {
        if let Change::Topic(Topic::Outputs) = change {
            self.requirements.listen(&self.outputs);
        }
    }
}
