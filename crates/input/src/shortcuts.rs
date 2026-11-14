//! The shortcuts a user can bind to keys of their own under Settings, Keybinds. Each one names
//! an action, the keys it has out of the box and where it listens. The user's choices are kept
//! as a map from shortcut id to keystrokes, which [`bindings`] turns into key bindings.

use std::collections::BTreeMap;

use gpui::{Action, KeyBinding, Keystroke};
use ui::{FORM_CONTEXT, INPUT_CONTEXT, MENU_CONTEXT, TABLE_CONTEXT};

use crate::{
    DecreaseFontSize, IncreaseFontSize, NavigateBack, NavigateForward, OpenFilter, OpenSearch,
    OpenSettings, POWERBAR_CONTEXT, Refresh, ResetFontSize, SEARCH_CONTEXT, SeekBack, SeekForward,
    SongNext, SongPrevious, ToggleFullscreen, ToggleLyrics, TogglePlayback, TogglePowerbar,
    ToggleQueue, ToggleRepeat, ToggleShuffle, ToggleWindowFullscreen, VolumeDown, VolumeUp,
    WORKSPACE_CONTEXT,
};

/// The modifiers that make a keystroke a command rather than typing. A key pressed without any
/// of them would type into a field, so it only works away from text.
const COMMAND_MODIFIERS: [&str; 6] = ["ctrl", "alt", "cmd", "super", "win", "fn"];

/// Builds the binding of one keystroke to a shortcut's action in a context.
type Bind = fn(&str, Option<&str>) -> KeyBinding;

/// The group a shortcut is listed under in settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Playback,
    Navigation,
    View,
}

impl Section {
    pub const ALL: [Self; 3] = [Self::Playback, Self::Navigation, Self::View];

    /// The Fluent key of the group's heading.
    pub fn key(self) -> &'static str {
        match self {
            Self::Playback => "keybinds-group-playback",
            Self::Navigation => "keybinds-group-navigation",
            Self::View => "keybinds-group-view",
        }
    }

    /// The icon before the group's heading.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Playback => "icons/play.svg",
            Self::Navigation => "icons/arrow-right-left.svg",
            Self::View => "icons/monitor.svg",
        }
    }
}

/// Where a shortcut's keys are heard.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// Anywhere in the app, while typing in a field too.
    Anywhere,
    /// Anywhere in the workspace except a text field, for keys a field would take as typing.
    AwayFromText,
    /// Only where no text field, list, menu, form, search or powerbar has the keys, so the page
    /// keeps its own use of arrow keys. Keys a list takes for moving through rows stay with it.
    Browsing,
}

/// One action the user can bind to keys of their own.
pub struct Shortcut {
    /// The id the user's choice is saved under in `settings.json`.
    pub id: &'static str,
    /// The Fluent key of what the shortcut does.
    pub key: &'static str,
    pub section: Section,
    scope: Scope,
    bind: Bind,
    /// The keys it has out of the box on every platform.
    defaults: &'static [&'static str],
    /// The keys it has out of the box on macOS only, after `defaults`.
    mac: &'static [&'static str],
}

impl Shortcut {
    /// The keys the shortcut has out of the box on this platform.
    pub fn defaults(&self) -> Vec<&'static str> {
        let mut keys = self.defaults.to_vec();
        if cfg!(target_os = "macos") {
            keys.extend(self.mac);
        }
        keys
    }

    /// The keys the shortcut answers to now, the user's own when they chose any.
    pub fn keys(&self, overrides: &BTreeMap<String, Vec<String>>) -> Vec<String> {
        match overrides.get(self.id) {
            Some(keys) => keys.clone(),
            None => self.defaults().into_iter().map(str::to_owned).collect(),
        }
    }

    /// Whether the user changed the shortcut's keys from its defaults.
    pub fn changed(&self, overrides: &BTreeMap<String, Vec<String>>) -> bool {
        overrides.contains_key(self.id)
    }

    /// The context `keystroke` is bound in. A key with no command modifier only works away from
    /// text whatever the shortcut's scope, so binding a letter never stops it from typing.
    fn context(&self, keystroke: &str) -> Option<String> {
        let plain = !keystroke
            .split('-')
            .any(|part| COMMAND_MODIFIERS.contains(&part));
        match (self.scope, plain) {
            (Scope::Anywhere, false) => None,
            (Scope::Browsing, _) => Some(browsing_context()),
            _ => Some(format!("{WORKSPACE_CONTEXT} && !{INPUT_CONTEXT}")),
        }
    }
}

