//! Theme files
//!
//! A theme file names its colours over the dark palette. The app bundles some from
//! `assets/themes`, the user adds their own to the themes folder, and both are read by the same
//! parser. A folder is a family and each file in it a variant, while a loose file is a family of
//! one. A user file with the id of a bundled one takes its place.
//!
//! A theme link carries a whole theme file in the link itself, so installing one never fetches
//! anything. A link may instead carry a family, an object with the folder's name in `family` and
//! its files by name in `variants`, which installs as a folder. Sonora checks every file as
//! strictly as one in the folder and writes clean copies rebuilt from what it parsed.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context as _, Result};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use ui::{Theme, ThemeKind, ThemeOverrides};

include!(concat!(env!("OUT_DIR"), "/themes.rs"));

const FORMAT_VERSION: u32 = 1;
/// The scheme every link meant for Sonora itself uses.
const SCHEME: &str = "sonora:";
/// What a theme link starts with. The theme file or family follows as unpadded base64url.
const THEME_LINK: &str = "sonora://theme/";
/// The longest theme link read. A full theme file encodes to about 2 KiB.
const LINK_LIMIT: usize = 16 * 1024;
/// How many variants a linked family may have.
const VARIANT_LIMIT: usize = 8;
/// How many characters a linked theme's name or author may have.
const LABEL_LIMIT: usize = 64;
/// How many bytes of a theme's name its file name keeps.
const SLUG_LIMIT: usize = 40;
/// How many numbered names an install tries before it gives up on finding a free one.
const NAME_ATTEMPTS: usize = 100;
/// Names Windows keeps for devices whatever the extension, so a theme file never takes one.
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];
/// The bundled themes in the order the Appearance page lists them. A bundled file missing here
/// sorts after these by id.
const ORDER: [&str; 15] = [
    "catppuccin/latte",
    "catppuccin/frappe",
    "catppuccin/macchiato",
    "catppuccin/mocha",
    "rose-pine/main",
    "rose-pine/moon",
    "rose-pine/dawn",
    "gruvbox/dark",
    "gruvbox/dark-hard",
    "gruvbox/light",
    "gruvbox/light-hard",
    "solarized/dark",
    "solarized/light",
    "nord",
    "dracula",
];

/// The bundled themes, parsed once and sorted by `ORDER`.
static BUNDLED_THEMES: LazyLock<Vec<ThemeVariant>> = LazyLock::new(|| {
    let mut themes = BUNDLED
        .iter()
        .filter_map(|(id, bytes)| {
            let family = id.split_once('/').map_or(*id, |(family, _)| family);
            parse(bytes, id, family)
                .map(|theme| ThemeVariant {
                    id: (*id).to_owned(),
                    bundled: true,
                    ..theme
                })
                .inspect_err(|error| {
                    log::error!("settings: cannot load bundled theme {id}: {error:#}")
                })
                .ok()
        })
        .collect::<Vec<_>>();
    themes.sort_by_key(|theme| {
        let rank = ORDER.iter().position(|id| *id == theme.id);
        (rank.unwrap_or(ORDER.len()), theme.id.clone())
    });
    themes
});

/// One theme file. Its id is the path under the themes folder without the extension, such as
/// `catppuccin/mocha` or `noctalia`, and that id is what the settings store.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeVariant {
    pub id: String,
    /// The id of the family it belongs to, the folder it sits in or its own id when loose.
    pub family: String,
    /// The name inside its family, such as "Mocha". A family of one uses the whole title.
    pub name: String,
    /// The name the file gives, such as "Catppuccin Mocha".
    pub title: String,
    pub author: String,
    pub overrides: ThemeOverrides,
    /// Whether the app ships this file rather than the user's themes folder holding it.
    pub bundled: bool,
}

impl ThemeVariant {
    /// The colours this variant paints with, before rounding, font size, opacity or a cover tint
    /// are applied. Enough to draw a miniature of it.
    pub fn palette(&self) -> Theme {
        Theme::for_kind(ThemeKind::Dark).with_overrides(&self.overrides)
    }

