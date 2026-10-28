use dioxus::prelude::use_hook;
use dioxus_i18n::prelude::{I18n, I18nConfig, use_init_i18n};
use unic_langid::{LanguageIdentifier, langid};

pub const SUPPORTED: [&str; 3] = ["en-US", "fr-FR", "es-ES"];
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub const STORAGE_KEY: &str = "explorer.locale";

const FALLBACK: LanguageIdentifier = langid!("en-US");

const RESOURCES: [(LanguageIdentifier, &str); 3] = [
    (langid!("en-US"), include_str!("../locales/en-US.ftl")),
    (langid!("fr-FR"), include_str!("../locales/fr-FR.ftl")),
    (langid!("es-ES"), include_str!("../locales/es-ES.ftl")),
];

/// `text` without the bidi isolation marks Fluent puts around arguments, for attributes such as
/// `aria-label` and `title`, where assistive technologies may read them out.
pub fn attr(text: String) -> String {
    if text.contains(['\u{2068}', '\u{2069}']) {
        text.replace(['\u{2068}', '\u{2069}'], "")
    } else {
        text
    }
}

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

pub fn supported() -> impl Iterator<Item = LanguageIdentifier> {
    RESOURCES.into_iter().map(|(id, _)| id)
}

/// A stored override wins when it names a supported locale; otherwise the browser decides.
pub fn initial_locale(stored: Option<&str>, browser: &[String]) -> LanguageIdentifier {
    stored
        .and_then(|tag| tag.parse::<LanguageIdentifier>().ok())
        .filter(|id| supported().any(|candidate| candidate == *id))
        .unwrap_or_else(|| negotiate(browser))
}

pub fn native_name(lang: &LanguageIdentifier) -> &'static str {
    match lang.language.as_str() {
        "fr" => "Français",
        "es" => "Español",
        _ => "English",
    }
}

pub fn use_init_i18n_from_browser() {
    let i18n = use_init_i18n(|| {
        let locale = initial_locale(stored_locale().as_deref(), &browser_languages());
        RESOURCES.into_iter().fold(
            I18nConfig::new(locale).with_fallback(FALLBACK),
            I18nConfig::with_locale,
        )
    });
    use_hook(|| set_document_lang(&i18n.language()));
}

/// Switches the UI language and remembers the choice for the next visit.
pub fn change_locale(mut i18n: I18n, locale: LanguageIdentifier) {
    store_locale(&locale);
    set_document_lang(&locale);
    i18n.set_language(locale);
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(target_arch = "wasm32")]
fn stored_locale() -> Option<String> {
    local_storage()?.get_item(STORAGE_KEY).ok().flatten()
}

#[cfg(target_arch = "wasm32")]
fn store_locale(locale: &LanguageIdentifier) {
    // Private browsing may refuse storage; the choice then lasts for this page only.
    if let Some(storage) = local_storage() {
        let _ = storage.set_item(STORAGE_KEY, &locale.to_string());
    }
}

#[cfg(target_arch = "wasm32")]
fn set_document_lang(locale: &LanguageIdentifier) {
    if let Some(root) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.document_element())
    {
        let _ = root.set_attribute("lang", &locale.to_string());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn stored_locale() -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn store_locale(_: &LanguageIdentifier) {}

#[cfg(not(target_arch = "wasm32"))]
fn set_document_lang(_: &LanguageIdentifier) {}

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

    #[test]
    fn attr_drops_isolation_marks() {
        assert_eq!(
            attr("Cancel \u{2068}a.txt\u{2069}".to_owned()),
            "Cancel a.txt"
        );
        assert_eq!(attr("Upload".to_owned()), "Upload");
    }

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

    #[test]
    fn initial_locale_prefers_valid_stored_value() {
        assert_eq!(
            initial_locale(Some("es-ES"), &tags(&["fr-FR"])),
            langid!("es-ES")
        );
    }

    #[test]
    fn initial_locale_ignores_invalid_stored_value() {
        assert_eq!(
            initial_locale(Some("de-DE"), &tags(&["fr-FR"])),
            langid!("fr-FR")
        );
        assert_eq!(
            initial_locale(Some("not a tag"), &tags(&[])),
            langid!("en-US")
        );
        assert_eq!(initial_locale(None, &tags(&["es"])), langid!("es-ES"));
    }

    #[test]
    fn native_names() {
        assert_eq!(native_name(&langid!("en-US")), "English");
        assert_eq!(native_name(&langid!("fr-FR")), "Français");
        assert_eq!(native_name(&langid!("es-ES")), "Español");
    }
}
