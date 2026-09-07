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
    Russian,
    Ukrainian,
    Polish,
    PortugueseBrazilian,
}

impl Language {
    pub const ALL: [Self; 11] = [
        Self::English,
        Self::German,
        Self::Spanish,
        Self::French,
        Self::Italian,
        Self::Indonesian,
        Self::Japanese,
        Self::Russian,
        Self::Ukrainian,
        Self::Polish,
        Self::PortugueseBrazilian,
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
            Self::Russian => "ru",
            Self::Ukrainian => "uk",
            Self::Polish => "pl",
            Self::PortugueseBrazilian => "pt-BR",
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
            Self::Russian => "Русский",
            Self::Ukrainian => "Українська",
            Self::Polish => "Polski",
            Self::PortugueseBrazilian => "Português (Brasil)",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|language| language.id() == id)
    }

    pub fn detect() -> Self {
        let Some(locale) = sys_locale::get_locale() else {
            return Self::English;
        };
        let primary = base(&locale);

        Self::ALL
            .into_iter()
            .find(|language| base(language.id()) == primary)
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
            Self::Russian => langid!("ru"),
            Self::Ukrainian => langid!("uk"),
            Self::Polish => langid!("pl"),
            Self::PortugueseBrazilian => langid!("pt-BR"),
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
            Self::Russian => include_str!("../../../assets/i18n/ru/main.ftl"),
            Self::Ukrainian => include_str!("../../../assets/i18n/uk/main.ftl"),
            Self::Polish => include_str!("../../../assets/i18n/pl/main.ftl"),
            Self::PortugueseBrazilian => include_str!("../../../assets/i18n/pt-BR/main.ftl"),
        }
    }
}

pub fn resolve(id: &str) -> Language {
    Language::from_id(id).unwrap_or_else(Language::detect)
}

fn base(tag: &str) -> String {
    tag.split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}