    /// Whether the variant has a light background, which is how a family picks the variant that
    /// matches the system appearance.
    pub fn is_light(&self) -> bool {
        self.palette().background.l >= 0.5
    }
}

/// A theme as the Appearance page offers it: a name over one or more variants. Bundled families
/// come first in a fixed order, then the user's own sorted by name.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeFamily {
    pub id: String,
    /// The words every variant title starts with, or the folder name when they share none.
    pub name: String,
    /// Never empty.
    pub variants: Vec<ThemeVariant>,
    /// Whether the app ships the family. The user may still have replaced or added variants.
    pub bundled: bool,
}

impl ThemeFamily {
    pub fn variant(&self, id: &str) -> Option<&ThemeVariant> {
        self.variants.iter().find(|variant| variant.id == id)
    }
}

/// A theme a link offered, checked and waiting for the user to install it or turn it down. A
/// single theme is a family of one.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeOffer {
    /// The family as it would load, named the way the Appearance page names it. Its ids are the
    /// ones the link asks for, and installing may add a number to them.
    pub family: ThemeFamily,
    /// The file to write for each variant in the same order, rebuilt from the parsed fields
    /// rather than copied from the link.
    files: Vec<Vec<u8>>,
}

impl ThemeOffer {
    /// Whether the offer installs as a folder rather than a loose file.
    fn folder(&self) -> bool {
        self.family
            .variants
            .iter()
            .any(|variant| variant.id.contains('/'))
    }
}

/// A scan of the user's themes folder and whether transient I/O failures merit another scan.
pub(super) struct Loaded {
    pub themes: Vec<ThemeVariant>,
    pub retry: bool,
}

#[derive(Deserialize)]
struct ThemeFile {
    name: String,
    author: String,
    version: u32,
    theme: ThemeOverrides,
}

/// Groups the bundled themes and the user's into families, with a user file replacing the
/// bundled one of the same id.
pub(super) fn catalog(user: &[ThemeVariant]) -> Vec<ThemeFamily> {
    let mut user = user.to_vec();
    user.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut families: Vec<ThemeFamily> = Vec::new();
    for theme in BUNDLED_THEMES.iter().cloned().chain(user) {
        let Some(family) = families.iter_mut().find(|family| family.id == theme.family) else {
            families.push(ThemeFamily {
                id: theme.family.clone(),
                name: String::new(),
                bundled: theme.bundled,
                variants: vec![theme],
            });
            continue;
        };
        match family
            .variants
            .iter_mut()
            .find(|variant| variant.id == theme.id)
        {
            Some(variant) => *variant = theme,
            None => family.variants.push(theme),
        }
    }

    families.iter_mut().for_each(name);
    let user = families.iter().position(|family| !family.bundled);
    if let Some(start) = user {
        families[start..].sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
    }
    families
}

/// Loads every valid theme in `directory`: loose JSON files and JSON files one folder down. A
/// file that fails to read or parse keeps its `previous` value, so a half-written save does not
/// make a theme flicker away.
pub(super) fn load(directory: &Path, previous: &[ThemeVariant]) -> Loaded {
    let kept = |retry| Loaded {
        themes: previous.to_vec(),
        retry,
    };
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => {
            log::warn!(
                "settings: themes path is not a directory: {}",
                directory.display()
            );
            return kept(false);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Loaded {
                themes: Vec::new(),
                retry: false,
            };
        }
        Err(error) => {
            log::warn!("settings: cannot inspect {}: {error}", directory.display());
            return kept(true);
        }
    }
    let Ok((files, folders)) = entries(directory) else {
        return kept(true);
    };

    let mut found = Vec::new();
    for path in files {
        let Some(id) = utf8_stem(&path) else {
            continue;
        };
        if reserved(id) {
            log::warn!(
                "settings: theme {} uses a reserved identifier",
                path.display()
            );
            continue;
        }
        found.push((path.clone(), id.to_owned(), id.to_owned()));
    }

    let mut themes = Vec::new();
    let mut retry = false;
    for folder in folders {
        let Some(family) = folder.file_name().and_then(|name| name.to_str()) else {
            log::warn!("settings: theme folder is not UTF-8: {}", folder.display());
            continue;
        };
        if reserved(family) {
            log::warn!(
                "settings: theme folder {} uses a reserved identifier",
                folder.display()
            );
            continue;
        }
        let Ok((files, _)) = entries(&folder) else {
            retry = true;
            themes.extend(
                previous
                    .iter()
                    .filter(|theme| theme.family == family)
                    .cloned(),
            );
            continue;
        };
        for path in files {
            if let Some(stem) = utf8_stem(&path) {
                found.push((path.clone(), format!("{family}/{stem}"), family.to_owned()));
            }
        }
    }

    for (path, id, family) in found {
        let earlier = || previous.iter().find(|theme| theme.id == id).cloned();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("settings: cannot read theme {}: {error}", path.display());
                retry = true;
                themes.extend(earlier());
                continue;
            }
        };
        match parse(&bytes, &path.display().to_string(), &family) {
            Ok(theme) => themes.push(ThemeVariant { id, ..theme }),
            Err(error) => {
                log::warn!("settings: cannot load theme {}: {error:#}", path.display());
                themes.extend(earlier());
            }
        }
    }
    Loaded { themes, retry }
}

