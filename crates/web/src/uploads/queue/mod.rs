mod actions;
mod effects;
mod item;
mod refresh;
mod transfer;

use std::collections::{HashMap, HashSet};

use shared::api::conflict::ConflictPolicy;
use shared::api::uploads::PendingUpload;

use super::MAX_PARALLEL;
use super::plan::{BatchPlan, FileSpec};
use crate::api;
use crate::conflict::Decision;

pub use effects::{Effect, Event};
use item::Phase;
pub use item::{Item, ItemKey, Kind, Status};

/// Upload state machine: events in, requests to send out.
#[derive(Debug, Default)]
pub struct Queue {
    items: Vec<Item>,
    next_key: u64,
    next_batch: u64,
    decisions: HashMap<u64, Decision>,
    last_refresh: HashMap<String, i64>,
    unrefreshed: HashSet<String>,
    invalid_names: usize,
    unreadable: usize,
}

impl Queue {
    pub fn add(&mut self, plan: BatchPlan, pending: &[PendingUpload], _now_ms: i64) -> Vec<Effect> {
        let batch = self.next_batch;
        self.next_batch += 1;
        self.invalid_names += plan.skipped;
        self.unreadable += plan.unreadable;
        let folders = plan.folders.into_iter().map(|path| {
            let kind = Kind::Folder {
                volume: plan.volume.clone(),
                path,
            };
            (kind, None, 0)
        });
        let files = plan.files.into_iter().map(|spec| {
            let resumed = pending.iter().find(|upload| {
                may_resume(upload, &spec)
                    && upload.fingerprint.is_some()
                    && upload.fingerprint == spec.fingerprint
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
                confirmed: 0,
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
                    item.confirmed = 0;
                    self.advance(index, url, 0, now_ms, &mut effects);
                }
                Some(index) if self.items[index].status == Status::Cancelled => {
                    effects.push(Effect::Delete { key, url });
                }
                _ => {}
            },
            Event::Progress { key, sent } => {
                if let Some(index) = self.in_flight(key) {
                    self.items[index].sent = sent;
                }
            }
            Event::Patched { key, offset } => self.resume(key, offset, true, now_ms, &mut effects),
            Event::HeadDone { key, offset } => {
                self.resume(key, offset, false, now_ms, &mut effects);
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
            Event::CancelTooLate { key } => self.cancel_too_late(key, now_ms, &mut effects),
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
        let mut due: Vec<String> = self
            .unrefreshed
            .iter()
            .filter(|volume| self.refresh_due(volume, now_ms))
            .cloned()
            .collect();
        due.sort();
        for volume in due {
            self.refresh(volume, now_ms, &mut effects);
        }
        self.pump(&mut effects);
        effects
    }

    /// Picked or dropped items left out for their name, since the last clear.
    pub fn invalid_names(&self) -> usize {
        self.invalid_names
    }

    /// Picked or dropped items the browser could not read, since the last clear.
    pub fn unreadable(&self) -> usize {
        self.unreadable
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
        self.invalid_names = 0;
        self.unreadable = 0;
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
            .and_then(Decision::policy);
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
}

/// Whether `upload` may continue `spec` if their fingerprints match: the same destination, size
/// and date, and the policy a new item starts with, so that the user is still asked on a conflict.
pub fn may_resume(upload: &PendingUpload, spec: &FileSpec) -> bool {
    upload.volume == spec.volume
        && upload.path == spec.path
        && upload.length == spec.size
        && upload.modified_ms == spec.modified_ms
        && upload.conflict == ConflictPolicy::Fail
}

#[cfg(test)]
mod tests {
    use shared::api::error::ErrorBody;

    use super::*;
    use crate::api::ApiFailure;
    use crate::uploads::CHUNK_SIZE;

    const MIB: u64 = 1024 * 1024;
    const FINGERPRINT: &str = "0123456789abcdef";

    fn spec(path: &str, size: u64) -> FileSpec {
        FileSpec {
            volume: "media".into(),
            path: path.into(),
            size,
            modified_ms: Some(1000),
            fingerprint: Some(FINGERPRINT.into()),
        }
    }

    fn plan(files: Vec<FileSpec>) -> BatchPlan {
        BatchPlan {
            volume: "media".into(),
            files,
            folders: Vec::new(),
            skipped: 0,
            unreadable: 0,
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
                at_ms: 1100,
                connection_lost: true,
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
            [Effect::WakeAt { at_ms: 4000 }]
        );
        assert!(matches!(
            queue.items()[0].status,
            Status::Retrying {
                connection_lost: false,
                ..
            }
        ));
        queue.tick(4000);
        assert_eq!(
            fail(&mut queue, 0, ApiFailure::Http(502), 4000),
            [Effect::WakeAt { at_ms: 8000 }]
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
        let key0 = key(&queue, 0);
        let storage = server("insufficient_storage");
        assert_eq!(
            fail(&mut queue, 0, storage.clone(), 1),
            [Effect::Delete {
                key: key0,
                url: url(key0)
            }]
        );
        assert_eq!(queue.items()[0].upload_url, None);
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
            conflict: ConflictPolicy::Fail,
            fingerprint: Some(FINGERPRINT.into()),
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
    fn upload_started_with_another_policy_is_not_resumed() {
        let mut queue = Queue::default();
        let replace = PendingUpload {
            conflict: ConflictPolicy::Replace,
            ..pending(Some(1000))
        };
        let effects = queue.add(plan(vec![spec("a.bin", 40 * MIB)]), &[replace], 0);
        assert_eq!(effects, [create(&queue, 0, ConflictPolicy::Fail)]);
        assert_eq!(queue.items()[0].upload_url, None);
    }

    #[test]
    fn other_content_is_not_resumed() {
        let other = PendingUpload {
            fingerprint: Some("fedcba9876543210".into()),
            ..pending(Some(1000))
        };
        let unknown = PendingUpload {
            fingerprint: None,
            ..pending(Some(1000))
        };
        for upload in [other, unknown] {
            let mut queue = Queue::default();
            let effects = queue.add(plan(vec![spec("a.bin", 40 * MIB)]), &[upload], 0);
            assert_eq!(effects, [create(&queue, 0, ConflictPolicy::Fail)]);
        }
        let mut queue = Queue::default();
        let unread = FileSpec {
            fingerprint: None,
            ..spec("a.bin", 40 * MIB)
        };
        let effects = queue.add(plan(vec![unread]), &[pending(Some(1000))], 0);
        assert_eq!(effects, [create(&queue, 0, ConflictPolicy::Fail)]);
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
        // Nothing is left on the server to delete.
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
    fn batch_decision_is_forgotten_with_the_batch() {
        let mut queue = Queue::default();
        queue.add(files(2, 10), &[], 0);
        fail(&mut queue, 0, server("already_exists"), 1);
        decide(&mut queue, 0, Decision::Replace, true);
        assert_eq!(queue.decisions.len(), 1);
        fail(&mut queue, 0, server("permission_denied"), 20);
        assert_eq!(queue.decisions.len(), 1);
        let key1 = key(&queue, 1);
        queue.handle(Event::Cancel { key: key1 }, 21);
        assert!(queue.decisions.is_empty());

        queue.add(files(2, 10), &[], 30);
        for i in 2..4 {
            fail(&mut queue, i, server("already_exists"), 31);
        }
        decide(&mut queue, 2, Decision::Cancel, true);
        assert!(queue.decisions.is_empty());
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
                Effect::Delete {
                    key: key0,
                    url: url(key0)
                },
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

    #[test]
    fn repeated_patch_failures_back_off() {
        let mut queue = Queue::default();
        queue.add(plan(vec![spec("a.bin", 40 * MIB)]), &[], 0);
        let key = key(&queue, 0);
        created(&mut queue, 0, 0);
        let mut now = 0;
        for delay in [1000, 2000, 4000] {
            assert_eq!(
                fail(&mut queue, 0, ApiFailure::Network, now),
                [Effect::WakeAt { at_ms: now + delay }]
            );
            now += delay;
            assert_eq!(queue.tick(now), [Effect::Head { key, url: url(key) }]);
            queue.handle(Event::HeadDone { key, offset: 0 }, now);
        }

        queue.handle(
            Event::Patched {
                key,
                offset: 16 * MIB,
            },
            now,
        );
        assert_eq!(
            fail(&mut queue, 0, ApiFailure::Network, now),
            [Effect::WakeAt { at_ms: now + 1000 }]
        );
    }

    #[test]
    fn server_errors_are_transient_except_507() {
        let mut queue = Queue::default();
        queue.add(files(3, 10), &[], 0);
        assert_eq!(
            fail(&mut queue, 0, server("internal"), 0),
            [Effect::WakeAt { at_ms: 1000 }]
        );
        assert_eq!(
            fail(&mut queue, 1, ApiFailure::Http(503), 0),
            [Effect::WakeAt { at_ms: 1000 }]
        );
        assert!(fail(&mut queue, 2, server("insufficient_storage"), 0).is_empty());
        assert_eq!(
            queue.items()[2].status,
            Status::Failed(server("insufficient_storage"))
        );
    }

    #[test]
    fn too_many_patches_is_retried() {
        let mut queue = Queue::default();
        queue.add(files(1, 10), &[], 0);
        created(&mut queue, 0, 0);
        assert_eq!(
            fail(&mut queue, 0, server("too_many_patches"), 0),
            [Effect::WakeAt { at_ms: 1000 }]
        );
    }

    #[test]
    fn unauthenticated_fails() {
        let mut queue = Queue::default();
        queue.add(files(1, 10), &[], 0);
        assert!(fail(&mut queue, 0, server("unauthenticated"), 0).is_empty());
        assert_eq!(
            queue.items()[0].status,
            Status::Failed(server("unauthenticated"))
        );
    }

    #[test]
    fn offset_without_upload_url_fails() {
        let mut queue = Queue::default();
        queue.add(files(1, 10), &[], 0);
        let key = key(&queue, 0);
        queue.handle(Event::Patched { key, offset: 5 }, 0);
        assert_eq!(queue.items()[0].status, Status::Failed(ApiFailure::Network));
    }

    #[test]
    fn cancel_after_the_commit_shows_done() {
        let mut queue = Queue::default();
        queue.add(files(2, 10), &[], 0);
        created(&mut queue, 0, 0);
        let key0 = key(&queue, 0);
        queue.handle(Event::Cancel { key: key0 }, 1);
        assert_eq!(queue.items()[0].status, Status::Cancelled);

        let effects = queue.handle(Event::CancelTooLate { key: key0 }, 2);
        assert_eq!(queue.items()[0].status, Status::Done);
        assert_eq!(queue.items()[0].sent, 10);
        assert_eq!(effects, [refresh()]);

        // Only a cancelled item turns done.
        let key1 = key(&queue, 1);
        assert!(
            queue
                .handle(Event::CancelTooLate { key: key1 }, 3)
                .is_empty()
        );
        assert_eq!(queue.items()[1].status, Status::Uploading);
    }

    #[test]
    fn late_created_after_cancel_deletes() {
        let mut queue = Queue::default();
        queue.add(files(1, 10), &[], 0);
        let key = key(&queue, 0);
        assert_eq!(
            queue.handle(Event::Cancel { key }, 0),
            [Effect::Abort { key }]
        );
        assert_eq!(
            created(&mut queue, 0, 1),
            [Effect::Delete { key, url: url(key) }]
        );
        assert_eq!(queue.items()[0].status, Status::Cancelled);
    }

    #[test]
    fn folder_retries_after_transient_failure() {
        let mut queue = Queue::default();
        let mut batch = files(0, 0);
        batch.folders = vec!["Empty".into()];
        queue.add(batch, &[], 0);
        assert_eq!(
            fail(&mut queue, 0, ApiFailure::Network, 0),
            [Effect::WakeAt { at_ms: 1000 }]
        );
        assert_eq!(
            queue.tick(1000),
            [Effect::CreateFolder {
                key: key(&queue, 0),
                volume: "media".into(),
                path: "Empty".into(),
            }]
        );
    }

    #[test]
    fn head_at_full_length_confirms_with_empty_patch() {
        let mut queue = Queue::default();
        queue.add(plan(vec![spec("a.bin", 10)]), &[], 0);
        let key = key(&queue, 0);
        created(&mut queue, 0, 0);
        fail(&mut queue, 0, server("internal"), 1);
        assert_eq!(queue.tick(1001), [Effect::Head { key, url: url(key) }]);

        let empty_patch = Effect::Patch {
            key,
            url: url(key),
            offset: 10,
            len: 0,
        };
        assert_eq!(
            queue.handle(Event::HeadDone { key, offset: 10 }, 1002),
            std::slice::from_ref(&empty_patch)
        );
        assert_eq!(queue.items()[0].status, Status::Uploading);
        assert_eq!(queue.items()[0].sent, 10);

        assert_eq!(
            fail(&mut queue, 0, ApiFailure::Network, 1003),
            [Effect::WakeAt { at_ms: 3003 }]
        );
        queue.tick(3003);
        assert_eq!(
            queue.handle(Event::HeadDone { key, offset: 10 }, 3004),
            [empty_patch]
        );
        assert_eq!(
            queue.handle(Event::Patched { key, offset: 10 }, 3005),
            [refresh()]
        );
        assert_eq!(queue.items()[0].status, Status::Done);
    }

    #[test]
    fn resumed_complete_upload_is_confirmed_before_done() {
        let mut queue = Queue::default();
        let mut complete = pending(Some(1000));
        complete.offset = complete.length;
        queue.add(plan(vec![spec("a.bin", 40 * MIB)]), &[complete], 0);
        let key = key(&queue, 0);
        let resumed = "/files/uploads/abc%20def".to_owned();
        assert_eq!(
            queue.handle(
                Event::HeadDone {
                    key,
                    offset: 40 * MIB
                },
                1
            ),
            [Effect::Patch {
                key,
                url: resumed,
                offset: 40 * MIB,
                len: 0,
            }]
        );
        assert_eq!(queue.items()[0].status, Status::Uploading);
    }

    #[test]
    fn fixable_commit_failure_keeps_the_staged_file() {
        for code in ["permission_denied", "already_exists", "path_not_found"] {
            let mut queue = Queue::default();
            queue.add(plan(vec![spec("a.bin", 20 * MIB)]), &[], 0);
            let key = key(&queue, 0);
            created(&mut queue, 0, 0);
            queue.handle(
                Event::Patched {
                    key,
                    offset: 16 * MIB,
                },
                1,
            );
            assert!(fail(&mut queue, 0, server(code), 2).is_empty());
            assert_eq!(queue.items()[0].status, Status::Failed(server(code)));
        }
    }

    #[test]
    fn fixable_failure_before_the_last_chunk_deletes() {
        let mut queue = Queue::default();
        queue.add(plan(vec![spec("a.bin", 20 * MIB)]), &[], 0);
        let key = key(&queue, 0);
        created(&mut queue, 0, 0);
        assert_eq!(
            fail(&mut queue, 0, server("permission_denied"), 1),
            [Effect::Delete { key, url: url(key) }]
        );
    }

    #[test]
    fn cross_device_commit_fails() {
        let mut queue = Queue::default();
        queue.add(files(1, 10), &[], 0);
        created(&mut queue, 0, 0);
        let key = key(&queue, 0);
        assert_eq!(
            fail(&mut queue, 0, server("cross_device"), 1),
            [Effect::Delete { key, url: url(key) }]
        );
        assert_eq!(
            queue.items()[0].status,
            Status::Failed(server("cross_device"))
        );
        assert!(queue.tick(i64::MAX).is_empty());
    }

    #[test]
    fn throttled_completion_refreshes_when_due() {
        let mut queue = Queue::default();
        queue.add(files(3, 1), &[], 0);
        let finish = |queue: &mut Queue, index, now| {
            created(queue, index, now);
            let key = key(queue, index);
            queue.handle(Event::Patched { key, offset: 1 }, now)
        };
        assert_eq!(finish(&mut queue, 0, 100), [refresh()]);
        assert_eq!(finish(&mut queue, 1, 500), [Effect::WakeAt { at_ms: 2100 }]);
        assert!(queue.is_active());
        assert!(queue.tick(2099).is_empty());
        assert_eq!(queue.tick(2100), [refresh()]);
        assert!(queue.tick(5000).is_empty());

        assert_eq!(finish(&mut queue, 2, 5000), [refresh()]);
    }

    #[test]
    fn skipped_items_are_counted_until_cleared() {
        let mut queue = Queue::default();
        let mut batch = files(1, 1);
        batch.skipped = 2;
        batch.unreadable = 4;
        queue.add(batch, &[], 0);
        let mut batch = files(0, 0);
        batch.skipped = 1;
        queue.add(batch, &[], 0);
        assert_eq!(queue.invalid_names(), 3);
        assert_eq!(queue.unreadable(), 4);
        queue.clear_finished();
        assert_eq!(queue.invalid_names(), 0);
        assert_eq!(queue.unreadable(), 0);
        assert_eq!(queue.items().len(), 1);
    }
}
