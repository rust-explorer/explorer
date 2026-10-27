mod conflict;
mod store;
#[cfg(target_arch = "wasm32")]
mod transport;

use dioxus::prelude::*;
use shared::api::browse::EntryKind;
use shared::api::tasks::TaskSpec;

pub use store::{Effect, Event, Task, TaskKey, TaskList};

use crate::conflict::Decision;
use crate::revision::Revision;

/// The app-wide list of the background tasks this tab started.
#[derive(Clone, Copy)]
pub struct TaskStore {
    list: Signal<TaskList>,
    revision: Signal<u64>,
}

pub fn use_tasks_provider() -> TaskStore {
    let Revision(revision) = use_context();
    use_context_provider(|| TaskStore {
        list: Signal::new(TaskList::default()),
        revision,
    })
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

    fn dispatch(mut self, event: Event) {
        let effects = self.list.write().handle(event);
        self.run(effects);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run(mut self, effects: Vec<Effect>) {
        for effect in effects {
            if effect == Effect::Refresh {
                *self.revision.write() += 1;
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
            }
        }
    }
}