/// Writes the backup of a removed built-in theme into `directory` when `id` names one and no
/// theme file of that id is there yet, so whoever wore it keeps it. Returns whether it wrote.
pub(super) fn adopt_legacy(id: &str, directory: &Path) -> bool {
    let Some((_, bytes)) = LEGACY.iter().find(|(name, _)| *name == id) else {
        return false;
    };
    let path = directory.join(format!("{id}.json"));
    if fs::symlink_metadata(&path).is_ok() {
        return false;
    }
    if let Err(error) = fs::create_dir_all(directory).and_then(|()| fs::write(&path, bytes)) {
        log::warn!("settings: cannot write {}: {error}", path.display());
        return false;
    }
    log::info!("settings: moved legacy theme {id} into the themes folder");
    true
}

/// Whether `text` is a link in Sonora's own scheme, which no provider reads and which is never
/// a file to play.
pub fn is_own_link(text: &str) -> bool {
    text.trim()
        .get(..SCHEME.len())
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
}

/// Reads the theme a `sonora://theme/` link carries. The link has to be short, its payload pure
/// base64url, and the file inside has to pass the same checks as one in the themes folder. A
/// name or author with control or direction characters is refused, since both are shown as
/// they are.
pub fn read_link(text: &str) -> Result<ThemeOffer> {
    let text = text.trim();
    anyhow::ensure!(text.len() <= LINK_LIMIT, "theme link is too long");
    let payload = text
        .get(..THEME_LINK.len())
        .filter(|prefix| prefix.eq_ignore_ascii_case(THEME_LINK))
        .map(|_| &text[THEME_LINK.len()..])
        .context("not a theme link")?;
    // a browser on Windows may hand the link over with a slash added
    let payload = payload.strip_suffix('/').unwrap_or(payload);
    let payload = payload.trim_end_matches('=');
    anyhow::ensure!(
        !payload.is_empty()
            && payload
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "theme link is not base64url"
    );
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .context("cannot decode the theme link")?;
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    let value: serde_json::Value =
        serde_json::from_slice(bytes).context("cannot parse the theme link")?;

    let (mut family, files) = match value.get("variants") {
        Some(_) => read_family(&value)?,
        None => {
            let (variant, file) = clean(&value)?;
            let id = slug(&variant.title);
            let variant = ThemeVariant {
                id: id.clone(),
                family: id.clone(),
                ..variant
            };
            let family = ThemeFamily {
                id,
                name: String::new(),
                variants: vec![variant],
                bundled: false,
            };
            (family, vec![file])
        }
    };
    name(&mut family);
    Ok(ThemeOffer { family, files })
}

