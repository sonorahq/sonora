//! Jam invites, membership, and Dealer notifications.

use http::StatusCode;

use super::Worker;
use crate::connect::{Event, Invite, Jam, JamMember, Mode};
use crate::spotify::{auth::sign_in_error, session::Session};
use spotifyapis::dealer::Message;
use spotifyapis::proto::jam::{SessionUpdate, SessionUpdateReason, SocialSession};

/// Where a Jam's token goes to make a link that opens in Spotify's apps and on the web.
pub(in crate::spotify) const INVITE: &str = "https://open.spotify.com/socialsession/";

impl Worker {
    /// Joins the group session a shared link names, as a listener on this device. Spotify then
    /// moves the host's playback here the way it does for any transfer.
    pub(super) async fn join(&mut self, link: &str) {
        if self.mode != Mode::Publish || !self.dealer_ready() {
            return log::warn!("connect: cannot join a jam while not listed as a device");
        }
        let Some(token) = jam_token(link).await else {
            return log::warn!("connect: no jam token in the link");
        };
        let connection = self.session.connection_id().to_owned();
        let result = self
            .session
            .api()
            .join_jam(&token, self.session.device_id(), &connection)
            .await;
        self.enter_jam(result);
    }

    /// Starts one bootstrap read while the worker keeps processing Dealer updates.
    pub(super) fn begin_jam_read(&mut self) {
        if self.jam.is_none() && self.jam_read.is_none() {
            let session = self.session.clone();
            self.jam_read = Some(Box::pin(
                async move { Self::current_session(&session).await },
            ));
        }
    }

    pub(super) fn finish_jam_read(&mut self, read: anyhow::Result<Option<Read>>) {
        self.jam_read = None;
        match read {
            Ok(read) => self.set_jam(read),
            Err(error) => log::warn!("connect: cannot read the jam: {error}"),
        }
    }

    /// Reuses any bootstrap read when a membership action needs the session ID immediately.
    async fn ensure_jam(&mut self) -> anyhow::Result<()> {
        if self.jam.is_none() {
            let read = match self.jam_read.take() {
                Some(read) => read.await?,
                None => Self::current_session(&self.session).await?,
            };
            self.set_jam(read);
        }
        Ok(())
    }

