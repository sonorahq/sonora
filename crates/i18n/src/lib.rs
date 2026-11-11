mod language;

use std::borrow::Cow;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentResource, FluentValue};
use gpui::SharedString;

pub use fluent_bundle::FluentArgs;
pub use language::{AUTO, Language, resolve};

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// One bundle per locale, each parsed the first time a lookup needs it. An ordinary run only
/// ever parses the active language and the English fallback.
static BUNDLES: [OnceLock<FluentBundle<FluentResource>>; Language::ALL.len()] =
    [const { OnceLock::new() }; Language::ALL.len()];

pub trait Value<'a> {
    fn value(self) -> FluentValue<'a>;
}

macro_rules! values {
    ($($type:ty),* $(,)?) => {
        $(
            impl<'a> Value<'a> for $type {
                fn value(self) -> FluentValue<'a> {
                    FluentValue::from(self)
                }
            }
        )*
    };
}

values![
    i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64, String
];

impl<'a> Value<'a> for &'a str {
    fn value(self) -> FluentValue<'a> {
        FluentValue::from(self)
    }
}

impl<'a> Value<'a> for &'a String {
    fn value(self) -> FluentValue<'a> {
        FluentValue::from(self.as_str())
    }
}

impl<'a> Value<'a> for &'a SharedString {
    fn value(self) -> FluentValue<'a> {
        FluentValue::from(self.as_ref())
    }
}

#[macro_export]
macro_rules! t {
    ($key:literal) => {
        $crate::lookup($key, ::std::option::Option::None)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {{
        let mut args = $crate::FluentArgs::new();
        $(
            args.set(stringify!($name), $crate::Value::value($value));
        )+
        $crate::lookup($key, ::std::option::Option::Some(&args))
    }};
}

pub fn language() -> Language {
    Language::ALL[ACTIVE.load(Ordering::Relaxed)]
}

pub fn set(language: Language) {
    ACTIVE.store(language as usize, Ordering::Relaxed);
}

pub fn lookup(key: &str, args: Option<&FluentArgs>) -> SharedString {
    let active = language();
    if let Some(text) = format(active, key, args) {
        return text;
    }

    log::warn!("i18n: {key} is missing from {}", active.id());
    if active != Language::English
        && let Some(text) = format(Language::English, key, args)
    {
        return text;
    }
    SharedString::from(key.to_owned())
}

pub fn translate(key: &str) -> SharedString {
    let active = language();
    if let Some(text) = format(active, key, None) {
        return text;
    }
    if active != Language::English
        && let Some(text) = format(Language::English, key, None)
    {
        return text;
    }
    SharedString::from(key.to_owned())
}

/// Groups a signed integer using the active locale, unless it is below `threshold`.
pub fn grouped_number(value: i128, threshold: u128) -> SharedString {
    if value.unsigned_abs() < threshold {
        return value.to_string().into();
    }
    let digits = value.unsigned_abs().to_string();
    let groups: Vec<&str> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect();
    let sign = match value < 0 {
        true => "-",
        false => "",
    };
    format!("{sign}{}", groups.join(&t!("number-group"))).into()
}

fn format(language: Language, key: &str, args: Option<&FluentArgs>) -> Option<SharedString> {
    let bundle = BUNDLES
        .get(language as usize)?
        .get_or_init(|| bundle(language));
    let pattern = bundle.get_message(key)?.value()?;

    let mut errors = Vec::new();
    let text = bundle.format_pattern(pattern, args, &mut errors);
    for error in &errors {
        log::warn!("i18n: cannot format {key}: {error}");
    }
    // The bundles are never dropped, so a message with nothing to fill in is shared as is.
    Some(match text {
        Cow::Borrowed(text) => SharedString::new_static(text),
        Cow::Owned(text) => SharedString::from(text),
    })
}

fn bundle(language: Language) -> FluentBundle<FluentResource> {
    let mut bundle = FluentBundle::new_concurrent(vec![language.tag()]);
    bundle.set_use_isolating(false);

    match FluentResource::try_new(language.source().to_owned()) {
        Ok(resource) => {
            if let Err(errors) = bundle.add_resource(resource) {
                for error in errors {
                    log::error!("i18n: cannot load {}: {error}", language.id());
                }
            }
        }
        Err((resource, errors)) => {
            for error in errors {
                log::error!("i18n: cannot parse {}: {error}", language.id());
            }
            bundle.add_resource_overriding(resource);
        }
    }
    bundle
}

#[cfg(test)]
fn keys(language: Language) -> Vec<&'static str> {
    let mut keys: Vec<&str> = language
        .source()
        .lines()
        .filter(|line| !line.starts_with([' ', '#', '.', '*', '[']))
        .filter_map(|line| line.split_once(" =").map(|(key, _)| key))
        .filter(|key| !key.is_empty() && !key.starts_with('-'))
        .collect();
    keys.sort_unstable();
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_carries_every_key() {
        let english = keys(Language::English);
        assert!(english.len() > 100);
    }

    #[test]
    fn no_locale_invents_a_key() {
        let english = keys(Language::English);

        for language in Language::ALL {
            let extra: Vec<&str> = keys(language)
                .into_iter()
                .filter(|key| !english.contains(key))
                .collect();

            assert!(extra.is_empty(), "{} has extra {extra:?}", language.id());
        }
    }

    #[test]
    fn every_message_resolves() {
        for language in Language::ALL {
            for key in keys(language) {
                assert!(format(language, key, None).is_some(), "{key} has no value");
            }
        }
    }

    #[test]
    fn plurals_pick_the_right_category() {
        let cases = [
            (Language::English, ["1 song", "2 songs", "5 songs"]),
            (Language::Russian, ["1 трек", "2 трека", "5 треков"]),
            (Language::Ukrainian, ["1 трек", "2 треки", "5 треків"]),
            (Language::Polish, ["1 utwór", "2 utwory", "5 utworów"]),
            (Language::Czech, ["1 skladba", "2 skladby", "5 skladeb"]),
        ];

        for (language, expected) in cases {
            for (count, text) in [1, 2, 5].into_iter().zip(expected) {
                let mut args = FluentArgs::new();
                args.set("count", count);
                assert_eq!(format(language, "count-songs", Some(&args)).unwrap(), text);
            }
        }
    }

    #[test]
    fn translate_resolves_known_key_or_preserves_literal() {
        assert_eq!(translate("home-quick-picks"), "Quick picks");
        assert_eq!(translate("home-recently-added"), "Recently added");
        assert_eq!(translate("Top 50 - Global"), "Top 50 - Global");
    }
}
