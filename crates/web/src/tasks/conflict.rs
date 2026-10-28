use shared::api::browse::EntryKind;
use shared::api::conflict::ConflictPolicy;
use shared::api::tasks::TaskSpec;

use crate::api::ApiFailure;

pub(super) const ALREADY_EXISTS: &str = "already_exists";

/// The existing entry a copy or move would land on.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub volume: String,
    pub path: String,
    pub kind: EntryKind,
}

impl Conflict {
    /// From the details of a 409 `already_exists`.
    pub(super) fn of(failure: &ApiFailure) -> Option<Self> {
        let ApiFailure::Server(body) = failure else {
            return None;
        };
        let details = body
            .details
            .as_ref()
            .filter(|_| body.code == ALREADY_EXISTS)?;
        let field = |name| details.get(name)?.as_str().map(str::to_owned);
        let kind = match field("kind").as_deref() {
            Some("dir") => EntryKind::Dir,
            _ => EntryKind::File,
        };
        Some(Self {
            volume: field("volume")?,
            path: field("path")?,
            kind,
        })
    }
}

pub(super) fn policy_of(spec: &TaskSpec) -> Option<ConflictPolicy> {
    match spec {
        TaskSpec::Copy { conflict, .. } | TaskSpec::Move { conflict, .. } => Some(*conflict),
        TaskSpec::Delete { .. } | TaskSpec::Usage { .. } => None,
    }
}

pub(super) fn with_policy(spec: &TaskSpec, policy: ConflictPolicy) -> TaskSpec {
    let mut spec = spec.clone();
    if let TaskSpec::Copy { conflict, .. } | TaskSpec::Move { conflict, .. } = &mut spec {
        *conflict = policy;
    }
    spec
}
