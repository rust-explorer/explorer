use dioxus::prelude::*;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const STORAGE_KEY: &str = "explorer.view";

/// How the browse page shows a folder; remembered in this browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

impl ViewMode {
    /// An unknown or missing stored value gives the default.
    pub fn parse(stored: Option<&str>) -> Self {
        match stored {
            Some("grid") => Self::Grid,
            _ => Self::List,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Grid => "grid",
        }
    }
}

/// App-wide, so the choice holds across navigation.
pub fn use_view_mode_provider() -> Signal<ViewMode> {
    use_context_provider(|| Signal::new(ViewMode::parse(stored().as_deref())))
}

pub fn set_view_mode(mut mode: Signal<ViewMode>, value: ViewMode) {
    store(value);
    mode.set(value);
}

#[cfg(target_arch = "wasm32")]
fn stored() -> Option<String> {
    crate::i18n::local_storage()?
        .get_item(STORAGE_KEY)
        .ok()
        .flatten()
}

#[cfg(target_arch = "wasm32")]
fn store(mode: ViewMode) {
    // Private browsing may refuse storage; the choice then lasts for this page only.
    if let Some(storage) = crate::i18n::local_storage() {
        let _ = storage.set_item(STORAGE_KEY, mode.as_str());
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn stored() -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
fn store(_: ViewMode) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_value_round_trips() {
        for mode in [ViewMode::List, ViewMode::Grid] {
            assert_eq!(ViewMode::parse(Some(mode.as_str())), mode);
        }
    }

    #[test]
    fn unknown_stored_value_gives_the_list() {
        assert_eq!(ViewMode::parse(None), ViewMode::List);
        assert_eq!(ViewMode::parse(Some("")), ViewMode::List);
        assert_eq!(ViewMode::parse(Some("GRID")), ViewMode::List);
        assert_eq!(ViewMode::parse(Some("tiles")), ViewMode::List);
    }
}
