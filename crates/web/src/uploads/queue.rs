use std::collections::{HashMap, HashSet};

use shared::api::{ConflictPolicy, PendingUpload};

use super::plan::{BatchPlan, FileSpec};
use super::{CHUNK_SIZE, MAX_PARALLEL, REFRESH_EVERY_MS, retry_delay_ms};
use crate::api::{self, ApiFailure};

const ALREADY_EXISTS: &str = "already_exists";
const UPLOAD_NOT_FOUND: &str = "upload_not_found";
const UPLOAD_BUSY: &str = "upload_busy";
const OFFSET_MISMATCH: &str = "offset_mismatch";

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
enum Phase {
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
    phase: Phase,
    attempts: u32,
    restarted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    CreateFolder {
        key: ItemKey,
        volume: String,
        path: String,
    },
    Create {
        key: ItemKey,
        spec: FileSpec,
        conflict: ConflictPolicy,
    },
    Head {
        key: ItemKey,
        url: String,
    },
    Patch {
        key: ItemKey,
        url: String,
        offset: u64,
        len: u64,
    },
    /// Stops the item's in-flight request.
    Abort {
        key: ItemKey,
    },
    /// Terminates the upload on the server.
    Delete {
        url: String,
    },
    /// `tick` is due at that time.
    WakeAt {
        at_ms: i64,
    },
    Refresh {
        volume: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    FolderCreated {
        key: ItemKey,
    },
    Created {
        key: ItemKey,
        url: String,
    },
    /// `sent` counts every byte of the item the server has, not just this request's.
    Progress {
        key: ItemKey,
        sent: u64,
    },
    Patched {
        key: ItemKey,
        offset: u64,
    },
    HeadDone {
        key: ItemKey,
        offset: u64,
    },
    Failed {
        key: ItemKey,
        failure: ApiFailure,
    },
    Decide {
        key: ItemKey,
        decision: Decision,
        apply_to_all: bool,
    },
    Cancel {
        key: ItemKey,
    },
}

/// Upload state machine: events in, requests to send out.
#[derive(Debug, Default)]
pub struct Queue {
    items: Vec<Item>,
    next_key: u64,
    next_batch: u64,
    decisions: HashMap<u64, Decision>,
    last_refresh: HashMap<String, i64>,
    unrefreshed: HashSet<String>,
}

impl Status {
    fn is_active(&self) -> bool {
        matches!(
            self,
            Self::Queued | Self::NeedsDecision | Self::Uploading | Self::Retrying { .. }
        )
    }
}

impl Item {
    fn volume(&self) -> &str {
        match &self.kind {
            Kind::File(spec) => &spec.volume,
            Kind::Folder { volume, .. } => volume,
        }
    }

    fn size(&self) -> u64 {
        match &self.kind {
            Kind::File(spec) => spec.size,
            Kind::Folder { .. } => 0,
        }
    }

