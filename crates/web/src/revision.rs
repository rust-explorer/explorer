use dioxus::prelude::*;

/// Bumped when something changed on disk; the browse page's volume and listing resources read it.
#[derive(Clone, Copy)]
pub struct Revision(pub Signal<u64>);

pub fn use_revision_provider() -> Revision {
    use_context_provider(|| Revision(Signal::new(0)))
}
