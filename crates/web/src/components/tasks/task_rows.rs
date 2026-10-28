use std::collections::HashMap;

use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::tasks::{Location, TaskInfo, TaskSpec, TaskState};

use crate::components::common::icons::{CheckIcon, CloseIcon};
use crate::components::common::{FailureNote, ICON_BUTTON, file_name};
use crate::format::{format_count, format_size};
use crate::i18n::attr;
use crate::routes::{Route, go, usage_route};
use crate::tasks::{Task, use_tasks};

const NOTE: &str = "mt-0.5 text-xs text-slate-500 dark:text-slate-400";

/// How many tasks of one batch get a row; the others are counted on one line.
const BATCH_ROWS: usize = 20;

/// The task rows of the activity panel; a big batch (a delete of thousands of entries) shows its
/// first tasks and a count of the others.
#[component]
pub fn TaskRows() -> Element {
    let list = use_tasks().list();
    let list = list.read();
    let shown: Vec<&Task> = list.shown().collect();
    let rows = grouped(shown.iter().copied(), |task| task.batch);

    rsx! {
        for row in rows {
            match row {
                Row::Item(task) => rsx! {
                    TaskRow { key: "task-{task.key.0}", task: Task::clone(task) }
                },
                Row::More { batch, count } => rsx! {
                    li { key: "more-{batch}", class: "py-2 {NOTE}", {t!("task-more", count: count)} }
                },
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum Row<T> {
    Item(T),
    /// The `count` items of `batch` past the first [`BATCH_ROWS`].
    More {
        batch: u64,
        count: usize,
    },
}

/// The items in order, at most [`BATCH_ROWS`] per batch, each batch's count of the others
/// after its last item.
fn grouped<T>(items: impl Iterator<Item = T> + Clone, batch_of: impl Fn(&T) -> u64) -> Vec<Row<T>> {
    let mut sizes: HashMap<u64, usize> = HashMap::new();
    for item in items.clone() {
        *sizes.entry(batch_of(&item)).or_default() += 1;
    }
    let mut seen: HashMap<u64, usize> = HashMap::new();
    let mut rows = Vec::new();
    for item in items {
        let batch = batch_of(&item);
        let seen = seen.entry(batch).or_default();
        *seen += 1;
        let size = sizes[&batch];
        if *seen <= BATCH_ROWS {
            rows.push(Row::Item(item));
        }
        if *seen == size && size > BATCH_ROWS {
            rows.push(Row::More {
                batch,
                count: size - BATCH_ROWS,
            });
        }
    }
    rows
}

#[component]
fn TaskRow(task: Task) -> Element {
    let store = use_tasks();
    let source = task.spec.source();
    let name = entry_name(source).to_owned();
    let full_path = display(source);
    let state = task.state();
    let key = task.key;

    rsx! {
        li { class: "border-t border-slate-100 py-2 first:border-t-0 dark:border-slate-800",
            div { class: "flex items-center gap-2",
                span {
                    class: "min-w-0 flex-1 truncate text-gray-900 dark:text-slate-100",
                    title: "{full_path}",
                    {label(&task.spec, task.info.as_ref())}
                }
                if let (TaskSpec::Usage { source }, Some(info), TaskState::Done) = (&task.spec, &task.info, state) {
                    ViewLink { route: usage_route(&info.id, source), name: name.clone() }
                }
                if state == TaskState::Done && !task.vanished() {
                    span {
                        class: "text-green-600 dark:text-green-400",
                        role: "img",
                        aria_label: t!("task-done"),
                        CheckIcon {}
                    }
                }
                if state.is_active() {
                    button {
                        r#type: "button",
                        class: ICON_BUTTON,
                        aria_label: attr(t!("task-cancel", name: name.clone())),
                        title: attr(t!("task-cancel", name: name)),
                        onclick: move |_| store.cancel(key),
                        CloseIcon { class: "size-3.5" }
                    }
                }
            }
            TaskStatus { task }
        }
    }
}

#[component]
fn TaskStatus(task: Task) -> Element {
    let signed_out = use_tasks().list().read().signed_out();
    if task.conflict().is_some() {
        return rsx! {
            p { class: NOTE, {t!("task-deciding")} }
        };
    }
    if task.is_active() && signed_out {
        return rsx! {
            p { class: NOTE, {t!("task-signed-out")} }
        };
    }
    if let Some(failure) = task.cancel_failure().filter(|_| task.is_active()) {
        return rsx! {
            p { class: "mt-0.5 text-xs text-red-700 dark:text-red-400", {t!("task-cancel-failed")} }
            FailureNote { failure: failure.clone() }
        };
    }
    if task.vanished() {
        return rsx! {
            p { class: NOTE, {t!("task-vanished")} }
        };
    }
    let usage = matches!(task.spec, TaskSpec::Usage { .. });
    match (task.state(), &task.info) {
        // Its label has the counts.
        (TaskState::Running, Some(_)) if usage => rsx! {},
        (TaskState::Running, Some(info)) => match percent(info) {
            Some(percent) => rsx! {
                div {
                    class: "mt-1 h-1 rounded-full bg-slate-200 dark:bg-slate-700",
                    role: "progressbar",
                    aria_label: t!("task-progress"),
                    aria_valuemin: "0",
                    aria_valuemax: "100",
                    aria_valuenow: "{percent}",
                    div {
                        class: "h-1 rounded-full bg-indigo-600 dark:bg-indigo-400",
                        width: "{percent}%",
                    }
                }
            },
            None => rsx! {
                p { class: NOTE, {files_note(info)} }
            },
        },
        (TaskState::Queued | TaskState::Running, _) => rsx! {
            p { class: NOTE, {t!("task-waiting")} }
        },
        (TaskState::Failed, _) => match task.failure() {
            Some(failure) => rsx! {
                FailureNote { failure }
            },
            None => rsx! {},
        },
        (TaskState::Cancelled, _) => rsx! {
            p { class: NOTE, {t!("task-cancelled")} }
        },
        (TaskState::Done, Some(info)) if info.skipped > 0 => rsx! {
            p { class: NOTE, {skipped_note(&task.spec, info.skipped)} }
        },
        (TaskState::Done, _) => rsx! {},
    }
}

/// Opens the result of a finished usage scan. The panel lives outside the router's pages, so it
/// navigates by hand.
#[component]
fn ViewLink(route: Route, name: String) -> Element {
    rsx! {
        a {
            href: route.to_string(),
            class: "shrink-0 rounded px-1 text-xs font-medium text-indigo-600 hover:underline focus-visible:outline-2 focus-visible:outline-indigo-600 dark:text-indigo-400",
            aria_label: attr(t!("task-view-usage", name: name)),
            onclick: move |event: MouseEvent| {
                let modifiers = event.modifiers();
                if modifiers.ctrl() || modifiers.meta() || modifiers.shift() {
                    return;
                }
                event.prevent_default();
                go(route.clone());
            },
            {t!("task-view")}
        }
    }
}

fn label(spec: &TaskSpec, info: Option<&TaskInfo>) -> String {
    let name = entry_name(spec.source()).to_owned();
    match spec {
        TaskSpec::Delete { .. } => t!("task-deleting", name: name),
        TaskSpec::Usage { .. } => match info.filter(|info| info.state != TaskState::Queued) {
            Some(info) => {
                let locale = i18n().language();
                t!(
                    "task-analysing-counts",
                    name: name,
                    count: info.files_done,
                    files: format_count(info.files_done, &locale),
                    size: format_size(info.bytes_done, &locale)
                )
            }
            None => t!("task-analysing", name: name),
        },
        TaskSpec::Copy { dest_dir, .. } => {
            t!("task-copying", name: name, folder: entry_name(dest_dir).to_owned())
        }
        TaskSpec::Move { dest_dir, .. } => {
            t!("task-moving", name: name, folder: entry_name(dest_dir).to_owned())
        }
    }
}

/// A move leaves what it skipped in the source.
fn skipped_note(spec: &TaskSpec, count: u64) -> String {
    match spec {
        TaskSpec::Move { .. } => t!("task-skipped-move", count: count),
        _ => t!("task-skipped", count: count),
    }
}

fn files_note(info: &TaskInfo) -> String {
    match info.files_total {
        Some(total) => t!("task-files", done: info.files_done, total: total),
        None => t!("task-files-done", count: info.files_done),
    }
}

/// The last component of the path, or the volume name at its root.
fn entry_name(location: &Location) -> &str {
    if location.path.is_empty() {
        &location.volume
    } else {
        file_name(&location.path)
    }
}

fn display(location: &Location) -> String {
    if location.path.is_empty() {
        location.volume.clone()
    } else {
        format!("{}/{}", location.volume, location.path)
    }
}

/// Byte progress, when the total is known.
fn percent(info: &TaskInfo) -> Option<u64> {
    let total = info.bytes_total?;
    let percent = (u128::from(info.bytes_done) * 100)
        .checked_div(u128::from(total))
        .map_or(100, |percent| percent.min(100));
    u64::try_from(percent).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(volume: &str, path: &str) -> Location {
        Location {
            volume: volume.to_owned(),
            path: path.to_owned(),
        }
    }

    fn info(bytes_done: u64, bytes_total: Option<u64>) -> TaskInfo {
        TaskInfo {
            id: "a".to_owned(),
            spec: TaskSpec::Delete {
                source: location("media", "a"),
            },
            state: TaskState::Running,
            files_done: 0,
            files_total: None,
            bytes_done,
            bytes_total,
            skipped: 0,
            final_path: None,
            error: None,
        }
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(entry_name(&location("media", "Films/a.mkv")), "a.mkv");
        assert_eq!(entry_name(&location("media", "")), "media");
        assert_eq!(
            display(&location("media", "Films/a.mkv")),
            "media/Films/a.mkv"
        );
        assert_eq!(display(&location("media", "")), "media");
    }

    #[test]
    fn big_batches_are_cut() {
        let items: Vec<(u64, usize)> = (0..25)
            .map(|i| (7, i))
            .chain([(8, 0)])
            .chain((25..30).map(|i| (7, i)))
            .collect();
        let rows = grouped(items.iter(), |item| item.0);
        assert_eq!(rows.len(), BATCH_ROWS + 2);
        assert!(
            rows[..20]
                .iter()
                .all(|row| matches!(row, Row::Item((7, _))))
        );
        assert_eq!(rows[20], Row::Item(&(8, 0)));
        assert_eq!(
            rows[21],
            Row::More {
                batch: 7,
                count: 10
            }
        );

        let small = [(1, 0), (2, 0), (1, 1)];
        assert_eq!(
            grouped(small.iter(), |item| item.0),
            small.iter().map(Row::Item).collect::<Vec<_>>()
        );
    }

    #[test]
    fn percent_needs_a_byte_total() {
        assert_eq!(percent(&info(10, None)), None);
        assert_eq!(percent(&info(25, Some(100))), Some(25));
        assert_eq!(percent(&info(0, Some(0))), Some(100));
        assert_eq!(percent(&info(200, Some(100))), Some(100));
    }
}