const SHORTCUTS: &[Shortcut] = &[
    Shortcut {
        id: "toggle-playback",
        key: "keybind-toggle-playback",
        section: Section::Playback,
        scope: Scope::AwayFromText,
        bind: bind::<TogglePlayback>,
        defaults: &["space"],
        mac: &[],
    },
    Shortcut {
        id: "song-previous",
        key: "keybind-song-previous",
        section: Section::Playback,
        scope: Scope::AwayFromText,
        bind: bind::<SongPrevious>,
        defaults: &["ctrl-left"],
        mac: &[],
    },
    Shortcut {
        id: "song-next",
        key: "keybind-song-next",
        section: Section::Playback,
        scope: Scope::AwayFromText,
        bind: bind::<SongNext>,
        defaults: &["ctrl-right"],
        mac: &[],
    },
    Shortcut {
        id: "volume-up",
        key: "keybind-volume-up",
        section: Section::Playback,
        scope: Scope::Browsing,
        bind: bind::<VolumeUp>,
        defaults: &["up"],
        mac: &[],
    },
    Shortcut {
        id: "volume-down",
        key: "keybind-volume-down",
        section: Section::Playback,
        scope: Scope::Browsing,
        bind: bind::<VolumeDown>,
        defaults: &["down"],
        mac: &[],
    },
    Shortcut {
        id: "seek-back",
        key: "keybind-seek-back",
        section: Section::Playback,
        scope: Scope::Browsing,
        bind: bind::<SeekBack>,
        defaults: &["left"],
        mac: &[],
    },
    Shortcut {
        id: "seek-forward",
        key: "keybind-seek-forward",
        section: Section::Playback,
        scope: Scope::Browsing,
        bind: bind::<SeekForward>,
        defaults: &["right"],
        mac: &[],
    },
    Shortcut {
        id: "toggle-shuffle",
        key: "keybind-toggle-shuffle",
        section: Section::Playback,
        scope: Scope::Anywhere,
        bind: bind::<ToggleShuffle>,
        defaults: &[],
        mac: &[],
    },
    Shortcut {
        id: "toggle-repeat",
        key: "keybind-toggle-repeat",
        section: Section::Playback,
        scope: Scope::Anywhere,
        bind: bind::<ToggleRepeat>,
        defaults: &[],
        mac: &[],
    },
    Shortcut {
        id: "navigate-back",
        key: "keybind-navigate-back",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<NavigateBack>,
        defaults: &["alt-left"],
        mac: &["cmd-["],
    },
    Shortcut {
        id: "navigate-forward",
        key: "keybind-navigate-forward",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<NavigateForward>,
        defaults: &["alt-right"],
        mac: &["cmd-]"],
    },
    Shortcut {
        id: "open-search",
        key: "keybind-open-search",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<OpenSearch>,
        defaults: &["cmd-f", "ctrl-f"],
        mac: &[],
    },
    Shortcut {
        id: "toggle-powerbar",
        key: "keybind-toggle-powerbar",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<TogglePowerbar>,
        defaults: &["cmd-k", "ctrl-k"],
        mac: &[],
    },
    Shortcut {
        id: "open-filter",
        key: "keybind-open-filter",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<OpenFilter>,
        defaults: &["/"],
        mac: &[],
    },
    Shortcut {
        id: "open-settings",
        key: "keybind-open-settings",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<OpenSettings>,
        defaults: &["ctrl-,", "cmd-,"],
        mac: &[],
    },
    Shortcut {
        id: "refresh",
        key: "keybind-refresh",
        section: Section::Navigation,
        scope: Scope::Anywhere,
        bind: bind::<Refresh>,
        defaults: &["cmd-r", "ctrl-r"],
        mac: &[],
    },
    Shortcut {
        id: "toggle-fullscreen",
        key: "keybind-toggle-fullscreen",
        section: Section::View,
        scope: Scope::AwayFromText,
        bind: bind::<ToggleFullscreen>,
        defaults: &["f"],
        mac: &[],
    },
    Shortcut {
        id: "toggle-queue",
        key: "keybind-toggle-queue",
        section: Section::View,
        scope: Scope::Anywhere,
        bind: bind::<ToggleQueue>,
        defaults: &[],
        mac: &[],
    },
    Shortcut {
        id: "toggle-lyrics",
        key: "keybind-toggle-lyrics",
        section: Section::View,
        scope: Scope::Anywhere,
        bind: bind::<ToggleLyrics>,
        defaults: &[],
        mac: &[],
    },
    Shortcut {
        id: "toggle-window-fullscreen",
        key: "keybind-toggle-window-fullscreen",
        section: Section::View,
        scope: Scope::Anywhere,
        bind: bind::<ToggleWindowFullscreen>,
        defaults: &["f11"],
        mac: &["ctrl-cmd-f"],
    },
    Shortcut {
        id: "increase-font-size",
        key: "keybind-increase-font-size",
        section: Section::View,
        scope: Scope::Anywhere,
        bind: bind::<IncreaseFontSize>,
        defaults: &["cmd-=", "ctrl-=", "cmd-+", "ctrl-+"],
        mac: &[],
    },
    Shortcut {
        id: "decrease-font-size",
        key: "keybind-decrease-font-size",
        section: Section::View,
        scope: Scope::Anywhere,
        bind: bind::<DecreaseFontSize>,
        defaults: &["cmd--", "ctrl--"],
        mac: &[],
    },
    Shortcut {
        id: "reset-font-size",
        key: "keybind-reset-font-size",
        section: Section::View,
        scope: Scope::Anywhere,
        bind: bind::<ResetFontSize>,
        defaults: &["cmd-0", "ctrl-0"],
        mac: &[],
    },
];

