use std::collections::HashMap;

use dioxus::prelude::*;

use super::{Event, ItemKey, Queue, SpeedMeter};
use crate::conflict::Decision;
use crate::format::now_ms;

/// Bumped by `Effect::Refresh`; the browse page's volume and listing resources read it.
#[derive(Clone, Copy)]
pub struct Revision(pub Signal<u64>);

/// The app-wide upload queue and the browser state its transfers need.
#[derive(Clone, Copy)]
pub struct Uploads {
    queue: Signal<Queue>,
    meters: CopyValue<HashMap<ItemKey, SpeedMeter>>,
    revision: Signal<u64>,
    #[cfg(target_arch = "wasm32")]
    files: CopyValue<HashMap<ItemKey, web_sys::File>>,
    #[cfg(target_arch = "wasm32")]
    requests: CopyValue<HashMap<ItemKey, web_sys::XmlHttpRequest>>,
}

pub fn use_uploads_provider() -> Uploads {
    let revision = use_context_provider(|| Revision(Signal::new(0)));
    let uploads = use_context_provider(|| Uploads {
        queue: Signal::new(Queue::default()),
        meters: CopyValue::new(HashMap::new()),
        revision: revision.0,
        #[cfg(target_arch = "wasm32")]
        files: CopyValue::new(HashMap::new()),
        #[cfg(target_arch = "wasm32")]
        requests: CopyValue::new(HashMap::new()),
    });
    #[cfg(target_arch = "wasm32")]
    {
        use_leave_guard(uploads.queue);
        use_stray_drop_guard();
    }
    uploads
}

pub fn use_uploads() -> Uploads {
    use_context()
}

impl Uploads {
    pub fn queue(&self) -> Signal<Queue> {
        self.queue
    }

    pub fn speed(&self, key: ItemKey) -> Option<u64> {
        self.meters.read().get(&key)?.bytes_per_sec()
    }

    pub fn decide(&self, key: ItemKey, decision: Decision, apply_to_all: bool) {
        self.dispatch(Event::Decide {
            key,
            decision,
            apply_to_all,
        });
    }

    pub fn cancel(&self, key: ItemKey) {
        self.dispatch(Event::Cancel { key });
    }

    pub fn clear_finished(&self) {
        let mut uploads = *self;
        let mut queue = uploads.queue.write();
        queue.clear_finished();
        let kept = |key: &ItemKey| queue.items().iter().any(|item| item.key == *key);
        uploads.meters.write().retain(|key, _| kept(key));
        #[cfg(target_arch = "wasm32")]
        uploads.files.write().retain(|key, _| kept(key));
    }

