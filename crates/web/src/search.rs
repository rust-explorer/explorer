use dioxus::prelude::*;

/// The server refuses shorter queries.
pub const MIN_QUERY_CHARS: usize = 2;
pub const DEBOUNCE_MS: u32 = 300;

/// What the header's search field asks for; the browse page clears it on navigation.
#[derive(Clone, Copy)]
pub struct SearchQuery(pub Signal<String>);

pub fn use_search_provider() -> SearchQuery {
    use_context_provider(|| SearchQuery(Signal::new(String::new())))
}

/// The query to search for, trimmed, once it is long enough.
pub fn effective_query(raw: &str) -> Option<&str> {
    let query = raw.trim();
    (query.chars().count() >= MIN_QUERY_CHARS).then_some(query)
}

#[cfg(target_arch = "wasm32")]
pub async fn sleep(ms: u32) {
    gloo_timers::future::TimeoutFuture::new(ms).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn sleep(_: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_need_two_characters_after_trimming() {
        assert_eq!(effective_query(""), None);
        assert_eq!(effective_query(" a "), None);
        assert_eq!(effective_query("é"), None);
        assert_eq!(effective_query(" ab "), Some("ab"));
        assert_eq!(effective_query("été"), Some("été"));
    }
}