/// Every shortcut, in the order settings lists them.
pub fn shortcuts() -> impl Iterator<Item = &'static Shortcut> {
    SHORTCUTS.iter()
}

/// The shortcut saved under `id`.
pub fn shortcut(id: &str) -> Option<&'static Shortcut> {
    shortcuts().find(|shortcut| shortcut.id == id)
}

/// The keystroke to show for the shortcut `id`: the first of the user's keys, or the default
/// this platform's users know best, which on macOS is the one with ⌘ and elsewhere the one
/// without the Super key. `None` when the shortcut has no key.
pub fn shown(id: &str, overrides: &BTreeMap<String, Vec<String>>) -> Option<String> {
    let shortcut = shortcut(id)?;
    if let Some(keys) = overrides.get(id) {
        return keys.first().cloned();
    }
    let defaults = shortcut.defaults();
    let mac = cfg!(target_os = "macos");
    defaults
        .iter()
        .find(|key| key.split('-').any(|part| part == "cmd") == mac)
        .or_else(|| defaults.first())
        .map(|key| (*key).to_owned())
}

/// The bindings of every shortcut, each to the user's keys or to its defaults. A saved key that
/// does not parse, from a hand edit of `settings.json`, is skipped.
pub(crate) fn bindings(overrides: &BTreeMap<String, Vec<String>>) -> Vec<KeyBinding> {
    shortcuts()
        .flat_map(|shortcut| {
            shortcut
                .keys(overrides)
                .into_iter()
                .filter(|key| parse(key).is_some())
                .map(|key| (shortcut.bind)(&key, shortcut.context(&key).as_deref()))
        })
        .collect()
}

