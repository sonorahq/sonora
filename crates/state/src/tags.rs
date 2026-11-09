use std::sync::Arc;

use music::{MusicApi, Track, TrackTags};

use crate::{Cx, Event, Job, Outcome, Session, Store, Target, Toast, join};

/// Where the tag editor's read of a track's tags stands.
pub enum TagState {
    Loading,
    Ready(Box<TrackTags>),
    Failed(String),
}

/// The tag editor: the track it is open on, its tags as read from the file, and whether a
/// save is under way. It lives in `Store::pages.tags` while the editor is up.
pub struct Tags {
    track: Option<Track>,
    state: TagState,
    saving: bool,
    task: Option<Job>,
}

impl Tags {
    pub fn new() -> Self {
        Self {
            track: None,
            state: TagState::Loading,
            saving: false,
            task: None,
        }
    }

    pub fn track(&self) -> Option<&Track> {
        self.track.as_ref()
    }

    pub fn state(&self) -> &TagState {
        &self.state
    }

    pub fn saving(&self) -> bool {
        self.saving
    }

    pub fn close(&mut self) {
        self.task = None;
        self.track = None;
        self.saving = false;
    }

    pub fn open(&mut self, track: Track, session: &Session, cx: &mut Cx) {
        let Some(id) = track.id.clone() else {
            return;
        };
        let picked = match music::is_local_id(&id) {
            true => session.local_client(),
            false => session.client(),
        };
        let Some(client) = picked else {
            return;
        };

        self.track = Some(track);
        self.state = TagState::Loading;
        self.saving = false;

        let asked = id.clone();
        self.task = Some(cx.spawn(|link| async move {
            let read = client.track_tags(&asked).await;

            link.post(move |store| {
                let Some(this) = editing(store, &id) else {
                    return;
                };
                this.state = match read {
                    Ok(tags) => TagState::Ready(Box::new(tags)),
                    Err(error) => {
                        log::warn!("tags: cannot read the tags: {error:#}");
                        TagState::Failed(format!("{error:#}"))
                    }
                };
            });
        }));
    }

    pub fn save(&mut self, tags: TrackTags, session: &Session, cx: &mut Cx) {
        let Some(id) = self.track.as_ref().and_then(|track| track.id.clone()) else {
            return;
        };
        let Some(client) = self.client(session) else {
            return;
        };
        if self.saving {
            return;
        }

        self.saving = true;

        let io = cx.io.clone();
        let name = tags.title.clone();
        let target = Some(Target::Song(id.clone().into()));
        let has_local_folder = !session.local_paths().is_empty();
        let written_to = id.clone();
        self.task = Some(cx.spawn(|link| async move {
            // A write runs to the end on its own, so closing the editor never leaves a file
            // half written.
            let written =
                join(io.spawn(async move { client.set_track_tags(&written_to, tags).await }))
                    .await;

            link.post(move |store| {
                let Some(this) = editing(store, &id) else {
                    return;
                };
                this.saving = false;
                match written {
                    Ok(()) => {
                        this.track = None;
                        if has_local_folder {
                            store.rescan_local(false);
                        }
                        store.cx.emit(Event::Toast(
                            Toast::new(Outcome::Done, "toast-tags-saved")
                                .name(name)
                                .target(target),
                        ));
                    }
                    Err(error) => {
                        log::warn!("tags: cannot save the tags: {error:#}");
                        store.cx.emit(Event::Toast(Toast::new(
                            Outcome::Failed,
                            "toast-tags-failed",
                        )));
                    }
                }
            });
        }));
    }

    fn client(&self, session: &Session) -> Option<Arc<dyn MusicApi>> {
        let id = self.track.as_ref()?.id.as_deref();
        let local = id.is_some_and(music::is_local_id);
        match local {
            true => session.local_client(),
            false => session.client(),
        }
    }
}

impl Default for Tags {
    fn default() -> Self {
        Self::new()
    }
}

/// The tag editor, while it is still open on the track `id`.
fn editing<'a>(store: &'a mut Store, id: &str) -> Option<&'a mut Tags> {
    store
        .pages
        .tags
        .as_mut()
        .filter(|tags| tags.track.as_ref().and_then(|track| track.id.as_deref()) == Some(id))
}
