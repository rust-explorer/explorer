use shared::api::tasks::{Location, TaskSpec, TaskState};

use super::store::{Effect, Task, TaskList};

/// The server keeps the results of a user's newest finished scans only, one per folder: see
/// `MAX_USAGE_TREES` in `crates/server/src/tasks/mod.rs`.
pub const MAX_USAGE_TREES: usize = 4;

/// A folder's size as this tab's usage tasks know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderSize {
    Unknown,
    Computing,
    Known(u64),
}

impl TaskList {
    /// The newest queued, running or finished usage task of exactly `volume`/`path`.
    pub fn newest_usage(&self, volume: &str, path: &str) -> Option<&Task> {
        self.tasks().iter().rev().find(|task| {
            matches!(&task.spec, TaskSpec::Usage { source } if source.volume == volume && source.path == path)
                && matches!(
                    task.state(),
                    TaskState::Queued | TaskState::Running | TaskState::Done
                )
        })
    }

    pub fn folder_size(&self, volume: &str, path: &str) -> FolderSize {
        match self.newest_usage(volume, path) {
            None => FolderSize::Unknown,
            Some(task) => match (&task.info, task.state()) {
                (Some(info), TaskState::Done) => FolderSize::Known(info.bytes_done),
                _ => FolderSize::Computing,
            },
        }
    }

    /// Whether the server may still have the result of the finished scan `task`: it is among
    /// the [`MAX_USAGE_TREES`] newest finished scans of distinct folders this tab knows.
    fn result_kept(&self, task: &Task) -> bool {
        let mut newer: Vec<&Location> = Vec::new();
        for other in self.tasks().iter().rev() {
            if other.state() != TaskState::Done {
                continue;
            }
            let TaskSpec::Usage { source } = &other.spec else {
                continue;
            };
            if other.key == task.key {
                return newer.len() < MAX_USAGE_TREES;
            }
            if !newer.contains(&source) {
                newer.push(source);
            }
        }
        false
    }