/// The keystroke a key press spells, in the form bindings and settings store, or `None` for a
/// press of a modifier on its own.
pub fn keystroke_of(keystroke: &Keystroke) -> Option<String> {
    let modifier = matches!(
        keystroke.key.as_str(),
        "shift" | "control" | "ctrl" | "alt" | "platform" | "cmd" | "super" | "win" | "function"
    );
    (!modifier && !keystroke.key.is_empty()).then(|| keystroke.unparse())
}

/// Gives the shortcut `id` the key `keystroke` alone, and takes that key from every other
/// shortcut that had it, so one key never does two things. Returns the shortcuts it was taken
/// from. A shortcut whose keys come back to its defaults drops out of `overrides`.
pub fn assign(
    overrides: &mut BTreeMap<String, Vec<String>>,
    id: &str,
    keystroke: &str,
) -> Vec<&'static Shortcut> {
    let taken = take(overrides, id, &[keystroke]);
    overrides.insert(id.to_owned(), vec![keystroke.to_owned()]);
    settle(overrides);
    taken
}

/// Sets the shortcut `id` back to its defaults, taking them from any other shortcut the user
/// gave one of them to since. Returns the shortcuts they were taken from.
pub fn reset(overrides: &mut BTreeMap<String, Vec<String>>, id: &str) -> Vec<&'static Shortcut> {
    let defaults = shortcut(id).map(Shortcut::defaults).unwrap_or_default();
    let taken = take(overrides, id, &defaults);
    overrides.remove(id);
    settle(overrides);
    taken
}

/// Takes every one of `keystrokes` from each shortcut but `id` that answers to it, and returns
/// those shortcuts.
fn take(
    overrides: &mut BTreeMap<String, Vec<String>>,
    id: &str,
    keystrokes: &[&str],
) -> Vec<&'static Shortcut> {
    let wanted = |key: &String| keystrokes.iter().any(|keystroke| same(key, keystroke));
    let mut taken = Vec::new();
    for other in shortcuts().filter(|other| other.id != id) {
        let keys = other.keys(overrides);
        if keys.iter().any(wanted) {
            let left = keys.into_iter().filter(|key| !wanted(key)).collect();
            overrides.insert(other.id.to_owned(), left);
            taken.push(other);
        }
    }
    taken
}

/// Takes every key from the shortcut `id`, so it does nothing until bound again.
pub fn clear(overrides: &mut BTreeMap<String, Vec<String>>, id: &str) {
    overrides.insert(id.to_owned(), Vec::new());
    settle(overrides);
}

/// Drops every choice that matches its shortcut's defaults again, and any for a shortcut that
/// no longer exists.
fn settle(overrides: &mut BTreeMap<String, Vec<String>>) {
    overrides.retain(|id, keys| {
        let Some(shortcut) = shortcut(id) else {
            return false;
        };
        let defaults = shortcut.defaults();
        let matches = keys.len() == defaults.len()
            && keys
                .iter()
                .all(|key| defaults.iter().any(|default| same(key, default)));
        !matches
    });
}

/// Whether two spelled keystrokes press the same keys, so `secondary-q` and `ctrl-q` match off
/// macOS.
fn same(left: &str, right: &str) -> bool {
    match (parse(left), parse(right)) {
        (Some(left), Some(right)) => left.modifiers == right.modifiers && left.key == right.key,
        _ => left == right,
    }
}

/// The keystroke `key` spells, or `None` when it is not one. A chord of several keystrokes is
/// not a shortcut settings can record, so it is refused too.
fn parse(key: &str) -> Option<Keystroke> {
    match key.contains(' ') {
        true => None,
        false => Keystroke::parse(key).ok(),
    }
}

