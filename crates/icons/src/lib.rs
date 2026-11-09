use std::sync::atomic::{AtomicUsize, Ordering};

pub const BASE: &str = "lucide";
pub const SAMPLES: &[&str] = &["house", "heart-filled", "play-filled", "shuffle", "search"];

const SHARED: &str = "common";
const KIND: &str = ".svg";

include!(concat!(env!("OUT_DIR"), "/packs.rs"));

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// One icon of a pack: its file's bytes and the same drawing as Rune paints it.
pub struct Icon {
    pub name: &'static str,
    pub path: &'static str,
    pub bytes: &'static [u8],
    pub drawn: rune::Icon,
}

pub struct Pack {
    pub id: &'static str,
    start: usize,
    end: usize,
}

impl Pack {
    pub fn title(&self) -> String {
        let mut letters = self.id.chars();
        match letters.next() {
            Some(first) => first.to_uppercase().chain(letters).collect(),
            None => String::new(),
        }
    }

    pub fn icons(&self) -> &'static [Icon] {
        &ICONS[self.start..self.end]
    }

    pub fn icon(&self, name: &str) -> Option<&'static Icon> {
        let icons = self.icons();
        let at = icons.binary_search_by(|icon| icon.name.cmp(name)).ok()?;
        icons.get(at)
    }
}

pub fn packs() -> impl Iterator<Item = &'static Pack> {
    PACKS.iter().filter(|pack| pack.id != SHARED)
}

pub fn pack(id: &str) -> Option<&'static Pack> {
    packs().find(|pack| pack.id == id)
}

pub fn active() -> &'static Pack {
    let at = ACTIVE.load(Ordering::Relaxed);
    PACKS.get(at).unwrap_or(&PACKS[0])
}

pub fn set(id: &str) {
    let chosen = PACKS
        .iter()
        .position(|pack| pack.id == id && pack.id != SHARED)
        .or_else(|| PACKS.iter().position(|pack| pack.id == BASE))
        .unwrap_or_default();

    ACTIVE.store(chosen, Ordering::Relaxed);
}

/// The icon a call site names (`"icons/play.svg"` or `"play"`) in the active pack, falling back
/// to the shared brand icons and then the base pack. Rune draws the returned icon.
pub fn get(icon: &str) -> &'static rune::Icon {
    &icon_in(active(), icon).drawn
}

/// [`get`] for a chosen pack, as the pack picker previews them.
pub fn get_in(pack: &Pack, icon: &str) -> &'static rune::Icon {
    &icon_in(pack, icon).drawn
}

pub fn path(icon: impl AsRef<str>) -> &'static str {
    shown(active(), icon)
}

pub fn shown(pack: &Pack, icon: impl AsRef<str>) -> &'static str {
    icon_in(pack, icon.as_ref()).path
}

pub fn asset(path: &str) -> Option<&'static [u8]> {
    let (id, file) = path.strip_prefix("icons/")?.split_once('/')?;
    let folder = PACKS.iter().find(|pack| pack.id == id)?;
    folder.icon(stem(file)).map(|icon| icon.bytes)
}

fn icon_in(pack: &Pack, icon: &str) -> &'static Icon {
    let name = stem(icon);
    chased(pack, name).unwrap_or_else(|| {
        log::warn!("icons: {name} is in no pack");
        chased(pack, "circle-help").unwrap_or(&ICONS[0])
    })
}

fn chased(pack: &Pack, name: &str) -> Option<&'static Icon> {
    pack.icon(name)
        .or_else(|| folder(SHARED)?.icon(name))
        .or_else(|| folder(BASE)?.icon(name))
}

fn folder(id: &str) -> Option<&'static Pack> {
    PACKS.iter().find(|pack| pack.id == id)
}

fn stem(icon: &str) -> &str {
    let name = icon.rsplit('/').next().unwrap_or(icon);
    name.strip_suffix(KIND).unwrap_or(name)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_pack_has_its_own_pin() {
        for pack in super::packs() {
            assert!(pack.icon("pin").is_some(), "{} borrows its pin", pack.id);
            let path = super::shown(pack, "icons/pin.svg");
            assert!(
                super::asset(path).is_some(),
                "pin missing from {}",
                pack.id
            );
        }
    }
}
