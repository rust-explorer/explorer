use dioxus_i18n::prelude::{I18nConfig, use_init_i18n};
use unic_langid::{LanguageIdentifier, langid};

pub const SUPPORTED: [&str; 3] = ["en-US", "fr-FR", "es-ES"];

const FALLBACK: LanguageIdentifier = langid!("en-US");

const RESOURCES: [(LanguageIdentifier, &str); 3] = [
    (langid!("en-US"), include_str!("../locales/en-US.ftl")),
    (langid!("fr-FR"), include_str!("../locales/fr-FR.ftl")),
    (langid!("es-ES"), include_str!("../locales/es-ES.ftl")),
];

/// Picks the first supported locale whose language subtag matches one of `preferred`.
pub fn negotiate(preferred: &[String]) -> LanguageIdentifier {
    let supported: Vec<LanguageIdentifier> = SUPPORTED
        .iter()
        .map(|tag| tag.parse().expect("supported locale tags are valid"))
        .collect();
    preferred
        .iter()
        .filter_map(|tag| tag.parse::<LanguageIdentifier>().ok())
        .find_map(|wanted| {
            supported
                .iter()
                .find(|candidate| candidate.language == wanted.language)
                .cloned()
        })
        .unwrap_or(FALLBACK)
}

pub fn use_init_i18n_from_browser() {
    use_init_i18n(|| {
        RESOURCES.into_iter().fold(
            I18nConfig::new(negotiate(&browser_languages())).with_fallback(FALLBACK),
            I18nConfig::with_locale,
        )
    });
}

#[cfg(target_arch = "wasm32")]
fn browser_languages() -> Vec<String> {
    let Some(navigator) = web_sys::window().map(|window| window.navigator()) else {
        return Vec::new();
    };
    let languages: Vec<String> = navigator
        .languages()
        .iter()
        .filter_map(|value| value.as_string())
        .collect();
    if languages.is_empty() {
        navigator.language().into_iter().collect()
    } else {
        languages
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_languages() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn bundled_resources_match_supported_locales() {
        let bundled: Vec<String> = RESOURCES.iter().map(|(id, _)| id.to_string()).collect();
        assert_eq!(bundled, SUPPORTED);
    }

    #[test]
    fn negotiate_exact_match() {
        assert_eq!(negotiate(&tags(&["fr-FR"])), langid!("fr-FR"));
    }

    #[test]
    fn negotiate_language_only() {
        assert_eq!(negotiate(&tags(&["es-MX"])), langid!("es-ES"));
        assert_eq!(negotiate(&tags(&["fr"])), langid!("fr-FR"));
    }

    #[test]
    fn negotiate_first_supported_wins() {
        assert_eq!(
            negotiate(&tags(&["de-DE", "fr-CA", "en-GB"])),
            langid!("fr-FR")
        );
    }

    #[test]
    fn negotiate_falls_back_to_english() {
        assert_eq!(negotiate(&tags(&["de-DE"])), langid!("en-US"));
        assert_eq!(negotiate(&[]), langid!("en-US"));
    }
}