/// Reads the family object of a theme link into the family and its files, the variants sorted
/// by file name the way the themes folder lists them. The folder and file names have to be what
/// `slug` makes of them, so a link never names anything on disk Sonora would not name itself.
fn read_family(value: &serde_json::Value) -> Result<(ThemeFamily, Vec<Vec<u8>>)> {
    let object = value
        .as_object()
        .context("theme link must contain an object")?;
    for field in object.keys() {
        anyhow::ensure!(
            matches!(field.as_str(), "family" | "variants"),
            "unknown theme family field `{field}`"
        );
    }
    let id = object
        .get("family")
        .and_then(serde_json::Value::as_str)
        .context("family must be a string")?;
    anyhow::ensure!(
        slug(id) == id && !reserved(id),
        "family `{id}` is not a usable folder name"
    );
    let variants = object
        .get("variants")
        .and_then(serde_json::Value::as_object)
        .context("variants must contain an object")?;
    anyhow::ensure!(
        (2..=VARIANT_LIMIT).contains(&variants.len()),
        "a theme family has 2 to {VARIANT_LIMIT} variants, not {}",
        variants.len()
    );

    let mut entries = variants.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(stem, _)| format!("{stem}.json"));
    let mut family = ThemeFamily {
        id: id.to_owned(),
        name: String::new(),
        variants: Vec::new(),
        bundled: false,
    };
    let mut files = Vec::new();
    for (stem, value) in entries {
        anyhow::ensure!(
            slug(stem) == *stem,
            "variant `{stem}` is not a usable file name"
        );
        let (variant, file) = clean(value)?;
        family.variants.push(ThemeVariant {
            id: format!("{id}/{stem}"),
            family: id.to_owned(),
            ..variant
        });
        files.push(file);
    }
    Ok((family, files))
}

/// Checks one theme file a link carries and rebuilds it from the fields that parsed. A name or
/// author with control or direction characters is refused, since both are shown as they are.
/// The variant comes back without ids.
fn clean(value: &serde_json::Value) -> Result<(ThemeVariant, Vec<u8>)> {
    let bytes = serde_json::to_vec(value).context("cannot encode the theme")?;
    let variant = parse(&bytes, "theme link", "")?;
    for (field, value) in [("name", &variant.title), ("author", &variant.author)] {
        anyhow::ensure!(
            value.chars().count() <= LABEL_LIMIT,
            "theme {field} is too long"
        );
        anyhow::ensure!(
            !value.chars().any(hidden),
            "theme {field} has control characters"
        );
    }

    let colors = value
        .get("theme")
        .and_then(serde_json::Value::as_object)
        .context("theme must contain an object")?
        .iter()
        .filter(|(_, color)| color.is_string())
        .map(|(field, color)| (field.clone(), color.clone()))
        .collect::<serde_json::Map<_, _>>();
    let file = serde_json::to_vec_pretty(&serde_json::json!({
        "name": variant.title,
        "author": variant.author,
        "version": FORMAT_VERSION,
        "theme": colors,
    }))
    .context("cannot encode the theme")?;
    let variant = parse(&file, "theme link", "")?;
    Ok((variant, file))
}

/// Writes `offer` into `directory` and returns the id of the family it became. A single theme
/// becomes a loose file and a family a folder, under the id the offer asks for. It replaces an
/// installed family only when that one has the same name and authors, and then deletes the
/// variants the offer no longer has. Otherwise it takes the first numbered id that no bundled
/// theme, reserved id or existing file or folder holds.
pub(super) fn install(
    offer: &ThemeOffer,
    directory: &Path,
    installed: &[ThemeVariant],
) -> Result<String> {
    fs::create_dir_all(directory).context("cannot create the themes folder")?;
    let base = &offer.family.id;
    let folder = offer.folder();
    let taken = |path: &Path| fs::symlink_metadata(path).is_ok();
    for attempt in 1..=NAME_ATTEMPTS {
        let id = match attempt {
            1 => base.clone(),
            attempt => format!("{base}-{attempt}"),
        };
        let bundled = BUNDLED_THEMES
            .iter()
            .any(|theme| theme.id == id || theme.family == id);
        let (path, other) = match folder {
            true => (directory.join(&id), directory.join(format!("{id}.json"))),
            false => (directory.join(format!("{id}.json")), directory.join(&id)),
        };
        if bundled || reserved(&id) || taken(&other) {
            continue;
        }
        let earlier = installed
            .iter()
            .filter(|theme| theme.family == id)
            .cloned()
            .collect::<Vec<_>>();
        if taken(&path) && !same(&id, earlier.clone(), &offer.family) {
            continue;
        }

        if !folder {
            write(directory, &path, &offer.files[0])?;
            return Ok(id);
        }
        fs::create_dir_all(&path).with_context(|| format!("cannot create {}", path.display()))?;
        let stems = offer
            .family
            .variants
            .iter()
            .map(|variant| stem(&variant.id))
            .collect::<Vec<_>>();
        for (stem, file) in stems.iter().zip(&offer.files) {
            write(&path, &path.join(format!("{stem}.json")), file)?;
        }
        for variant in earlier
            .iter()
            .filter(|variant| !stems.contains(&stem(&variant.id)))
        {
            let stale = directory.join(format!("{}.json", variant.id));
            if let Err(error) = fs::remove_file(&stale) {
                log::warn!("settings: cannot delete {}: {error}", stale.display());
            }
        }
        return Ok(id);
    }
    anyhow::bail!("cannot find a free name for theme {base}")
}

