use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

const BASE: &str = "lucide";
const KIND: &str = "svg";

fn main() {
    let assets = workspace().join("assets/icons");

    let mut folders = embed::tree(&assets, KIND);
    folders.sort_by_key(|(id, _)| (id != BASE, id.clone()));

    let mut icons = String::new();
    let mut packs = String::new();
    let mut names = Vec::new();
    let mut start = 0;

    for (id, gathered) in &folders {
        for item in gathered {
            let name = &item.name;
            let path = format!("icons/{id}/{name}.{KIND}");
            let file = item.file.to_str().expect("a utf-8 path");
            let svg = fs::read_to_string(&item.file).expect("an icon is readable text");
            let (view, body) = drawn(&svg);
            println!("cargo:rerun-if-changed={file}");
            writeln!(
                icons,
                "    Icon {{ name: {name:?}, path: {path:?}, bytes: include_bytes!({file:?}), \
                 drawn: rune::Icon {{ name: {name:?}, view: {view:?}, body: {body:?} }} }},"
            )
            .expect("a string never fails");
            names.push(name.clone());
        }

        let end = start + gathered.len();
        writeln!(
            packs,
            "    Pack {{ id: {id:?}, start: {start}, end: {end} }},"
        )
        .expect("a string never fails");
        start = end;
    }

    names.sort();
    names.dedup();
    let names: String = names
        .iter()
        .map(|name| format!("    {name:?},\n"))
        .collect();

    let source = format!(
        "pub(crate) static ICONS: &[Icon] = &[\n{icons}];\n\n\
         pub(crate) static PACKS: &[Pack] = &[\n{packs}];\n\n\
         pub static NAMES: &[&str] = &[\n{names}];\n"
    );

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets the output")).join("packs.rs");
    fs::write(&out, source).expect("cannot write the icon registry");
    println!("cargo:rerun-if-changed=build.rs");
}

/// The paint attributes an icon's root may set, which Rune's icons take from a group instead.
const PAINT: [&str; 8] = [
    "fill",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "fill-rule",
    "clip-rule",
    "opacity",
];

/// An SVG file as Rune draws an icon: the side of its square view box, and its elements inside a
/// group that carries what the root painted with.
fn drawn(svg: &str) -> (f32, String) {
    let open = svg.find("<svg").expect("an icon is an svg");
    let close = open + svg[open..].find('>').expect("the svg tag closes");
    let root = &svg[open..close];
    let end = svg.rfind("</svg>").expect("the svg element ends");
    let inner = svg[close + 1..end].trim();
    let view = attribute(root, "viewBox")
        .and_then(|bounds| bounds.split_whitespace().nth(2)?.parse().ok())
        .or_else(|| attribute(root, "width")?.parse().ok())
        .unwrap_or(24.0);
    let painted: String = PAINT
        .iter()
        .filter_map(|name| Some(format!(" {name}=\"{}\"", attribute(root, name)?)))
        .collect();
    let body = format!("<g{painted}>{inner}</g>")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (view, body)
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().last();
        let after = &rest[at + name.len()..];
        rest = after;
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let Some(value) = after.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start().strip_prefix('"')?;
        return value.split('"').next();
    }
}

/// The workspace root, taken from the crate's own manifest directory without touching the
/// filesystem, since `canonicalize` fails on a shared drive inside a Windows VM.
fn workspace() -> PathBuf {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo names the crate"));
    manifest
        .ancestors()
        .nth(2)
        .expect("the crate lives two levels under the workspace")
        .to_path_buf()
}
