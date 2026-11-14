use unic_langid::{LanguageIdentifier, langid};

pub const AUTO: &str = "auto";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum Language {
    English,
    German,
    Spanish,
    French,
    Italian,
    Indonesian,
    Japanese,
    Korean,
    Russian,
    Ukrainian,
    Polish,
    Czech,
    PortugueseBrazilian,
    PortugueseEuropean,
    ChineseSimplified,
    Turkish,
    Albanian,
}

impl Language {
    pub const ALL: [Self; 17] = [
        Self::English,
        Self::German,
        Self::Spanish,
        Self::French,
        Self::Italian,
        Self::Indonesian,
        Self::Japanese,
        Self::Korean,
        Self::Russian,
        Self::Ukrainian,
        Self::Polish,
        Self::Czech,
        Self::PortugueseBrazilian,
        Self::PortugueseEuropean,
        Self::ChineseSimplified,
        Self::Turkish,
        Self::Albanian,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::German => "de",
            Self::Spanish => "es",
            Self::French => "fr",
            Self::Italian => "it",
            Self::Indonesian => "id",
            Self::Japanese => "ja",
            Self::Korean => "ko",
            Self::Russian => "ru",
            Self::Ukrainian => "uk",
            Self::Polish => "pl",
            Self::Czech => "cs",
            Self::PortugueseBrazilian => "pt-BR",
            Self::PortugueseEuropean => "pt-PT",
            Self::ChineseSimplified => "zh-CN",
            Self::Turkish => "tr",
            Self::Albanian => "sq",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::German => "Deutsch",
            Self::Spanish => "Español",
            Self::French => "Français",
            Self::Italian => "Italiano",
            Self::Indonesian => "Bahasa Indonesia",
            Self::Japanese => "日本語",
            Self::Korean => "한국어",
            Self::Russian => "Русский",
            Self::Ukrainian => "Українська",
            Self::Polish => "Polski",
            Self::Czech => "Čeština",
            Self::PortugueseBrazilian => "Português (Brasil)",
            Self::PortugueseEuropean => "Português (Portugal)",
            Self::ChineseSimplified => "简体中文",
            Self::Turkish => "Türkçe",
            Self::Albanian => "Shqip",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|language| language.id() == id)
    }

    pub fn detect() -> Self {
        sys_locale::get_locale().map_or(Self::English, |locale| Self::closest(&locale))
    }

    /// The shipped language nearest to a system locale such as `pt-PT` or `pt_PT.UTF-8`. A
    /// language shipped for more than one region is matched on the region first. A region
    /// nothing ships for falls back to the first variant of that language in `ALL`.
    fn closest(locale: &str) -> Self {
        let wanted = parts(locale);

        Self::ALL
            .into_iter()
            .find(|language| parts(language.id()) == wanted)
            .or_else(|| {
                Self::ALL
                    .into_iter()
                    .find(|language| parts(language.id()).0 == wanted.0)
            })
            .unwrap_or(Self::English)
    }

    pub(crate) fn tag(self) -> LanguageIdentifier {
        match self {
            Self::English => langid!("en-US"),
            Self::German => langid!("de"),
            Self::Spanish => langid!("es"),
            Self::French => langid!("fr"),
            Self::Italian => langid!("it"),
            Self::Indonesian => langid!("id"),
            Self::Japanese => langid!("ja"),
            Self::Korean => langid!("ko"),
            Self::Russian => langid!("ru"),
            Self::Ukrainian => langid!("uk"),
            Self::Polish => langid!("pl"),
            Self::Czech => langid!("cs"),
            Self::PortugueseBrazilian => langid!("pt-BR"),
            Self::PortugueseEuropean => langid!("pt-PT"),
            Self::ChineseSimplified => langid!("zh-CN"),
            Self::Turkish => langid!("tr"),
            Self::Albanian => langid!("sq"),
        }
    }

    pub(crate) fn source(self) -> &'static str {
        match self {
            Self::English => include_str!("../../../assets/i18n/en-US/main.ftl"),
            Self::German => include_str!("../../../assets/i18n/de/main.ftl"),
            Self::Spanish => include_str!("../../../assets/i18n/es/main.ftl"),
            Self::French => include_str!("../../../assets/i18n/fr/main.ftl"),
            Self::Italian => include_str!("../../../assets/i18n/it/main.ftl"),
            Self::Indonesian => include_str!("../../../assets/i18n/id/main.ftl"),
            Self::Japanese => include_str!("../../../assets/i18n/ja/main.ftl"),
            Self::Korean => include_str!("../../../assets/i18n/ko/main.ftl"),
            Self::Russian => include_str!("../../../assets/i18n/ru/main.ftl"),
            Self::Ukrainian => include_str!("../../../assets/i18n/uk/main.ftl"),
            Self::Polish => include_str!("../../../assets/i18n/pl/main.ftl"),
            Self::Czech => include_str!("../../../assets/i18n/cs/main.ftl"),
            Self::PortugueseBrazilian => include_str!("../../../assets/i18n/pt-BR/main.ftl"),
            Self::PortugueseEuropean => include_str!("../../../assets/i18n/pt-PT/main.ftl"),
            Self::ChineseSimplified => include_str!("../../../assets/i18n/zh-CN/main.ftl"),
            Self::Turkish => include_str!("../../../assets/i18n/tr/main.ftl"),
            Self::Albanian => include_str!("../../../assets/i18n/sq/main.ftl"),
        }
    }
}

pub fn resolve(id: &str) -> Language {
    Language::from_id(id).unwrap_or_else(Language::detect)
}

/// The language and the region of a locale tag, lowercased and uppercased. The encoding and
/// modifier a POSIX locale carries are dropped, and so is a script such as `Hans`.
fn parts(tag: &str) -> (String, Option<String>) {
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    let mut subtags = tag.split(['-', '_']);
    let language = subtags.next().unwrap_or_default().to_lowercase();
    let region = subtags
        .find(|subtag| {
            subtag.len() == 2 && subtag.chars().all(|letter| letter.is_ascii_alphabetic())
        })
        .map(str::to_uppercase);
    (language, region)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_picks_its_own_variant() {
        assert_eq!(Language::closest("pt-PT"), Language::PortugueseEuropean);
        assert_eq!(
            Language::closest("pt_PT.UTF-8"),
            Language::PortugueseEuropean
        );
        assert_eq!(Language::closest("pt-BR"), Language::PortugueseBrazilian);
        assert_eq!(Language::closest("zh-Hans-CN"), Language::ChineseSimplified);
    }

    #[test]
    fn an_unshipped_region_falls_back_to_the_language() {
        assert_eq!(Language::closest("pt"), Language::PortugueseBrazilian);
        assert_eq!(Language::closest("pt-AO"), Language::PortugueseBrazilian);
        assert_eq!(Language::closest("de-AT"), Language::German);
        assert_eq!(Language::closest("en-GB"), Language::English);
    }

    #[test]
    fn an_unknown_language_falls_back_to_english() {
        assert_eq!(Language::closest("nl-NL"), Language::English);
        assert_eq!(Language::closest(""), Language::English);
    }
}