    fn dispatch(mut self, event: Event) {
        let effects = self.queue.write().handle(event, now_ms());
        self.run(effects);
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn tick(mut self) {
        let effects = self.queue.write().tick(now_ms());
        self.run(effects);
    }

    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn progress(mut self, key: ItemKey, sent: u64) {
        self.meters
            .write()
            .entry(key)
            .or_default()
            .record(now_ms(), sent);
        self.dispatch(Event::Progress { key, sent });
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn run(mut self, effects: Vec<super::Effect>) {
        for effect in effects {
            if let super::Effect::Refresh { .. } = effect {
                *self.revision.write() += 1;
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::future::Future;
    use std::rc::Rc;

    use dioxus::core::{Runtime, spawn_forever};
    use dioxus::prelude::*;
    use gloo_timers::future::TimeoutFuture;
    use shared::api::uploads::PendingUpload;
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;
    use web_sys::{BeforeUnloadEvent, DragEvent, File};

    use super::Uploads;
    use crate::api::{self, ApiFailure};
    use crate::format::now_ms;
    use crate::uploads::transport::{self, Patch};
    use crate::uploads::{
        BatchPlan, Effect, Event, ItemKey, Kind, PickedFile, Queue, plan_batch, retry_delay_ms,
    };

    const UPLOAD_BUSY: &str = "upload_busy";
    const DELETE_ATTEMPTS: u32 = 5;

    impl Uploads {
        /// Uploads `files` into `base` of `volume`, resuming any the server already has.
        pub fn start(&self, volume: &str, base: &str, files: Vec<File>, empty_dirs: Vec<String>) {
            let files = files
                .into_iter()
                .map(|file| {
                    let relative = relative_path(&file);
                    (file, relative)
                })
                .collect();
            self.start_relative(volume, base, files, empty_dirs, 0);
        }

        /// Like `start`, with each file's path relative to `base` given, as for a dropped tree,
        /// and the count of items that could not be read.
        pub fn start_relative(
            &self,
            volume: &str,
            base: &str,
            files: Vec<(File, String)>,
            empty_dirs: Vec<String>,
            unreadable: usize,
        ) {
            let uploads = *self;
            let volume = volume.to_owned();
            let base = base.to_owned();
            spawn_forever(async move {
                let pending = match api::get_json::<Vec<PendingUpload>>("/api/uploads").await {
                    Ok(pending) => pending,
                    Err(failure) if failure.is_unauthenticated() => return,
                    Err(_) => Vec::new(),
                };
                let picked: Vec<(File, PickedFile)> = files
                    .into_iter()
                    .map(|(file, relative)| {
                        let picked = picked_file(&file, relative);
                        (file, picked)
                    })
                    .collect();
                // `plan_batch` drops invalid names; planning a file alone tells whether it stays.
                let kept: Vec<File> = picked
                    .iter()
                    .filter(|(_, picked)| {
                        !plan_batch(&volume, &base, vec![picked.clone()], Vec::new())
                            .files
                            .is_empty()
                    })
                    .map(|(file, _)| file.clone())
                    .collect();
                let picked = picked.into_iter().map(|(_, picked)| picked).collect();
                let mut plan = plan_batch(&volume, &base, picked, empty_dirs);
                plan.skipped += unreadable;
                uploads.add(plan, &pending, kept);
            });
        }

        /// `files` are the browser files of `plan.files`, in the same order.
        fn add(mut self, plan: BatchPlan, pending: &[PendingUpload], files: Vec<File>) {
            let effects = {
                let mut queue = self.queue.write();
                let before = queue.items().len();
                let effects = queue.add(plan, pending, now_ms());
                let keys = queue.items()[before..]
                    .iter()
                    .filter(|item| matches!(item.kind, Kind::File(_)))
                    .map(|item| item.key);
                self.files.write().extend(keys.zip(files));
                effects
            };
            self.run(effects);
        }

        pub(super) fn run(mut self, effects: Vec<Effect>) {
            for effect in effects {
                match effect {
                    Effect::CreateFolder { key, volume, path } => self.request(key, async move {
                        transport::create_folder(&volume, &path).await?;
                        Ok(Event::FolderCreated { key })
                    }),
                    Effect::Create {
                        key,
                        spec,
                        conflict,
                    } => self.request(key, async move {
                        let url = transport::create(&spec, conflict).await?;
                        Ok(Event::Created { key, url })
                    }),
                    Effect::Head { key, url } => self.request(key, async move {
                        let offset = transport::head(&url).await?;
                        Ok(Event::HeadDone { key, offset })
                    }),
                    Effect::Patch {
                        key,
                        url,
                        offset,
                        len,
                    } => self.patch(key, &url, offset, len),
                    Effect::Abort { key } => {
                        if let Some(xhr) = self.requests.write().remove(&key) {
                            let _ = xhr.abort();
                        }
                    }
                    Effect::Delete { url } => {
                        spawn_forever(delete(url));
                    }
                    Effect::WakeAt { at_ms } => {
                        let delay = u32::try_from((at_ms - now_ms()).max(0)).unwrap_or(u32::MAX);
                        spawn_forever(async move {
                            TimeoutFuture::new(delay).await;
                            self.tick();
                        });
                    }
                    Effect::Refresh { .. } => *self.revision.write() += 1,
                }
            }
        }

        fn request(
            self,
            key: ItemKey,
            request: impl Future<Output = Result<Event, ApiFailure>> + 'static,
        ) {
            spawn_forever(async move {
                let event = request
                    .await
                    .unwrap_or_else(|failure| Event::Failed { key, failure });
                self.dispatch(event);
            });
        }

        fn patch(mut self, key: ItemKey, url: &str, offset: u64, len: u64) {
            let Some(file) = self.files.read().get(&key).cloned() else {
                self.dispatch(Event::Failed {
                    key,
                    failure: ApiFailure::Http(0),
                });
                return;
            };
            // XHR calls it from outside the Dioxus runtime.
            let on_progress =
                Runtime::wrap_closure(move |loaded| self.progress(key, offset + loaded));
            let patch = match Patch::send(&file, url, offset, len, on_progress) {
                Ok(patch) => patch,
                Err(failure) => {
                    self.dispatch(Event::Failed { key, failure });
                    return;
                }
            };
            self.requests.write().insert(key, patch.request());
            spawn_forever(async move {
                let result = patch.finish().await;
                self.requests.write().remove(&key);
                self.dispatch(match result {
                    Ok(offset) => Event::Patched { key, offset },
                    Err(failure) => Event::Failed { key, failure },
                });
            });
        }
    }

    /// An aborted PATCH can hold the upload's lock for a moment, so a busy upload is retried.
    async fn delete(url: String) {
        for attempt in 1..=DELETE_ATTEMPTS {
            match transport::delete(&url).await {
                Err(ApiFailure::Server(body)) if body.code == UPLOAD_BUSY => {
                    let delay = u32::try_from(retry_delay_ms(attempt)).unwrap_or(u32::MAX);
                    TimeoutFuture::new(delay).await;
                }
                _ => return,
            }
        }
    }

    /// The picked folder's `webkitRelativePath`, or the bare name.
    fn relative_path(file: &File) -> String {
        js_sys::Reflect::get(file, &"webkitRelativePath".into())
            .ok()
            .and_then(|value| value.as_string())
            .filter(|relative| !relative.is_empty())
            .unwrap_or_else(|| file.name())
    }

    fn picked_file(file: &File, relative: String) -> PickedFile {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "sizes and timestamps are whole numbers far below 2^53"
        )]
        let (size, modified_ms) = (file.size() as u64, file.last_modified() as i64);
        PickedFile {
            relative,
            size,
            modified_ms: Some(modified_ms),
        }
    }

    /// Asks the browser to confirm leaving the page while uploads are running.
    pub(super) fn use_leave_guard(queue: Signal<Queue>) {
        let active = use_memo(move || queue.read().is_active());
        let listener = use_hook(|| {
            Rc::new(Closure::<dyn Fn(BeforeUnloadEvent)>::new(
                |event: BeforeUnloadEvent| {
                    event.prevent_default();
                    event.set_return_value(" ");
                },
            ))
        });
        use_effect(move || {
            let Some(window) = web_sys::window() else {
                return;
            };
            let callback = listener.as_ref().as_ref().unchecked_ref();
            let _ = if active() {
                window.add_event_listener_with_callback("beforeunload", callback)
            } else {
                window.remove_event_listener_with_callback("beforeunload", callback)
            };
        });
    }

    /// Files dropped outside the drop zone would otherwise open in the tab, ending the app and
    /// its uploads. The drop zone handles its own drags first and marks them handled.
    pub(super) fn use_stray_drop_guard() {
        use_hook(|| {
            let Some(window) = web_sys::window() else {
                return;
            };
            let listener = Closure::<dyn Fn(DragEvent)>::new(|event: DragEvent| {
                if event.default_prevented() {
                    return;
                }
                let Some(data) = event.data_transfer() else {
                    return;
                };
                if !data.types().iter().any(|kind| kind == "Files") {
                    return;
                }
                event.prevent_default();
                data.set_drop_effect("none");
            });
            for kind in ["dragover", "drop"] {
                let _ = window
                    .add_event_listener_with_callback(kind, listener.as_ref().unchecked_ref());
            }
            listener.forget();
        });
    }
}

#[cfg(target_arch = "wasm32")]
use browser::{use_leave_guard, use_stray_drop_guard};
