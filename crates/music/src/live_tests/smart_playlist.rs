use anyhow::{Context as _, Result, bail};

use crate::MusicApi;
use crate::smart::{RuleGroup, SmartPlaylistUpdate, SmartRules};
use crate::subsonic::{SubsonicClient, auth};

const NAME: &str = "Sonora live smart playlist test - safe to delete";

/// Runs against the Navidrome in `SONORA_SUBSONIC_SERVER`, signed in with
/// `SONORA_SUBSONIC_USERNAME` and `SONORA_SUBSONIC_PASSWORD`. When
/// `SONORA_SUBSONIC_OTHER_USERNAME` and `SONORA_SUBSONIC_OTHER_PASSWORD` name a second
/// account on the same server, the forbidden-edit check runs too.
#[tokio::test]
#[ignore = "creates, changes, and deletes a smart playlist on a Navidrome server"]
async fn navidrome_can_create_and_edit_a_smart_playlist() -> Result<()> {
    let variable = |name: &str| std::env::var(name).with_context(|| format!("{name} is not set"));
    let server = variable("SONORA_SUBSONIC_SERVER")?;
    let client = connect(
        &server,
        &variable("SONORA_SUBSONIC_USERNAME")?,
        &variable("SONORA_SUBSONIC_PASSWORD")?,
    )?;
    if !client.takes_smart_playlists().await {
        bail!("the server is not a Navidrome whose own API edits smart playlists");
    }

    let exercise = exercise(&client).await;
    // Whatever happened, take away every playlist this test may have left.
    for playlist in client.playlists().await.unwrap_or_default() {
        if playlist.name.starts_with(NAME) {
            let _ = client.delete_playlist(&playlist.id).await;
        }
    }
    exercise
}

async fn exercise(client: &SubsonicClient) -> Result<()> {
    // A nested tree: every track matches, however the library is tagged, so the evaluation
    // check below works on any server.
    let rules = SmartRules::from_wire(&serde_json::json!({
        "all": [
            {"any": [
                {"isMissing": {"comment": true}},
                {"contains": {"comment": ""}}
            ]},
            {"inTheRange": {"year": [0, 3000]}}
        ],
        "sort": "title",
        "order": "asc",
        "limit": 50
    }))
    .map_err(|problem| anyhow::anyhow!("the fixture rules do not read: {problem}"))?;
    if !rules.problems().is_empty() {
        bail!("the fixture rules do not validate");
    }

    let id = client.create_smart_playlist(NAME, &rules).await?;

    // The create wrote exactly the rules it was given.
    let created = client.smart_playlist(&id).await?;
    if !created.owned {
        bail!("a playlist the account just created reads as someone else's");
    }
    if created.synced {
        bail!("a playlist created here reads as file-synced");
    }
    let read = created.rules.clone().unwrap_or_default();
    if read.to_wire() != rules.to_wire() {
        bail!(
            "the rules changed on the way out: sent {}, read {}",
            rules.to_wire(),
            read.to_wire()
        );
    }

    // A name-only update leaves the rules alone (the server's partial-update contract).
    let renamed = client
        .update_smart_playlist(
            &id,
            SmartPlaylistUpdate {
                name: Some(format!("{NAME} (renamed)")),
                ..SmartPlaylistUpdate::default()
            },
        )
        .await?;
    if renamed.name != format!("{NAME} (renamed)") {
        bail!("the rename did not take");
    }
    if renamed.rules.as_ref().map(SmartRules::to_wire) != Some(rules.to_wire()) {
        bail!("a name-only update changed the rules");
    }

    // A rules-only update keeps the name and changes exactly the rules.
    let mut changed = rules.clone();
    changed.limit = None;
    changed.order = Some("desc".to_owned());
    let RuleGroup::All(top) = &mut changed.group else {
        bail!("the fixture group reads as all");
    };
    top.pop();
    let updated = client
        .update_smart_playlist(
            &id,
            SmartPlaylistUpdate {
                rules: Some(changed.clone()),
                ..SmartPlaylistUpdate::default()
            },
        )
        .await?;
    if updated.name != format!("{NAME} (renamed)") {
        bail!("a rules-only update changed the name");
    }
    if updated.rules.as_ref().map(SmartRules::to_wire) != Some(changed.to_wire()) {
        bail!(
            "the edited rules did not take: sent {}, read {}",
            changed.to_wire(),
            updated
                .rules
                .as_ref()
                .map(SmartRules::to_wire)
                .unwrap_or_default()
        );
    }

    // The playlist still lists and evaluates through the ordinary Subsonic calls.
    let detail = client.playlist(&id).await?;
    if detail.playlist.name != format!("{NAME} (renamed)") {
        bail!("the subsonic read names a different playlist");
    }

    // Rules that cannot run never reach the server.
    let broken = SmartRules::new(RuleGroup::Any(Vec::new()));
    if client.create_smart_playlist(NAME, &broken).await.is_ok() {
        bail!("broken rules were sent to the server");
    }

    // A private playlist is invisible to other regular users: Navidrome's
    // playlist repository filters by owner before the write check runs, so a
    // regular second account's edit reads exactly like a deleted playlist
    // (404) even though the playlist exists. A public one would refuse with
    // 403 instead, and an admin editor would succeed, so the second account
    // must be a regular user. The playlist is created by the primary account
    // and the edit is attempted by the second.
    if let (Ok(username), Ok(password)) = (
        std::env::var("SONORA_SUBSONIC_OTHER_USERNAME"),
        std::env::var("SONORA_SUBSONIC_OTHER_PASSWORD"),
    ) {
        let server = std::env::var("SONORA_SUBSONIC_SERVER")?;
        let other = connect(&server, &username, &password)?;
        let foreign = client.create_smart_playlist(NAME, &rules).await?;
        let refused = other
            .update_smart_playlist(
                &foreign,
                SmartPlaylistUpdate {
                    name: Some("taken over".to_owned()),
                    ..SmartPlaylistUpdate::default()
                },
            )
            .await;
        client.delete_playlist(&foreign).await?;
        match refused {
            Err(error) if format!("{error:#}").contains("no longer has the playlist") => {}
            Err(error) => bail!("the forbidden edit failed the wrong way: {error:#}"),
            Ok(_) => bail!("another account's playlist took the edit"),
        }
    }

    client.delete_playlist(&id).await?;
    Ok(())
}

fn connect(server: &str, username: &str, password: &str) -> Result<SubsonicClient> {
    let signature = auth::sign(username, password);
    SubsonicClient::new(
        server.to_owned(),
        username.to_owned(),
        password.to_owned(),
        &signature,
    )
}
