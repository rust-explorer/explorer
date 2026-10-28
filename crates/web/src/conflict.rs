use dioxus::prelude::*;
use shared::api::conflict::ConflictPolicy;

use crate::tasks::use_tasks;
use crate::uploads::use_uploads;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Cancel,
    Replace,
    KeepBoth,
}

impl Decision {
    pub fn policy(self) -> Option<ConflictPolicy> {
        match self {
            Self::Cancel => None,
            Self::Replace => Some(ConflictPolicy::Replace),
            Self::KeepBoth => Some(ConflictPolicy::KeepBoth),
        }
    }
}

/// True while an upload or a task waits for a conflict answer: its prompt has the screen, and a
/// rename's prompt waits for it to close.
pub fn use_queued_conflict() -> Memo<bool> {
    let uploads = use_uploads();
    let tasks = use_tasks();
    use_memo(move || {
        uploads.queue().read().pending_decision().is_some()
            || tasks.list().read().pending_decision().is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_map_to_policies() {
        assert_eq!(Decision::Cancel.policy(), None);
        assert_eq!(Decision::Replace.policy(), Some(ConflictPolicy::Replace));
        assert_eq!(Decision::KeepBoth.policy(), Some(ConflictPolicy::KeepBoth));
    }
}