    fn create(&mut self) -> Effect {
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

    fn patch(&mut self, url: String, offset: u64) -> Effect {
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

fn policy(decision: Decision) -> Option<ConflictPolicy> {
    match decision {
        Decision::Replace => Some(ConflictPolicy::Replace),
        Decision::KeepBoth => Some(ConflictPolicy::KeepBoth),
        Decision::Cancel => None,
    }
}

fn is_transient(failure: &ApiFailure) -> bool {
    match failure {
        ApiFailure::Network => true,
        ApiFailure::Http(status) => *status >= 500 && *status != 507,
        ApiFailure::Server(body) => matches!(body.code.as_str(), UPLOAD_BUSY | OFFSET_MISMATCH),
    }
}

impl Queue {
    pub fn add(&mut self, plan: BatchPlan, pending: &[PendingUpload], _now_ms: i64) -> Vec<Effect> {
        let batch = self.next_batch;
        self.next_batch += 1;
        let folders = plan.folders.into_iter().map(|path| {
            let kind = Kind::Folder {
                volume: plan.volume.clone(),
                path,
            };
            (kind, None, 0)
        });
        let files = plan.files.into_iter().map(|spec| {
            let resumed = pending.iter().find(|upload| {
                upload.volume == spec.volume
                    && upload.path == spec.path
                    && upload.length == spec.size
                    && upload.modified_ms == spec.modified_ms
            });
            let url = resumed.map(|upload| api::upload_url(&upload.id));
            let sent = resumed.map_or(0, |upload| upload.offset);
            (Kind::File(spec), url, sent)
        });
        for (kind, upload_url, sent) in folders.chain(files) {
            let key = ItemKey(self.next_key);
            self.next_key += 1;
            self.items.push(Item {
                key,
                batch,
                kind,
                status: Status::Queued,
                sent,
                upload_url,
                conflict: ConflictPolicy::Fail,
                phase: Phase::Idle,
                attempts: 0,
                restarted: false,
            });
        }
        let mut effects = Vec::new();
        self.pump(&mut effects);
        effects
    }

    pub fn handle(&mut self, event: Event, now_ms: i64) -> Vec<Effect> {
        let mut effects = Vec::new();
        match event {
            Event::FolderCreated { key } => {
                if let Some(index) = self.in_flight(key) {
                    self.finish(index, Status::Done, now_ms, &mut effects);
                }
            }
            Event::Created { key, url } => match self.index(key) {
                Some(index) if self.items[index].status == Status::Uploading => {
                    let item = &mut self.items[index];
                    item.upload_url = Some(url.clone());
                    item.attempts = 0;
                    self.advance(index, url, 0, now_ms, &mut effects);
                }
                Some(index) if self.items[index].status == Status::Cancelled => {
                    effects.push(Effect::Delete { url });
                }
                _ => {}
            },
            Event::Progress { key, sent } => {
                if let Some(index) = self.in_flight(key) {
                    self.items[index].sent = sent;
                }
            }
            Event::Patched { key, offset } | Event::HeadDone { key, offset } => {
                if let Some(index) = self.in_flight(key) {
                    let item = &mut self.items[index];
                    item.attempts = 0;
                    if let Some(url) = item.upload_url.clone() {
                        self.advance(index, url, offset, now_ms, &mut effects);
                    }
                }
            }
            Event::Failed { key, failure } => {
                if let Some(index) = self.in_flight(key) {
                    self.fail(index, failure, now_ms, &mut effects);
                }
            }
            Event::Decide {
                key,
                decision,
                apply_to_all,
            } => self.decide(key, decision, apply_to_all, now_ms, &mut effects),
            Event::Cancel { key } => self.cancel(key, now_ms, &mut effects),
        }
        self.pump(&mut effects);
        effects
    }

    pub fn tick(&mut self, now_ms: i64) -> Vec<Effect> {
        for item in &mut self.items {
            if matches!(item.status, Status::Retrying { at_ms, .. } if at_ms <= now_ms) {
                item.status = Status::Queued;
            }
        }
        let mut effects = Vec::new();
        self.pump(&mut effects);
        effects
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn pending_decision(&self) -> Option<&Item> {
        self.items
            .iter()
            .find(|item| item.status == Status::NeedsDecision)
    }

    pub fn batch_len(&self, batch: u64) -> usize {
        self.items.iter().filter(|item| item.batch == batch).count()
    }

    pub fn is_active(&self) -> bool {
        self.items.iter().any(|item| item.status.is_active())
    }

    pub fn clear_finished(&mut self) {
        self.items.retain(|item| item.status.is_active());
    }

    fn index(&self, key: ItemKey) -> Option<usize> {
        self.items.iter().position(|item| item.key == key)
    }

    fn in_flight(&self, key: ItemKey) -> Option<usize> {
        self.index(key)
            .filter(|&index| self.items[index].status == Status::Uploading)
    }

    fn pump(&mut self, effects: &mut Vec<Effect>) {
        let mut running = self
            .items
            .iter()
            .filter(|item| item.status == Status::Uploading)
            .count();
        for index in 0..self.items.len() {
            if running >= MAX_PARALLEL {
                break;
            }
            if self.items[index].status == Status::Queued {
                effects.push(self.start(index));
                running += 1;
            }
        }
    }

    fn start(&mut self, index: usize) -> Effect {
        let decided = self
            .decisions
            .get(&self.items[index].batch)
            .copied()
            .and_then(policy);
        let item = &mut self.items[index];
        item.status = Status::Uploading;
        match (&item.kind, item.upload_url.clone()) {
            (Kind::Folder { volume, path }, _) => {
                item.phase = Phase::CreatingFolder;
                Effect::CreateFolder {
                    key: item.key,
                    volume: volume.clone(),
                    path: path.clone(),
                }
            }
            (Kind::File(_), Some(url)) => {
                item.phase = Phase::Heading;
                Effect::Head { key: item.key, url }
            }
            (Kind::File(_), None) => {
                if item.conflict == ConflictPolicy::Fail
                    && let Some(conflict) = decided
                {
                    item.conflict = conflict;
                }
                item.create()
            }
        }
    }

    fn advance(
        &mut self,
        index: usize,
        url: String,
        offset: u64,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let item = &mut self.items[index];
        if offset >= item.size() {
            item.sent = item.size();
            self.finish(index, Status::Done, now_ms, effects);
        } else {
            effects.push(item.patch(url, offset));
        }
    }

    fn fail(&mut self, index: usize, failure: ApiFailure, now_ms: i64, effects: &mut Vec<Effect>) {
        let decision = self.decisions.get(&self.items[index].batch).copied();
        let item = &mut self.items[index];
        if is_transient(&failure) {
            item.attempts += 1;
            let at_ms = now_ms + retry_delay_ms(item.attempts);
            item.status = Status::Retrying {
                attempt: item.attempts,
                at_ms,
            };
            item.phase = Phase::Idle;
            effects.push(Effect::WakeAt { at_ms });
            return;
        }
        let code = match &failure {
            ApiFailure::Server(body) => body.code.as_str(),
            _ => "",
        };
        match (code, item.phase) {
            (ALREADY_EXISTS, Phase::Creating) if item.conflict == ConflictPolicy::Fail => {
                match decision.map(policy) {
                    Some(Some(conflict)) => {
                        item.conflict = conflict;
                        effects.push(item.create());
                    }
                    Some(None) => self.finish(index, Status::Skipped, now_ms, effects),
                    None => {
                        item.status = Status::NeedsDecision;
                        item.phase = Phase::Idle;
                    }
                }
            }
            (UPLOAD_NOT_FOUND, Phase::Heading | Phase::Patching) if !item.restarted => {
                item.restarted = true;
                item.upload_url = None;
                item.sent = 0;
                effects.push(item.create());
            }
            _ => self.finish(index, Status::Failed(failure), now_ms, effects),
        }
    }

    fn decide(
        &mut self,
        key: ItemKey,
        decision: Decision,
        apply_to_all: bool,
        now_ms: i64,
        effects: &mut Vec<Effect>,
    ) {
        let Some(index) = self
            .index(key)
            .filter(|&index| self.items[index].status == Status::NeedsDecision)
        else {
            return;
        };
        let batch = self.items[index].batch;
        let targets: Vec<usize> = if apply_to_all {
            self.decisions.insert(batch, decision);
            (0..self.items.len())
                .filter(|&i| {
                    self.items[i].batch == batch && self.items[i].status == Status::NeedsDecision
                })
                .collect()
        } else {
            vec![index]
        };
        for target in targets {
            match policy(decision) {
                Some(conflict) => {
                    let item = &mut self.items[target];
                    item.conflict = conflict;
                    item.status = Status::Queued;
                }
                None => self.finish(target, Status::Skipped, now_ms, effects),
            }
        }
    }

    fn cancel(&mut self, key: ItemKey, now_ms: i64, effects: &mut Vec<Effect>) {
        let Some(index) = self.index(key) else {
            return;
        };
        let item = &self.items[index];
        if !item.status.is_active() {
            return;
        }
        if item.status == Status::Uploading {
            effects.push(Effect::Abort { key });
        }
        if let Some(url) = item.upload_url.clone() {
            effects.push(Effect::Delete { url });
        }
        self.finish(index, Status::Cancelled, now_ms, effects);
    }

    /// Ends an item and refreshes its volume's listing when completions are due.
    fn finish(&mut self, index: usize, status: Status, now_ms: i64, effects: &mut Vec<Effect>) {
        let done = status == Status::Done;
        let item = &mut self.items[index];
        item.status = status;
        item.phase = Phase::Idle;
        let volume = item.volume().to_owned();
        if done {
            self.unrefreshed.insert(volume.clone());
        }
        if !self.unrefreshed.contains(&volume) {
            return;
        }
        let idle = !self
            .items
            .iter()
            .any(|item| item.status.is_active() && item.volume() == volume);
        let due = done
            && self
                .last_refresh
                .get(&volume)
                .is_none_or(|&last| now_ms - last >= REFRESH_EVERY_MS);
        if idle || due {
            self.unrefreshed.remove(&volume);
            self.last_refresh.insert(volume.clone(), now_ms);
            effects.push(Effect::Refresh { volume });
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::api::ErrorBody;

    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn spec(path: &str, size: u64) -> FileSpec {
        FileSpec {
            volume: "media".into(),
            path: path.into(),
            size,
            modified_ms: Some(1000),
        }
    }

    fn plan(files: Vec<FileSpec>) -> BatchPlan {
        BatchPlan {
            volume: "media".into(),
            files,
            folders: Vec::new(),
            skipped: 0,
        }
    }

    fn files(count: usize, size: u64) -> BatchPlan {
        plan((0..count).map(|i| spec(&format!("f{i}"), size)).collect())
    }

    fn server(code: &str) -> ApiFailure {
        ApiFailure::Server(ErrorBody {
            code: code.into(),
            message: String::new(),
            details: None,
            request_id: None,
        })
    }

    fn key(queue: &Queue, index: usize) -> ItemKey {
        queue.items()[index].key
    }

    fn url(key: ItemKey) -> String {
        format!("/files/uploads/u{}", key.0)
    }

    fn create(queue: &Queue, index: usize, conflict: ConflictPolicy) -> Effect {
        let Kind::File(spec) = &queue.items()[index].kind else {
            panic!("not a file");
        };
        Effect::Create {
            key: key(queue, index),
            spec: spec.clone(),
            conflict,
        }
    }

    fn created(queue: &mut Queue, index: usize, now_ms: i64) -> Vec<Effect> {
        let key = key(queue, index);
        queue.handle(Event::Created { key, url: url(key) }, now_ms)
    }

    fn fail(queue: &mut Queue, index: usize, failure: ApiFailure, now_ms: i64) -> Vec<Effect> {
        let key = key(queue, index);
        queue.handle(Event::Failed { key, failure }, now_ms)
    }

    fn refresh() -> Effect {
        Effect::Refresh {
            volume: "media".into(),
        }
    }

    fn uploading(queue: &Queue) -> usize {
        queue
            .items()
            .iter()
            .filter(|item| item.status == Status::Uploading)
            .count()
    }

    #[test]
    fn uploads_a_file_in_chunks() {
        let mut queue = Queue::default();
        let effects = queue.add(plan(vec![spec("a.bin", 40 * MIB)]), &[], 0);
        let key = key(&queue, 0);
        assert_eq!(effects, [create(&queue, 0, ConflictPolicy::Fail)]);

        let patch = |offset, len| Effect::Patch {
            key,
            url: url(key),
            offset,
            len,
        };
        assert_eq!(created(&mut queue, 0, 10), [patch(0, CHUNK_SIZE)]);
        queue.handle(Event::Progress { key, sent: MIB }, 15);
        assert_eq!(queue.items()[0].sent, MIB);
        let patched = |queue: &mut Queue, offset| queue.handle(Event::Patched { key, offset }, 20);
        assert_eq!(patched(&mut queue, 16 * MIB), [patch(16 * MIB, 16 * MIB)]);
        assert_eq!(patched(&mut queue, 32 * MIB), [patch(32 * MIB, 8 * MIB)]);
        assert_eq!(patched(&mut queue, 40 * MIB), [refresh()]);
        assert_eq!(queue.items()[0].status, Status::Done);
        assert_eq!(queue.items()[0].sent, 40 * MIB);
        assert!(!queue.is_active());
    }

    #[test]
    fn zero_byte_file_needs_no_patch() {
        let mut queue = Queue::default();
        queue.add(plan(vec![spec("empty.txt", 0)]), &[], 0);
        assert_eq!(created(&mut queue, 0, 5), [refresh()]);
        assert_eq!(queue.items()[0].status, Status::Done);
    }

    #[test]
    fn creates_folders_first() {
        let mut queue = Queue::default();
        let mut batch = files(1, 1);
        batch.folders = vec!["Empty".into()];
        let effects = queue.add(batch, &[], 0);
        let folder = key(&queue, 0);
        assert_eq!(
            effects,
            [
                Effect::CreateFolder {
                    key: folder,
                    volume: "media".into(),
                    path: "Empty".into(),
                },
                create(&queue, 1, ConflictPolicy::Fail),
            ]
        );
        assert_eq!(queue.batch_len(queue.items()[0].batch), 2);
        assert_eq!(
            queue.handle(Event::FolderCreated { key: folder }, 5),
            [refresh()]
        );
        assert_eq!(queue.items()[0].status, Status::Done);
    }

    #[test]
    fn runs_at_most_three_transfers() {
        let mut queue = Queue::default();
        let effects = queue.add(files(5, 10), &[], 0);
        assert_eq!(
            effects,
            [0, 1, 2].map(|i| create(&queue, i, ConflictPolicy::Fail))
        );
        assert_eq!(uploading(&queue), 3);
        assert_eq!(queue.items()[3].status, Status::Queued);

        created(&mut queue, 0, 1);
        let key0 = key(&queue, 0);
        let effects = queue.handle(
            Event::Patched {
                key: key0,
                offset: 10,
            },
            2,
        );
        assert_eq!(
            effects,
            [refresh(), create(&queue, 3, ConflictPolicy::Fail)]
        );
        assert_eq!(uploading(&queue), 3);
        assert_eq!(queue.items()[4].status, Status::Queued);
    }

    #[test]
    fn transient_patch_failure_resyncs_with_head() {
        let mut queue = Queue::default();
        queue.add(plan(vec![spec("a.bin", 40 * MIB)]), &[], 0);
        let key = key(&queue, 0);
        created(&mut queue, 0, 0);

        assert_eq!(
            fail(&mut queue, 0, ApiFailure::Network, 100),
            [Effect::WakeAt { at_ms: 1100 }]
        );
        assert_eq!(
            queue.items()[0].status,
            Status::Retrying {
                attempt: 1,
                at_ms: 1100
            }
        );
        assert!(queue.tick(1099).is_empty());
        assert_eq!(queue.tick(1100), [Effect::Head { key, url: url(key) }]);
        assert_eq!(
            queue.handle(
                Event::HeadDone {
                    key,
                    offset: 5 * MIB
                },
                1200
            ),
            [Effect::Patch {
                key,
                url: url(key),
                offset: 5 * MIB,
                len: CHUNK_SIZE,
            }]
        );

        assert_eq!(
            fail(&mut queue, 0, server("upload_busy"), 2000),
            [Effect::WakeAt { at_ms: 3000 }]
        );
        queue.tick(3000);
        assert_eq!(
            fail(&mut queue, 0, ApiFailure::Http(502), 4000),
            [Effect::WakeAt { at_ms: 6000 }]
        );
    }

    #[test]
    fn retrying_item_frees_its_slot() {
        let mut queue = Queue::default();
        queue.add(files(4, 10), &[], 0);
        assert_eq!(
            fail(&mut queue, 0, server("offset_mismatch"), 0),
            [
                Effect::WakeAt { at_ms: 1000 },
                create(&queue, 3, ConflictPolicy::Fail)
            ]
        );
        assert!(queue.tick(1000).is_empty());
        assert_eq!(queue.items()[0].status, Status::Queued);
    }

    #[test]
    fn fatal_failures_stop() {
        let mut queue = Queue::default();
        queue.add(files(3, 10), &[], 0);
        created(&mut queue, 0, 0);
        let storage = server("insufficient_storage");
        assert!(fail(&mut queue, 0, storage.clone(), 1).is_empty());
        assert!(fail(&mut queue, 1, server("permission_denied"), 1).is_empty());
        assert!(fail(&mut queue, 2, ApiFailure::Http(507), 1).is_empty());
        assert!(queue.tick(i64::MAX).is_empty());
        assert_eq!(queue.items()[0].status, Status::Failed(storage));
        assert_eq!(
            queue.items()[1].status,
            Status::Failed(server("permission_denied"))
        );
        assert!(!queue.is_active());
    }

    fn pending(modified_ms: Option<i64>) -> PendingUpload {
        PendingUpload {
            id: "abc def".into(),
            volume: "media".into(),
            path: "a.bin".into(),
            length: 40 * MIB,
            offset: 16 * MIB,
            modified_ms,
        }
    }

    #[test]
    fn resumes_matching_pending_upload() {
        let mut queue = Queue::default();
        let effects = queue.add(
            plan(vec![spec("a.bin", 40 * MIB)]),
            &[pending(Some(1000))],
            0,
        );
        let key = key(&queue, 0);
        let resumed = "/files/uploads/abc%20def".to_owned();
        assert_eq!(
            effects,
            [Effect::Head {
                key,
                url: resumed.clone()
            }]
        );
        assert_eq!(
            queue.items()[0].upload_url.as_deref(),
            Some(resumed.as_str())
        );
        assert_eq!(
            queue.handle(
                Event::HeadDone {
                    key,
                    offset: 16 * MIB
                },
                1
            ),
            [Effect::Patch {
                key,
                url: resumed,
                offset: 16 * MIB,
                len: 16 * MIB,
            }]
        );
    }

    #[test]
    fn changed_file_is_not_resumed() {
        let mut queue = Queue::default();
        let effects = queue.add(
            plan(vec![spec("a.bin", 40 * MIB)]),
            &[pending(Some(2000))],
            0,
        );
        assert_eq!(effects, [create(&queue, 0, ConflictPolicy::Fail)]);
        assert_eq!(queue.items()[0].upload_url, None);
    }

    #[test]
    fn expired_upload_restarts_once() {
        let mut queue = Queue::default();
        queue.add(
            plan(vec![spec("a.bin", 40 * MIB)]),
            &[pending(Some(1000))],
            0,
        );
        assert_eq!(
            fail(&mut queue, 0, server("upload_not_found"), 1),
            [create(&queue, 0, ConflictPolicy::Fail)]
        );
        assert_eq!(queue.items()[0].upload_url, None);
        assert_eq!(queue.items()[0].sent, 0);
        created(&mut queue, 0, 2);
        assert!(fail(&mut queue, 0, server("upload_not_found"), 3).is_empty());
        assert_eq!(
            queue.items()[0].status,
            Status::Failed(server("upload_not_found"))
        );
    }

    fn decide(
        queue: &mut Queue,
        index: usize,
        decision: Decision,
        apply_to_all: bool,
    ) -> Vec<Effect> {
        let key = key(queue, index);
        queue.handle(
            Event::Decide {
                key,
                decision,
                apply_to_all,
            },
            10,
        )
    }

    #[test]
    fn conflict_asks_then_applies_to_all() {
        let mut queue = Queue::default();
        queue.add(files(3, 10), &[], 0);
        for i in 0..3 {
            assert!(fail(&mut queue, i, server("already_exists"), 1).is_empty());
        }
        assert_eq!(queue.items()[2].status, Status::NeedsDecision);
        assert_eq!(
            queue.pending_decision().map(|item| item.key),
            Some(key(&queue, 0))
        );

        assert_eq!(
            decide(&mut queue, 0, Decision::KeepBoth, true),
            [0, 1, 2].map(|i| create(&queue, i, ConflictPolicy::KeepBoth))
        );
        assert_eq!(queue.items()[1].conflict, ConflictPolicy::KeepBoth);
        assert_eq!(queue.pending_decision(), None);

        assert!(fail(&mut queue, 0, server("already_exists"), 20).is_empty());
        assert_eq!(
            queue.items()[0].status,
            Status::Failed(server("already_exists"))
        );
        assert_eq!(queue.pending_decision(), None);
    }

    #[test]
    fn later_conflict_uses_the_batch_decision() {
        let mut queue = Queue::default();
        queue.add(files(5, 10), &[], 0);
        for i in 0..3 {
            fail(&mut queue, i, server("already_exists"), 1);
        }
        assert_eq!(uploading(&queue), 2);
        assert_eq!(
            decide(&mut queue, 0, Decision::Replace, true),
            [create(&queue, 0, ConflictPolicy::Replace)]
        );
        assert_eq!(
            fail(&mut queue, 3, server("already_exists"), 20),
            [create(&queue, 3, ConflictPolicy::Replace)]
        );
        assert_eq!(queue.pending_decision(), None);

        let mut queue = Queue::default();
        queue.add(files(5, 10), &[], 0);
        fail(&mut queue, 0, server("already_exists"), 1);
        decide(&mut queue, 0, Decision::Cancel, true);
        assert_eq!(
            fail(&mut queue, 1, server("already_exists"), 2),
            [create(&queue, 4, ConflictPolicy::Fail)]
        );
        assert_eq!(queue.items()[1].status, Status::Skipped);
        assert_eq!(queue.pending_decision(), None);
    }

    #[test]
    fn cancel_decision_skips() {
        let mut queue = Queue::default();
        queue.add(files(3, 10), &[], 0);
        for i in 0..3 {
            fail(&mut queue, i, server("already_exists"), 1);
        }
        assert!(decide(&mut queue, 0, Decision::Cancel, false).is_empty());
        assert_eq!(queue.items()[0].status, Status::Skipped);
        assert_eq!(
            queue.pending_decision().map(|item| item.key),
            Some(key(&queue, 1))
        );

        assert_eq!(
            decide(&mut queue, 1, Decision::Replace, false),
            [create(&queue, 1, ConflictPolicy::Replace)]
        );
        assert_eq!(
            queue.pending_decision().map(|item| item.key),
            Some(key(&queue, 2))
        );
        let key2 = key(&queue, 2);
        assert!(queue.handle(Event::Cancel { key: key2 }, 11).is_empty());
        assert_eq!(queue.items()[2].status, Status::Cancelled);
    }

    #[test]
    fn cancel_uploading_aborts_and_deletes() {
        let mut queue = Queue::default();
        queue.add(files(5, 10), &[], 0);
        let cancel = |queue: &mut Queue, index| {
            let key = key(queue, index);
            queue.handle(Event::Cancel { key }, 5)
        };

        assert!(cancel(&mut queue, 4).is_empty());
        assert_eq!(queue.items()[4].status, Status::Cancelled);

        created(&mut queue, 0, 1);
        let key0 = key(&queue, 0);
        assert_eq!(
            cancel(&mut queue, 0),
            [
                Effect::Abort { key: key0 },
                Effect::Delete { url: url(key0) },
                create(&queue, 3, ConflictPolicy::Fail),
            ]
        );
        assert_eq!(
            cancel(&mut queue, 1),
            [Effect::Abort {
                key: key(&queue, 1)
            }]
        );
        assert!(
            queue
                .handle(
                    Event::Patched {
                        key: key0,
                        offset: 10
                    },
                    6
                )
                .is_empty()
        );
        assert_eq!(queue.items()[0].status, Status::Cancelled);

        created(&mut queue, 2, 7);
        let key2 = key(&queue, 2);
        queue.handle(
            Event::Patched {
                key: key2,
                offset: 10,
            },
            8,
        );
        assert!(cancel(&mut queue, 2).is_empty());
        assert_eq!(queue.items()[2].status, Status::Done);

        assert_eq!(queue.batch_len(queue.items()[0].batch), 5);
        queue.clear_finished();
        assert_eq!(queue.items().len(), 1);
        assert!(queue.is_active());
    }

    #[test]
    fn refresh_is_throttled() {
        let mut queue = Queue::default();
        queue.add(files(10, 1), &[], 0);
        let mut refreshes = Vec::new();
        for i in 0..10 {
            let now = i64::try_from(i).unwrap() * 100;
            created(&mut queue, i, now);
            let key = key(&queue, i);
            let effects = queue.handle(Event::Patched { key, offset: 1 }, now);
            if effects.contains(&refresh()) {
                refreshes.push(now);
            }
        }
        assert_eq!(refreshes, [0, 900]);
    }
}