/// Whether `earlier`, the installed variants of family `id`, are an older copy of `offer`: the
/// same family name from the same authors.
fn same(id: &str, earlier: Vec<ThemeVariant>, offer: &ThemeFamily) -> bool {
    if earlier.is_empty() {
        return false;
    }
    let mut family = ThemeFamily {
        id: id.to_owned(),
        name: String::new(),
        variants: earlier,
        bundled: false,
    };
    name(&mut family);
    let authors = |family: &ThemeFamily| {
        family
            .variants
            .iter()
            .map(|variant| variant.author.clone())
            .collect::<BTreeSet<_>>()
    };
    family.name == offer.name && authors(&family) == authors(offer)
}

/// The file name of variant `id` inside its folder, or the whole id for a loose file.
fn stem(id: &str) -> &str {
    id.rsplit_once('/').map_or(id, |(_, stem)| stem)
}

/// Deletes every file of the user's `family` from `directory`, then its folder once nothing else
/// is left in it. A file already gone counts as deleted. Bundled families are refused.
pub(super) fn remove(family: &ThemeFamily, directory: &Path) -> Result<()> {
    anyhow::ensure!(!family.bundled, "cannot delete bundled theme {}", family.id);
    for variant in &family.variants {
        let path = directory.join(format!("{}.json", variant.id));
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot delete {}", path.display()));
            }
        }
    }
    // only the variants of a folder carry a slash in their id, and a folder that still holds
    // anything else stays
    if family
        .variants
        .iter()
        .any(|variant| variant.id.contains('/'))
    {
        fs::remove_dir(directory.join(&family.id)).ok();
    }
    Ok(())
}

/// Parses one theme file. The id it returns is `family`, which a caller inside a folder
/// replaces with the full one. `origin` names the file in errors.
fn parse(bytes: &[u8], origin: &str, family: &str) -> Result<ThemeVariant> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let value: serde_json::Value =
        serde_json::from_slice(bytes).with_context(|| format!("cannot parse {origin}"))?;
    let object = value
        .as_object()
        .context("theme file must contain an object")?;
    for field in object.keys() {
        anyhow::ensure!(
            matches!(field.as_str(), "name" | "author" | "version" | "theme"),
            "unknown theme file field `{field}`"
        );
    }
    let theme = object
        .get("theme")
        .and_then(serde_json::Value::as_object)
        .context("theme must contain an object")?;
    for field in theme.keys() {
        anyhow::ensure!(
            ThemeOverrides::is_color(field),
            "`{field}` is not a theme color"
        );
    }
    let file: ThemeFile =
        serde_json::from_value(value).with_context(|| format!("cannot decode {origin}"))?;
    anyhow::ensure!(
        file.version == FORMAT_VERSION,
        "unsupported theme format version {}",
        file.version
    );
    let title = file.name.trim();
    anyhow::ensure!(!title.is_empty(), "theme name is empty");
    let author = file.author.trim();
    anyhow::ensure!(!author.is_empty(), "theme author is empty");
    if let Some(field) = file.theme.invalid_color() {
        anyhow::bail!("theme.{field} is not a valid color");
    }
    Ok(ThemeVariant {
        id: family.to_owned(),
        family: family.to_owned(),
        name: title.to_owned(),
        title: title.to_owned(),
        author: author.to_owned(),
        overrides: file.theme,
        bundled: false,
    })
}

