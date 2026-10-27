use std::sync::Arc;

use shared::api::tasks::{TaskInfo, TaskSpec};
use tracing::Instrument as _;

use super::registry::{Outcome, Progress, Tasks};
use crate::http::error::ApiError;

/// How a task stops short: a cancellation it noticed, or a failure. A failure after a cancel
/// request is still a failure.
#[derive(Debug)]
pub enum TaskError {
    Cancelled,
    Failed(ApiError),
}

impl From<ApiError> for TaskError {
    fn from(err: ApiError) -> Self {
        Self::Failed(err)
    }
}

impl Tasks {
    /// Queues a task for `user` that runs `op` on a blocking thread once a permit is free, in
    /// the current span. A failure keeps `request_id` so that it can be traced back.
    pub fn submit(
        self: &Arc<Self>,
        user: &str,
        spec: TaskSpec,
        request_id: Option<String>,
        op: impl FnOnce(&Progress) -> Result<(), TaskError> + Send + 'static,
    ) -> TaskInfo {
        let (id, info) = self.insert(user, spec);
        let tasks = Arc::clone(self);
        tokio::spawn(
            async move { tasks.run(&id, request_id, op).await }
                .instrument(tracing::Span::current()),
        );
        info
    }

    async fn run(
        self: Arc<Self>,
        id: &str,
        request_id: Option<String>,
        op: impl FnOnce(&Progress) -> Result<(), TaskError> + Send + 'static,
    ) {
        let Ok(_permit) = Arc::clone(&self.permits).acquire_owned().await else {
            return;
        };
        let Some(progress) = self.start(id) else {
            return;
        };
        let span = tracing::Span::current();
        let result = tokio::task::spawn_blocking({
            let progress = Arc::clone(&progress);
            move || {
                let _entered = span.enter();
                op(&progress)
            }
        })
        .await;
        let outcome = match result {
            Ok(Ok(())) => Outcome::Done,
            Ok(Err(TaskError::Cancelled)) => Outcome::Cancelled,
            Ok(Err(TaskError::Failed(err))) => Outcome::Failed(err.to_body(request_id)),
            Err(err) => Outcome::Failed(ApiError::internal(err).to_body(request_id)),
        };
        self.finish(id, outcome);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use shared::api::tasks::{Location, TaskState};

    use super::*;

    fn spec() -> TaskSpec {
        TaskSpec::Delete {
            source: Location {
                volume: "media".to_owned(),
                path: "a".to_owned(),
            },
        }
    }

    async fn until(tasks: &Tasks, done: impl Fn(&[TaskInfo]) -> bool) -> Vec<TaskInfo> {
        for _ in 0..500 {
            let list = tasks.list("ada");
            if done(&list) {
                return list;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("tasks never reached the expected state");
    }

    #[tokio::test]
    async fn runs_at_most_the_limit_and_cancels_while_queued() {
        let tasks = Arc::new(Tasks::with_limit(1));
        let (release, wait) = mpsc::channel::<()>();
        let first = tasks.submit("ada", spec(), None, move |_| {
            wait.recv().unwrap();
            Ok(())
        });
        let ran = Arc::new(AtomicBool::new(false));
        let second = tasks.submit("ada", spec(), None, {
            let ran = Arc::clone(&ran);
            move |_| {
                ran.store(true, Ordering::Relaxed);
                Ok(())
            }
        });

        until(&tasks, |list| list[0].state == TaskState::Running).await;
        assert_eq!(tasks.list("ada")[1].state, TaskState::Queued);
        assert!(tasks.cancel_or_dismiss("ada", &second.id));

        release.send(()).unwrap();
        let list = until(&tasks, |list| list[0].state == TaskState::Done).await;
        assert_eq!(list[0].id, first.id);
        assert_eq!(list[1].state, TaskState::Cancelled);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!ran.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn failure_and_cancellation_outcomes() {
        let tasks = Arc::new(Tasks::new());
        tasks.submit("ada", spec(), Some("req-1".to_owned()), |_| {
            Err(ApiError::forbidden().into())
        });
        let list = until(&tasks, |list| list[0].state == TaskState::Failed).await;
        let error = list[0].error.clone().unwrap();
        assert_eq!(error.code, "forbidden");
        assert_eq!(error.request_id.as_deref(), Some("req-1"));

        let (started, wait_start) = mpsc::channel::<()>();
        let (release, wait) = mpsc::channel::<()>();
        let stopped = tasks.submit("ada", spec(), None, move |progress| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            if progress.is_cancelled() {
                return Err(TaskError::Cancelled);
            }
            Ok(())
        });
        tokio::task::spawn_blocking(move || wait_start.recv().unwrap())
            .await
            .unwrap();
        assert!(tasks.cancel_or_dismiss("ada", &stopped.id));
        release.send(()).unwrap();
        let list = until(&tasks, |list| list[1].state == TaskState::Cancelled).await;
        assert_eq!(list[1].error, None);
    }

    #[tokio::test]
    async fn failure_after_a_cancel_request_stays_a_failure() {
        let tasks = Arc::new(Tasks::new());
        let (started, wait_start) = mpsc::channel::<()>();
        let (release, wait) = mpsc::channel::<()>();
        let task = tasks.submit("ada", spec(), None, move |_| {
            started.send(()).unwrap();
            wait.recv().unwrap();
            Err(ApiError::forbidden().into())
        });
        tokio::task::spawn_blocking(move || wait_start.recv().unwrap())
            .await
            .unwrap();
        assert!(tasks.cancel_or_dismiss("ada", &task.id));
        release.send(()).unwrap();
        let list = until(&tasks, |list| !list[0].state.is_active()).await;
        assert_eq!(list[0].state, TaskState::Failed);
        assert_eq!(list[0].error.as_ref().unwrap().code, "forbidden");
    }
}