/// Binds `keystroke` to a fresh `A` in `context`.
fn bind<A: Action + Default>(keystroke: &str, context: Option<&str>) -> KeyBinding {
    KeyBinding::new(keystroke, A::default(), context)
}

/// The context of a [`Scope::Browsing`] shortcut: the workspace with no text field, list, menu,
/// form, search or powerbar in the focus path.
fn browsing_context() -> String {
    format!(
        "{WORKSPACE_CONTEXT} && !{INPUT_CONTEXT} && !{MENU_CONTEXT} && !{FORM_CONTEXT} && !{TABLE_CONTEXT} && !{SEARCH_CONTEXT} && !{POWERBAR_CONTEXT}"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gpui::KeyContext;

    use super::*;

    /// The binding the default keymap gives action `A`.
    fn binding_of<A: Action>() -> KeyBinding {
        crate::bindings(&BTreeMap::new())
            .into_iter()
            .find(|binding| binding.action().as_any().is::<A>())
            .expect("the action has a default binding")
    }

    /// A context stack with one entry per name, as the focus path of a window gives it.
    fn stack(names: &[&str]) -> Vec<KeyContext> {
        names
            .iter()
            .map(|name| KeyContext::parse(name).expect("a valid context name"))
            .collect()
    }

    #[test]
    fn arrow_keys_default_to_volume_and_seek() {
        assert_eq!(
            shortcut("volume-up").map(Shortcut::defaults),
            Some(vec!["up"])
        );
        assert_eq!(
            shortcut("volume-down").map(Shortcut::defaults),
            Some(vec!["down"])
        );
        assert_eq!(
            shortcut("seek-back").map(Shortcut::defaults),
            Some(vec!["left"])
        );
        assert_eq!(
            shortcut("seek-forward").map(Shortcut::defaults),
            Some(vec!["right"])
        );
    }

    #[test]
    fn arrow_shortcuts_are_listed_under_playback() {
        for id in ["volume-up", "volume-down", "seek-back", "seek-forward"] {
            assert_eq!(
                shortcut(id).map(|shortcut| shortcut.section),
                Some(Section::Playback)
            );
        }
    }

    #[test]
    fn arrow_keys_do_nothing_while_a_text_field_has_focus() {
        let predicates = [
            binding_of::<VolumeUp>(),
            binding_of::<VolumeDown>(),
            binding_of::<SeekBack>(),
            binding_of::<SeekForward>(),
        ];
        for binding in &predicates {
            let predicate = binding.predicate().expect("bound in a context");
            assert!(!predicate.eval(&stack(&["Workspace", INPUT_CONTEXT])));
            assert!(!predicate.eval(&stack(&["Workspace", "Page", INPUT_CONTEXT])));
        }
    }

    #[test]
    fn arrow_keys_work_on_the_bare_workspace() {
        let predicates = [
            binding_of::<VolumeUp>(),
            binding_of::<VolumeDown>(),
            binding_of::<SeekBack>(),
            binding_of::<SeekForward>(),
        ];
        for binding in &predicates {
            let predicate = binding.predicate().expect("bound in a context");
            assert!(predicate.eval(&stack(&["Workspace"])));
        }
    }

    #[test]
    fn arrow_keys_yield_to_lists_menus_forms_search_and_the_powerbar() {
        let predicates = [
            binding_of::<VolumeUp>(),
            binding_of::<VolumeDown>(),
            binding_of::<SeekBack>(),
            binding_of::<SeekForward>(),
        ];
        let owners = [
            TABLE_CONTEXT,
            MENU_CONTEXT,
            FORM_CONTEXT,
            SEARCH_CONTEXT,
            POWERBAR_CONTEXT,
        ];
        for binding in &predicates {
            let predicate = binding.predicate().expect("bound in a context");
            for owner in owners {
                assert!(!predicate.eval(&stack(&["Workspace", owner])));
            }
        }
    }
}
