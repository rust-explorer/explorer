mod conflict;
mod store;
#[cfg(target_arch = "wasm32")]
mod transport;
mod usage;

use dioxus::prelude::*;
use shared::api::browse::EntryKind;
use shared::api::tasks::{Location, TaskSpec};

pub use store::{Effect, Event, Task, TaskKey, TaskList};
pub use usage::FolderSize;

use crate::conflict::Decision;
use crate::revision::Revision;

/// The app-wide list of the background tasks this tab started, plus those still running from
/// before a reload.
#[derive(Clone, Copy)]
pub struct TaskStore {
    list: Signal<TaskList>,
    revision: Signal<u64>,
}

pub fn use_tasks_provider() -> TaskStore {
    let Revision(revision) = use_context();
    let store = use_context_provider(|| TaskStore {
        list: Signal::new(TaskList::default()),
        revision,
    });
    #[cfg(target_arch = "wasm32")]
    use_hook(|| store.adopt_running());
    store
}

pub fn use_tasks() -> TaskStore {
    use_context()
}

impl TaskStore {
    pub fn list(&self) -> Signal<TaskList> {
        self.list
    }

    pub fn start(mut self, specs: Vec<TaskSpec>) {
        let effects = self.list.write().start(specs);
        self.run(effects);
    }

    /// One user action's copies or moves, sharing "apply to all".
    pub fn start_transfers(mut self, specs: Vec<(TaskSpec, EntryKind)>) {
        let effects = self.list.write().start_transfers(specs);
        self.run(effects);
    }

    /// Opens the newest queued, running or finished usage task of exactly `source` if there is
    /// one, otherwise starts one; with `open`, navigates to its usage page once its id is known.
    pub fn analyze(mut self, source: Location, open: bool, force_new: bool) {
        let effects = self.list.write().analyze(source, open, force_new);
        self.run(effects);
    }

    /// Scans `source` anew and navigates to its usage page at `at`, a folder under it.
    pub fn rescan_at(mut self, source: Location, at: String) {
        let effects = self.list.write().analyze_at(source, Some(at), true);
        self.run(effects);
    }

    pub fn decide(mut self, key: TaskKey, decision: Decision, apply_to_all: bool) {
        let effects = self.list.write().decide(key, decision, apply_to_all);
        self.run(effects);
    }

    pub fn cancel(mut self, key: TaskKey) {
        let effects = self.list.write().cancel(key);
        self.run(effects);
    }

    pub fn clear_finished(mut self) {
        let effects = self.list.write().clear_finished();
        self.run(effects);
    }

    #[cfg(target_arch = "wasm32")]
    fn adopt_running(mut self) {
        dioxus::core::spawn_forever(async move {
            if let Ok(infos) = transport::list().await {
                let effects = self.list.write().adopt(infos);
                self.run(effects);
            }
        });
    }

    fn dispatch(mut self, event: Event) {
        let effects = {
            let mut list = self.list.write();
            let effects = list.handle(event);
            list.stamp_finished(crate::format::now_ms());
            effects
        };
        self.run(effects);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run(mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Refresh => *self.revision.write() += 1,
                Effect::Open { id, at } => {
                    crate::routes::go(crate::routes::usage_route(&id, &at));
                }
                Effect::Post { .. } | Effect::Delete { .. } | Effect::PollAfter { .. } => {}
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn run(mut self, effects: Vec<Effect>) {
        use dioxus::core::spawn_forever;
        use gloo_timers::future::TimeoutFuture;

        for effect in effects {
            match effect {
                Effect::Post { key, spec } => {
                    spawn_forever(async move {
                        self.dispatch(match transport::create(&spec).await {
                            Ok(info) => Event::Posted {
                                key,
                                info: Box::new(info),
                            },
                            Err(failure) => Event::PostFailed { key, failure },
                        });
                    });
                }
                Effect::Delete { id } => {
                    spawn_forever(async move {
                        let _ = transport::delete(&id).await;
                    });
                }
                Effect::PollAfter { ms } => {
                    spawn_forever(async move {
                        TimeoutFuture::new(ms).await;
                        self.dispatch(match transport::list().await {
                            Ok(infos) => Event::Polled(infos),
                            Err(failure) => Event::PollFailed(failure),
                        });
                    });
                }
                Effect::Refresh => *self.revision.write() += 1,
                Effect::Open { id, at } => {
                    crate::routes::go(crate::routes::usage_route(&id, &at));
                }
            }
        }
    }
}