    /// Opens the newest usage task of `source` when there is one, otherwise starts one; with
    /// `open`, its page opens once its id is known. A scan still going on is always reused, a
    /// finished one only without `force_new` and while the server likely keeps its result.
    pub fn analyze(&mut self, source: Location, open: bool, force_new: bool) -> Vec<Effect> {
        let reused = self
            .newest_usage(&source.volume, &source.path)
            .filter(|task| task.is_active() || (!force_new && self.result_kept(task)))
            .map(|task| (task.key, task.server_id().map(str::to_owned)));
        match reused {
            Some((_, Some(id))) => {
                if open {
                    vec![Effect::Open { id, source }]
                } else {
                    Vec::new()
                }
            }
            Some((key, None)) => {
                if let Some(task) = self.task_mut(key) {
                    task.open_when_posted |= open;
                }
                Vec::new()
            }
            None => {
                let effects = self.start_batch(vec![(TaskSpec::Usage { source }, None)]);
                if let Some(task) = self.tasks_mut().last_mut() {
                    task.open_when_posted = open;
                }
                effects
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use shared::api::tasks::TaskInfo;

    use super::super::store::{Event, TaskKey};
    use super::*;

    fn location(path: &str) -> Location {
        Location {
            volume: "media".to_owned(),
            path: path.to_owned(),
        }
    }

    fn usage(path: &str) -> TaskSpec {
        TaskSpec::Usage {
            source: location(path),
        }
    }

    fn info(id: &str, path: &str, state: TaskState, bytes: u64) -> TaskInfo {
        TaskInfo {
            id: id.to_owned(),
            spec: usage(path),
            state,
            files_done: 3,
            files_total: None,
            bytes_done: bytes,
            bytes_total: None,
            skipped: 0,
            final_path: None,
            error: None,
        }
    }

    fn post(path: &str, key: u64) -> Effect {
        Effect::Post {
            key: TaskKey(key),
            spec: usage(path),
        }
    }

    fn open(id: &str, path: &str) -> Effect {
        Effect::Open {
            id: id.to_owned(),
            source: location(path),
        }
    }

    /// Starts a scan of `path` and has the server answer with `info`.
    fn scanned(list: &mut TaskList, info: TaskInfo) {
        let path = info.spec.source().path.clone();
        let effects = list.analyze(location(&path), false, true);
        let Some(Effect::Post { key, .. }) = effects.first() else {
            panic!("no post: {effects:?}");
        };
        list.handle(Event::Posted {
            key: *key,
            info: Box::new(info),
        });
    }

    #[test]
    fn folder_size_follows_the_newest_usage_task() {
        let mut list = TaskList::default();
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Unknown);

        scanned(&mut list, info("a", "Films", TaskState::Running, 10));
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Computing);
        assert_eq!(list.folder_size("media", "Films/x"), FolderSize::Unknown);
        assert_eq!(list.folder_size("other", "Films"), FolderSize::Unknown);

        list.handle(Event::Polled(vec![info(
            "a",
            "Films",
            TaskState::Done,
            500,
        )]));
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Known(500));

        scanned(&mut list, info("b", "Films", TaskState::Running, 1));
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Computing);
        list.handle(Event::Polled(vec![info(
            "b",
            "Films",
            TaskState::Done,
            700,
        )]));
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Known(700));
    }

    #[test]
    fn folder_size_skips_failed_and_cancelled_scans() {
        let mut list = TaskList::default();
        scanned(&mut list, info("a", "Films", TaskState::Done, 500));
        scanned(&mut list, info("b", "Films", TaskState::Cancelled, 1));
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Known(500));
    }

    #[test]
    fn finished_scans_are_stamped_once() {
        let mut list = TaskList::default();
        scanned(&mut list, info("a", "Films", TaskState::Running, 10));
        list.stamp_finished(5);
        assert_eq!(
            list.newest_usage("media", "Films")
                .unwrap()
                .finished_at_ms(),
            None
        );
        list.handle(Event::Polled(vec![info(
            "a",
            "Films",
            TaskState::Done,
            500,
        )]));
        list.stamp_finished(7);
        list.stamp_finished(9);
        assert_eq!(
            list.newest_usage("media", "Films")
                .unwrap()
                .finished_at_ms(),
            Some(7)
        );
    }

    #[test]
    fn analyze_starts_a_scan_and_opens_it_once_posted() {
        let mut list = TaskList::default();
        assert_eq!(
            list.analyze(location("Films"), true, false),
            [post("Films", 0)]
        );
        let effects = list.handle(Event::Posted {
            key: TaskKey(0),
            info: Box::new(info("a", "Films", TaskState::Queued, 0)),
        });
        assert_eq!(
            effects,
            [
                open("a", "Films"),
                Effect::PollAfter {
                    ms: super::super::store::POLL_EVERY_MS
                }
            ]
        );
    }

    #[test]
    fn analyze_reuses_a_running_or_done_scan() {
        let mut list = TaskList::default();
        scanned(&mut list, info("a", "Films", TaskState::Running, 10));
        assert_eq!(
            list.analyze(location("Films"), true, false),
            [open("a", "Films")]
        );
        assert_eq!(list.analyze(location("Films"), false, false), []);

        list.handle(Event::Polled(vec![info(
            "a",
            "Films",
            TaskState::Done,
            500,
        )]));
        assert_eq!(
            list.analyze(location("Films"), true, false),
            [open("a", "Films")]
        );
        assert_eq!(list.tasks().len(), 1);

        // Another folder, or a forced rescan, starts a new one.
        assert_eq!(
            list.analyze(location("Films/x"), true, false),
            [post("Films/x", 1)]
        );
        assert_eq!(
            list.analyze(location("Films"), true, true),
            [post("Films", 2)]
        );
    }

    #[test]
    fn a_forced_rescan_reuses_one_still_going_on() {
        let mut list = TaskList::default();
        assert_eq!(
            list.analyze(location("Films"), false, true),
            [post("Films", 0)]
        );
        // A double tap while the first one is being created.
        assert_eq!(list.analyze(location("Films"), true, true), []);
        let effects = list.handle(Event::Posted {
            key: TaskKey(0),
            info: Box::new(info("a", "Films", TaskState::Running, 0)),
        });
        assert_eq!(effects.first(), Some(&open("a", "Films")));
        assert_eq!(
            list.analyze(location("Films"), true, true),
            [open("a", "Films")]
        );
        assert_eq!(list.tasks().len(), 1);
    }

    #[test]
    fn analyze_rescans_once_the_server_dropped_the_result() {
        let mut list = TaskList::default();
        for (id, path) in [("a", "A"), ("b", "B"), ("c", "C"), ("d", "D")] {
            scanned(&mut list, info(id, path, TaskState::Done, 1));
        }
        // A rescan of a folder replaces its result, so it does not push another one out.
        scanned(&mut list, info("d2", "D", TaskState::Done, 1));
        assert_eq!(list.analyze(location("A"), true, false), [open("a", "A")]);

        scanned(&mut list, info("e", "E", TaskState::Done, 1));
        assert_eq!(list.analyze(location("B"), true, false), [open("b", "B")]);
        assert_eq!(list.analyze(location("A"), true, false), [post("A", 6)]);
        // Its size is still known.
        assert_eq!(list.folder_size("media", "B"), FolderSize::Known(1));
    }

    #[test]
    fn cleared_scans_leave_the_panel_but_keep_their_result() {
        let mut list = TaskList::default();
        scanned(&mut list, info("a", "Films", TaskState::Done, 500));
        scanned(&mut list, info("b", "Music", TaskState::Cancelled, 0));
        scanned(&mut list, info("c", "Photos", TaskState::Running, 0));
        assert_eq!(
            list.clear_finished(),
            [Effect::Delete { id: "b".to_owned() }],
            "the server keeps the finished scan's result"
        );
        let shown: Vec<&str> = list
            .shown()
            .filter_map(|task| Some(task.info.as_ref()?.id.as_str()))
            .collect();
        assert_eq!(shown, ["c"]);
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Known(500));
        assert_eq!(
            list.analyze(location("Films"), true, false),
            [open("a", "Films")]
        );
        assert_eq!(list.clear_finished(), []);
    }

    #[test]
    fn analyze_waits_for_the_id_of_a_scan_being_created() {
        let mut list = TaskList::default();
        assert_eq!(
            list.analyze(location("Films"), false, false),
            [post("Films", 0)]
        );
        assert_eq!(list.analyze(location("Films"), true, false), []);
        let effects = list.handle(Event::Posted {
            key: TaskKey(0),
            info: Box::new(info("a", "Films", TaskState::Queued, 0)),
        });
        assert_eq!(effects.first(), Some(&open("a", "Films")));
    }

    #[test]
    fn a_finished_scan_does_not_refresh_the_listing() {
        let mut list = TaskList::default();
        scanned(&mut list, info("a", "Films", TaskState::Running, 10));
        let effects = list.handle(Event::Polled(vec![info(
            "a",
            "Films",
            TaskState::Done,
            500,
        )]));
        assert_eq!(effects, []);

        list.analyze(location("Music"), false, true);
        let effects = list.handle(Event::Posted {
            key: TaskKey(1),
            info: Box::new(info("b", "Music", TaskState::Done, 1)),
        });
        assert_eq!(effects, []);
    }

    #[test]
    fn finished_scans_are_adopted_out_of_the_panel() {
        let mut list = TaskList::default();
        let effects = list.adopt(vec![
            info("a", "Films", TaskState::Done, 500),
            info("b", "Music", TaskState::Failed, 0),
            info("c", "Photos", TaskState::Running, 0),
        ]);
        assert_eq!(
            effects,
            [Effect::PollAfter {
                ms: super::super::store::POLL_EVERY_MS
            }]
        );
        assert_eq!(list.folder_size("media", "Films"), FolderSize::Known(500));
        assert_eq!(list.folder_size("media", "Music"), FolderSize::Unknown);
        let shown: Vec<&str> = list
            .shown()
            .filter_map(|task| Some(task.info.as_ref()?.id.as_str()))
            .collect();
        assert_eq!(shown, ["c"]);
        assert_eq!(
            list.analyze(location("Films"), true, false),
            [open("a", "Films")]
        );
        assert_eq!(
            list.adopt(vec![info("a", "Films", TaskState::Done, 500)]),
            []
        );
        assert_eq!(list.tasks().len(), 2);
    }

    #[test]
    fn analyze_starts_over_after_a_failed_scan() {
        let mut list = TaskList::default();
        scanned(&mut list, info("a", "Films", TaskState::Failed, 0));
        assert_eq!(
            list.analyze(location("Films"), true, false),
            [post("Films", 1)]
        );
    }
}
