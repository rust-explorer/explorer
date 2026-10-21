use shared::api::ConflictPolicy;

use super::Effect;
use crate::api::ApiFailure;
use crate::uploads::CHUNK_SIZE;
use crate::uploads::plan::FileSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemKey(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Replace,
    KeepBoth,
    Cancel,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Queued,
    NeedsDecision,
    Uploading,
    Retrying { attempt: u32, at_ms: i64 },
    Done,
    Failed(ApiFailure),
    Cancelled,
    Skipped,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    File(FileSpec),
    Folder { volume: String, path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    Idle,
    CreatingFolder,
    Creating,
    Heading,
    Patching,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub key: ItemKey,
    pub batch: u64,
    pub kind: Kind,
    pub status: Status,
    pub sent: u64,
    pub upload_url: Option<String>,
    pub conflict: ConflictPolicy,
    pub(super) phase: Phase,
    pub(super) attempts: u32,
    /// Last offset the server confirmed.
    pub(super) confirmed: u64,
    pub(super) restarted: bool,
}

impl Status {
    pub(super) fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::NeedsDecision | Self::Uploading | Self::Retrying { .. }
        )
    }
}

impl Item {
    pub(super) fn volume(&self) -> &str {
        match &self.kind {
            Kind::File(spec) => &spec.volume,
            Kind::Folder { volume, .. } => volume,
        }
    }

    pub(super) fn size(&self) -> u64 {
        match &self.kind {
            Kind::File(spec) => spec.size,
            Kind::Folder { .. } => 0,
        }
    }

    pub(super) fn create(&mut self) -> Effect {
        self.phase = Phase::Creating;
        let Kind::File(spec) = &self.kind else {
            unreachable!("folders are never uploaded");
        };
        Effect::Create {
            key: self.key,
            spec: spec.clone(),
            conflict: self.conflict,
        }
    }

    pub(super) fn patch(&mut self, url: String, offset: u64) -> Effect {
        self.phase = Phase::Patching;
        self.sent = offset;
        Effect::Patch {
            key: self.key,
            url,
            offset,
            len: CHUNK_SIZE.min(self.size() - offset),
        }
    }
}

pub(super) fn policy(decision: Decision) -> Option<ConflictPolicy> {
    match decision {
        Decision::Replace => Some(ConflictPolicy::Replace),
        Decision::KeepBoth => Some(ConflictPolicy::KeepBoth),
        Decision::Cancel => None,
    }
}
