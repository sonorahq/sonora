use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context as _, Result};
use roxmltree::{Document, Node};

use music::{Lyrics, LyricsLane, LyricsLine, LyricsWord, Voice};

const META: &str = "http://www.w3.org/ns/ttml#metadata";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const BACKGROUND: &str = "x-bg";
const ASIDES: [&str; 2] = ["x-translation", "x-roman"];

pub struct Sheet {
    pub lyrics: Lyrics,
    pub writers: Vec<String>,
}

pub fn parse(xml: &str) -> Result<Sheet> {
    let document = Document::parse(xml).context("cannot read the ttml")?;
    let singers = singers(&document);
    let lead = lead(&document, &singers);
    let mut lines: Vec<LyricsLine> = paragraphs(&document)
        .filter_map(|paragraph| line(paragraph, lead, &singers))
        .collect();

    let timed = lines
        .iter()
        .any(|line| line.end.is_some() || line.words.is_some());
    let lyrics = match timed {
        true => {
            super::lrc::normalize(&mut lines);
            Lyrics::Synced {
                lines: lines.into(),
            }
        }
        false => crate::plain(
            lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    };

    Ok(Sheet {
        lyrics,
        writers: writers(&document),
    })
}

fn paragraphs<'a>(document: &'a Document<'a>) -> impl Iterator<Item = Node<'a, 'a>> {
    document.descendants().filter(|node| named(*node, "p"))
}

fn writers(document: &Document) -> Vec<String> {
    document
        .descendants()
        .filter(|node| named(*node, "songwriter"))
        .filter_map(|node| node.text())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect()
}

fn singers<'a>(document: &'a Document<'a>) -> HashMap<&'a str, bool> {
    document
        .descendants()
        .filter(|node| named(*node, "agent"))
        .filter_map(|node| {
            let id = node.attribute((XML, "id"))?;
            Some((id, node.attribute("type") == Some("person")))
        })
        .collect()
}

fn lead<'a>(document: &'a Document<'a>, singers: &HashMap<&str, bool>) -> Option<&'a str> {
    paragraphs(document)
        .filter_map(|paragraph| paragraph.attribute((META, "agent")))
        .find(|agent| sings(singers, agent))
}

fn line(paragraph: Node, lead: Option<&str>, singers: &HashMap<&str, bool>) -> Option<LyricsLine> {
    let (words, secondary) = sung(paragraph);
    let text = match words.is_empty() {
        true => spoken(paragraph),
        false => joined(&words),
    };
    if text.is_empty() && secondary.is_empty() {
        return None;
    }

    let start = stamp(paragraph, "begin")
        .or_else(|| words.first().map(|word| word.start))
        .or_else(|| secondary.first().map(|lane| lane.start))
        .unwrap_or(Duration::ZERO);
    let end = stamp(paragraph, "end").or_else(|| words.last().map(|word| word.end));

    Some(LyricsLine {
        start,
        end: end.map(|end| end.max(start)),
        text,
        romanized: None,
        words: (!words.is_empty()).then_some(words),
        secondary,
        voice: voice(paragraph, lead, singers),
    })
}

fn voice(paragraph: Node, lead: Option<&str>, singers: &HashMap<&str, bool>) -> Voice {
    let Some(agent) = paragraph.attribute((META, "agent")) else {
        return Voice::Lead;
    };
    match sings(singers, agent) && Some(agent) != lead {
        true => Voice::Counter,
        false => Voice::Lead,
    }
}

fn sings(singers: &HashMap<&str, bool>, agent: &str) -> bool {
    singers.get(agent).copied().unwrap_or(false)
}

fn sung(parent: Node) -> (Vec<LyricsWord>, Vec<LyricsLane>) {
    let mut words: Vec<LyricsWord> = Vec::new();
    let mut lanes = Vec::new();
    for child in parent.children() {
        if child.is_text() {
            if let Some(word) = words.last_mut() {
                word.text.push_str(child.text().unwrap_or_default());
            }
            continue;
        }
        if !child.is_element() {
            continue;
        }
        match role(child) {
            Some(BACKGROUND) => lanes.extend(lane(child)),
            Some(role) if ASIDES.contains(&role) => {}
            _ if child.has_attribute("begin") => words.extend(word(child)),
            _ => {
                let (inner, mut nested) = sung(child);
                words.extend(inner);
                lanes.append(&mut nested);
            }
        }
    }
    (words, lanes)
}

fn lane(span: Node) -> Option<LyricsLane> {
    let (words, _) = sung(span);
    let text = joined(&words);
    if text.is_empty() {
        return None;
    }
    Some(LyricsLane {
        start: stamp(span, "begin").or_else(|| words.first().map(|word| word.start))?,
        end: stamp(span, "end").or_else(|| words.last().map(|word| word.end)),
        text,
        romanized: None,
        words: Some(words),
    })
}

fn word(span: Node) -> Option<LyricsWord> {
    let start = stamp(span, "begin")?;
    let end = stamp(span, "end").unwrap_or(start);
    let text: String = span
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect();
    (!text.trim().is_empty()).then_some(LyricsWord {
        start,
        end: end.max(start),
        text,
    })
}

fn spoken(paragraph: Node) -> String {
    paragraph
        .descendants()
        .filter(Node::is_text)
        .filter(|node| !aside(*node))
        .filter_map(|node| node.text())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn aside(node: Node) -> bool {
    node.ancestors()
        .filter_map(role)
        .any(|role| role == BACKGROUND || ASIDES.contains(&role))
}

fn joined(words: &[LyricsWord]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<String>()
        .trim()
        .to_owned()
}

fn role<'a>(node: Node<'a, 'a>) -> Option<&'a str> {
    node.attribute((META, "role"))
}

fn named(node: Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn stamp(node: Node, name: &str) -> Option<Duration> {
    let stamp = node.attribute(name)?.trim().trim_end_matches('s');
    let mut seconds = 0f64;
    for part in stamp.split(':') {
        seconds = seconds * 60. + part.parse::<f64>().ok()?;
    }
    if !seconds.is_finite() {
        return None;
    }
    Duration::try_from_secs_f64(seconds.max(0.)).ok()
}
