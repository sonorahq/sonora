use anyhow::{Context as _, Result, bail};
use serde_json::Value;
use ytmusic::{Client, YtMusic};

const CLIENTS: [(&str, Client); 3] = [
    ("WEB_REMIX", Client::Music),
    ("TVHTML5", Client::Tv),
    ("VISIONOS", Client::VisionOs),
];
const LISTED: usize = 8;

struct Caption {
    language: String,
    name: String,
    generated: bool,
    url: String,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let Some(link) = std::env::args().nth(1) else {
        bail!("usage: captions-prober <youtube link or video id> [language]");
    };
    let id = video_id(&link).context("cannot read a video id out of that link")?;
    let language = std::env::args().nth(2);
    let api = YtMusic::anonymous();
    let http = reqwest::Client::new();

    println!("video {id}");
    for (label, client) in CLIENTS {
        println!();
        println!("{label}");
        let found = match captions(&api, &id, client).await {
            Ok(found) => found,
            Err(error) => {
                println!("  no player response: {error:#}");
                continue;
            }
        };
        if found.is_empty() {
            println!("  no caption track");
            continue;
        }
        for caption in &found {
            println!(
                "  {:<8} {:<28} {}",
                caption.language,
                caption.name,
                match caption.generated {
                    true => "generated",
                    false => "authored",
                }
            );
        }
        let pick = found
            .iter()
            .find(|caption| {
                !caption.generated
                    && language
                        .as_deref()
                        .is_none_or(|wanted| caption.language == wanted)
            })
            .or_else(|| found.first());
        if let Some(caption) = pick {
            dump(&http, caption).await;
        }
    }

    Ok(())
}

async fn captions(api: &YtMusic, id: &str, client: Client) -> Result<Vec<Caption>> {
    let response = api.player_response(id, client).await?;
    let Some(tracks) = response
        .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    Ok(tracks.iter().filter_map(caption).collect())
}

fn caption(found: &Value) -> Option<Caption> {
    Some(Caption {
        language: found.get("languageCode")?.as_str()?.to_owned(),
        name: found
            .pointer("/name/runs/0/text")
            .or_else(|| found.pointer("/name/simpleText"))
            .and_then(Value::as_str)
            .unwrap_or("unnamed")
            .to_owned(),
        generated: found.get("kind").and_then(Value::as_str) == Some("asr"),
        url: found.get("baseUrl")?.as_str()?.to_owned(),
    })
}

async fn dump(http: &reqwest::Client, caption: &Caption) {
    match fetch(http, caption).await {
        Ok(events) => show(&events),
        Err(error) => println!("  {} did not come back: {error:#}", caption.language),
    }
}

async fn fetch(http: &reqwest::Client, caption: &Caption) -> Result<Vec<Value>> {
    let body = http
        .get(format!("{}&fmt=json3", caption.url))
        .send()
        .await
        .context("cannot reach the timed text endpoint")?
        .error_for_status()
        .context("the timed text endpoint refused")?
        .text()
        .await
        .context("cannot read the timed text body")?;
    if body.trim().is_empty() {
        bail!("empty body, the endpoint served nothing");
    }
    let parsed: Value = serde_json::from_str(&body).context("cannot parse the timed text body")?;
    Ok(parsed
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

fn show(events: &[Value]) {
    let lines: Vec<(u64, String, bool)> = events.iter().filter_map(line).collect();
    let Some((last, _, _)) = lines.last() else {
        println!("  no timed line in the track");
        return;
    };
    let worded = lines.iter().filter(|(_, _, worded)| *worded).count();
    println!(
        "  {} lines, {worded} word timed, up to {}",
        lines.len(),
        clock(*last)
    );
    for (at, text, _) in lines.iter().take(LISTED) {
        println!("    [{}] {text}", clock(*at));
    }
}

fn line(event: &Value) -> Option<(u64, String, bool)> {
    let at = event.get("tStartMs")?.as_u64()?;
    let segs = event.get("segs")?.as_array()?;
    let text: String = segs
        .iter()
        .filter_map(|seg| seg.get("utf8").and_then(Value::as_str))
        .collect();
    let text = text.trim().to_owned();
    if text.is_empty() {
        return None;
    }
    let worded = segs
        .iter()
        .filter(|seg| seg.get("tOffsetMs").is_some())
        .count()
        > 1;
    Some((at, text, worded))
}

fn video_id(link: &str) -> Option<String> {
    if let Some(rest) = link.split("youtu.be/").nth(1) {
        return Some(cut(rest));
    }
    if link.contains("youtube.com") {
        return link.split("v=").nth(1).map(cut);
    }
    let bare = link.len() == 11
        && link
            .chars()
            .all(|glyph| glyph.is_ascii_alphanumeric() || glyph == '-' || glyph == '_');
    bare.then(|| link.to_owned())
}

fn cut(rest: &str) -> String {
    rest.split(['?', '&', '/', '#'])
        .next()
        .unwrap_or(rest)
        .to_owned()
}

fn clock(ms: u64) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