    /// The group session the account is in. Spotify answers 404 when there is none, which is
    /// `Ok(None)`, unlike a request that failed.
    async fn current_session(session: &Session) -> anyhow::Result<Option<Read>> {
        let connection = session.connection_id().to_owned();
        match session
            .api()
            .current_jam(&connection)
            .await
            .map_err(sign_in_error)
        {
            Ok(Some(read)) => Ok(Some(read_social(read, session.username()))),
            Ok(None) => Err(anyhow::anyhow!("the jam answer cannot be read")),
            Err(error) if spotifyapis::http_status(&error) == Some(StatusCode::NOT_FOUND) => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Reads the group session a link names without joining it, and says what it found in an
    /// `Event::Invite`, which is `None` when the link names none or it cannot be read.
    pub(super) async fn peek(&mut self, link: &str) {
        if self.mode == Mode::Publish && !self.dealer_ready() && self.dealer.is_some() {
            self.held = Some(link.to_owned());
            return;
        }
        let found = self.peeked(link).await;
        self.out.send(Event::Invite(found)).ok();
    }

    async fn peeked(&mut self, link: &str) -> Option<Invite> {
        if self.mode != Mode::Publish || !self.dealer_ready() {
            log::warn!("connect: cannot look up a jam while not listed as a device");
            return None;
        }
        let Some(token) = jam_token(link).await else {
            log::warn!("connect: no jam token in the link");
            return None;
        };
        let connection = self.session.connection_id().to_owned();
        let session = self
            .session
            .api()
            .jam_invite(&token, &connection)
            .await
            .inspect_err(|error| log::warn!("connect: cannot look up the jam: {error}"))
            .ok()??;
        Some(Invite {
            link: format!("{INVITE}{token}"),
            jam: read_social(session, self.session.username()).jam,
        })
    }

    /// Starts a group session with this device as the one playing, or finds the one that runs.
    /// The link to invite others with comes back in the `Event::Jam`.
    pub(super) async fn start_jam(&mut self) {
        if self.mode != Mode::Publish || !self.dealer_ready() {
            return log::warn!("connect: cannot start a jam while not listed as a device");
        }
        let connection = self.session.connection_id().to_owned();
        let result = self
            .session
            .api()
            .start_jam(self.session.device_id(), &connection)
            .await;
        self.enter_jam(result);
    }

    /// Applies a start or join response, leaving empty responses for Dealer to complete.
    fn enter_jam(&mut self, result: anyhow::Result<Option<SocialSession>>) {
        match result.map_err(sign_in_error) {
            Ok(Some(read)) => self.set_jam(Some(read_social(read, self.session.username()))),
            Ok(None) => {}
            Err(error) => {
                log::warn!("connect: cannot start or join the jam: {error}");
                self.out.send(Event::JamFailed).ok();
            }
        }
    }

    /// Leaves the group session. The owner ends it for everyone instead, since leaving would
    /// only hand it on. A session that cannot be read is left as it is, since it may be owned.
    pub(super) async fn leave(&mut self) {
        if self.mode != Mode::Publish {
            return;
        }
        if let Err(error) = self.ensure_jam().await {
            log::warn!("connect: cannot read the jam to leave it: {error}");
            self.out.send(Event::LeaveFailed).ok();
            return;
        }
        if matches!(self.jam, Some(None)) {
            self.out.send(Event::Jam(None)).ok();
            return;
        }
        let Some((id, owner)) = self
            .jam
            .as_ref()
            .and_then(Option::as_ref)
            .filter(|read| is_path_id(&read.id, "_-"))
            .map(|read| (read.id.clone(), read.jam.owner))
        else {
            self.out.send(Event::LeaveFailed).ok();
            return;
        };
        let connection = self.session.connection_id().to_owned();
        match self
            .session
            .api()
            .leave_jam(&id, owner, self.session.device_id(), &connection)
            .await
            .map_err(sign_in_error)
        {
            Ok(_) => {
                self.set_jam(None);
            }
            Err(error) => {
                log::warn!("connect: cannot leave the jam: {error}");
                self.jam = None;
                self.out.send(Event::LeaveFailed).ok();
            }
        }
    }

    /// Removes `member` from the group session, which only its owner may do, and then reports
    /// who is left without waiting for the dealer to.
    pub(super) async fn kick(&mut self, member: &str) {
        if self.mode != Mode::Publish {
            return;
        }
        // the id goes into the path, so nothing else than what an id is made of is let through
        if !is_path_id(member, "._-") {
            return log::warn!("connect: not a member id to remove");
        }
        if let Err(error) = self.ensure_jam().await {
            return log::warn!("connect: cannot read the jam to remove from: {error}");
        }
        let Some(id) = self
            .jam
            .as_ref()
            .and_then(Option::as_ref)
            .filter(|read| read.jam.owner && is_path_id(&read.id, "_-"))
            .map(|read| read.id.clone())
        else {
            return log::warn!("connect: only the owner of a jam can remove a member");
        };
        let connection = self.session.connection_id().to_owned();
        let answer = self
            .session
            .api()
            .kick_jam_member(&id, member, self.session.device_id(), &connection)
            .await
            .map_err(sign_in_error);
        match answer {
            Ok(_) => {
                if let Some(Some(read)) = &mut self.jam {
                    read.jam.members.retain(|entry| entry.id != member);
                    self.out.send(Event::Jam(Some(read.jam.clone()))).ok();
                }
            }
            Err(error) => {
                self.jam = None;
                log::warn!("connect: cannot remove the member: {error}");
            }
        }
    }

    /// Handles the JSON notifications that Dealer sends for Jam.
    pub(super) fn jam_update(&mut self, message: Message) {
        let update: anyhow::Result<SessionUpdate> = message.decode_json();
        let Ok(mut update) = update else {
            return log::warn!("connect: unreadable jam update");
        };
        let ended = matches!(
            update.reason,
            SessionUpdateReason::SessionDeleted
                | SessionUpdateReason::YouLeft
                | SessionUpdateReason::YouWereKicked
        );
        let session = update
            .session
            .take()
            .filter(|session| !session.session_id.is_empty());
        if ended {
            let current = self.jam.as_ref().and_then(Option::as_ref);
            if !matches!((&session, current), (Some(ended), Some(current)) if ended.session_id != current.id)
            {
                self.set_jam(None);
            }
        } else if let Some(session) = session {
            self.set_jam(Some(read_social(session, self.session.username())));
        }
    }

    /// Records the latest Jam snapshot for actions that need its ID or owner.
    fn set_jam(&mut self, read: Option<Read>) {
        self.jam_read = None;
        if self.jam.as_ref() == Some(&read) {
            return;
        }
        let jam = read.as_ref().map(|read| read.jam.clone());
        self.jam = Some(read);
        self.out.send(Event::Jam(jam)).ok();
    }
}

/// The token of a group session in a shared link. Only an https `spotify.link` short link is
/// fetched, and then its address or its page names the session; no other text makes the app
/// fetch anything.
async fn jam_token(link: &str) -> Option<String> {
    let link = link.trim();
    let short = crate::link::address(link)
        .is_some_and(|url| url.scheme() == "https" && url.host_str() == Some("spotify.link"));
    if !short {
        return token_in(link);
    }
    // a short link sometimes answers with a page that names the session instead of redirecting
    let followed = crate::link::follow(link).await?;
    token_in(&followed.address).or_else(|| token_in(&followed.page))
}

/// The token that follows `socialsession/` in a link or a page, or `sessions/join/` in the
/// address Spotify itself gives a session. It is letters and digits only, since it goes into a
/// request.
fn token_in(text: &str) -> Option<String> {
    let rest = ["socialsession/", "sessions/join/"]
        .into_iter()
        .find_map(|marker| text.split(marker).nth(1))?;
    let token = rest.split(|c: char| !c.is_ascii_alphanumeric()).next()?;
    (!token.is_empty()).then(|| token.to_owned())
}

/// The link that invites others to the session at `url`, which Spotify gives as an address only
/// its own apps open.
fn invite(url: &str) -> Option<String> {
    match token_in(url) {
        Some(token) => Some(format!("{INVITE}{token}")),
        None => url.starts_with("https://").then(|| url.to_owned()),
    }
}

/// A group session as Spotify answers for it: what the app shows, the id to end it by, and
/// whether the account owns it.
#[derive(PartialEq, Eq)]
pub(super) struct Read {
    jam: Jam,
    id: String,
}

/// Converts one decoded session, retaining owner-only invite links.
fn read_social(session: SocialSession, own: &str) -> Read {
    let link = session
        .is_session_owner
        .then(|| invite(&session.join_session_url))
        .flatten();
    let owner = &session.session_owner_id;
    let mut members: Vec<_> = session
        .session_members
        .into_iter()
        .filter_map(|member| {
            let id = [member.id, member.username.clone()]
                .into_iter()
                .find(|id| !id.is_empty())?;
            let image = if member.image_url.is_empty() {
                member.large_image_url
            } else {
                member.image_url
            };
            Some(JamMember {
                host: !owner.is_empty() && (id == *owner || member.username == *owner),
                you: id == own || member.username == own,
                name: [member.display_name, member.username.clone()]
                    .into_iter()
                    .find(|name| !name.is_empty())
                    .unwrap_or_else(|| id.clone()),
                image: match image.strip_prefix("spotify:image:") {
                    Some(id) => Some(format!("https://i.scdn.co/image/{id}")),
                    None => image.starts_with("https://").then_some(image),
                },
                username: member.username,
                id,
            })
        })
        .collect();
    if session.is_session_owner
        && !members.iter().any(|member| member.host)
        && let Some(you) = members.iter_mut().find(|member| member.you)
    {
        you.host = true;
    }
    Read {
        jam: Jam {
            members,
            owner: session.is_session_owner,
            link,
        },
        id: session.session_id,
    }
}

/// Whether `id` is non-empty and made only of letters, digits and the `extra` characters, which
/// makes it safe to put in a URL path.
fn is_path_id(id: &str, extra: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
}