/// The JSON files and the folders directly inside `directory`, each sorted. Symlinks are
/// skipped. Fails on the first entry that cannot be read, after logging it.
fn entries(directory: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), ()> {
    let failed = |error: std::io::Error| {
        log::warn!("settings: cannot read {}: {error}", directory.display());
    };
    let mut files = Vec::new();
    let mut folders = Vec::new();
    for entry in fs::read_dir(directory).map_err(failed)? {
        let entry = entry.map_err(failed)?;
        let path = entry.path();
        let kind = entry.file_type().map_err(failed)?;
        if kind.is_dir() {
            folders.push(path);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            files.push(path);
        }
    }
    files.sort();
    folders.sort();
    Ok((files, folders))
}

fn utf8_stem(path: &Path) -> Option<&str> {
    let stem = path.file_stem().and_then(|stem| stem.to_str());
    if stem.is_none() {
        log::warn!("settings: theme filename is not UTF-8: {}", path.display());
    }
    stem
}

/// Whether `id` is taken by a built-in theme, which a theme file cannot replace.
fn reserved(id: &str) -> bool {
    ThemeKind::ALL.into_iter().any(|kind| kind.id() == id)
}

/// Names a family after the words all its variant titles share and each variant after the rest
/// of its title. A family of one, or one whose titles share nothing, keeps the whole titles.
fn name(family: &mut ThemeFamily) {
    let titles = family
        .variants
        .iter()
        .map(|variant| {
            variant
                .title
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let shared = match titles.as_slice() {
        [] | [_] => 0,
        [first, rest @ ..] => (0..first.len())
            .take_while(|&index| {
                rest.iter()
                    .all(|title| title.get(index) == first.get(index))
            })
            .count(),
    };

    family.name = match (shared, titles.as_slice()) {
        (_, [only]) => only.join(" "),
        (0, _) => family.id.clone(),
        (shared, [first, ..]) => first[..shared].join(" "),
        (_, []) => family.id.clone(),
    };
    for (variant, words) in family.variants.iter_mut().zip(&titles) {
        variant.name = match words.get(shared..).filter(|rest| !rest.is_empty()) {
            Some(rest) if titles.len() > 1 => rest.join(" "),
            _ => variant.title.clone(),
        };
    }
}

/// Whether `character` would act on the text around it rather than show: a control, a
/// direction mark or override, or an invisible joiner.
fn hidden(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{FEFF}'
        )
}

/// A file name for a theme called `title`: its ASCII letters and digits lowercased, every other
/// run of characters made one hyphen. A name with none of those, or one Windows keeps for a
/// device, becomes `theme` or gains a `-theme` suffix.
fn slug(title: &str) -> String {
    let mut slug = String::new();
    for character in title.chars().flat_map(char::to_lowercase) {
        match character.is_ascii_alphanumeric() {
            true => slug.push(character),
            false if !slug.is_empty() && !slug.ends_with('-') => slug.push('-'),
            false => {}
        }
    }
    slug.truncate(SLUG_LIMIT);
    let slug = slug.trim_matches('-');
    match slug {
        "" => "theme".to_owned(),
        slug if DEVICE_NAMES.contains(&slug) => format!("{slug}-theme"),
        slug => slug.to_owned(),
    }
}

/// Writes `bytes` to `path` through a hidden file beside it, so the folder watcher never reads
/// half a theme.
fn write(directory: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("theme path has no file name")?;
    let partial = directory.join(format!(".{name}.part"));
    fs::write(&partial, bytes).with_context(|| format!("cannot write {}", partial.display()))?;
    fs::rename(&partial, path).map_err(|error| {
        fs::remove_file(&partial).ok();
        anyhow::Error::new(error).context(format!("cannot write {}", path.display()))
    })
}
