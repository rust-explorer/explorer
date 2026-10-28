use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::{Entry, EntryKind, Rename, Renamed};
use shared::api::conflict::ConflictPolicy;

use super::form::{BUTTON, INPUT, PRIMARY, Problem};
use crate::api::{self, ApiFailure};
use crate::components::common::{ConflictPrompt, FocusTarget, Modal};
use crate::conflict::use_queued_conflict;
use crate::names::{NameError, validate_name};
use crate::revision::Revision;

const TITLE_ID: &str = "rename-title";
const INPUT_ID: &str = "rename-name";
const ERROR_ID: &str = "rename-error";

/// Renames `entry`, which lives in `parent` and is listed in `entries`; a taken name asks what
/// to do and sends again with the chosen policy. `on_renamed` gets the final name.
#[component]
pub fn RenameDialog(
    volume: String,
    parent: String,
    entry: Entry,
    entries: ReadSignal<Rc<Vec<Entry>>>,
    on_close: EventHandler<()>,
    on_renamed: EventHandler<String>,
    #[props(default)] return_focus: Option<FocusTarget>,
) -> Element {
    let Revision(mut revision) = use_context();
    let mut name = use_signal(|| entry.name.clone());
    let mut problem = use_signal(|| None::<Problem>);
    let mut busy = use_signal(|| false);
    let mut conflict = use_signal(|| None::<String>);
    let queued_conflict = use_queued_conflict();
    let at_root = parent.is_empty();
    let folder = if at_root {
        volume.clone()
    } else {
        format!("{volume}/{parent}")
    };
    let source = if at_root {
        entry.name.clone()
    } else {
        format!("{parent}/{}", entry.name)
    };
    let url = api::rename_url(&volume);

    let send = use_callback(move |(new_name, policy): (String, ConflictPolicy)| {
        let body = Rename {
            path: source.clone(),
            name: new_name.clone(),
            conflict: policy,
        };
        let url = url.clone();
        busy.set(true);
        problem.set(None);
        spawn(async move {
            match api::post_json_body::<_, Renamed>(&url, &body).await {
                Ok(renamed) => {
                    *revision.write() += 1;
                    on_close.call(());
                    on_renamed.call(final_name(&renamed.path).to_owned());
                }
                Err(ApiFailure::Server(body)) if body.code == "already_exists" => {
                    busy.set(false);
                    conflict.set(Some(new_name));
                }
                Err(failure) => {
                    busy.set(false);
                    problem.set(Some(Problem::Server(failure)));
                }
            }
        });
    });

    let current = entry.name.clone();
    let submit = move |event: FormEvent| {
        event.prevent_default();
        if busy() {
            return;
        }
        match new_name(&name(), &current, at_root) {
            Ok(None) => on_close.call(()),
            Ok(Some(new_name)) => send((new_name.to_owned(), ConflictPolicy::Fail)),
            Err(error) => problem.set(Some(Problem::Name(error))),
        }
    };

    let kind = entry.kind;
    let select_end = stem_end(&entry.name, kind);
    let message = problem.read().as_ref().map(Problem::message);
    // One conflict prompt at a time: this one comes back once the queued one is answered.
    let prompt = conflict().filter(|_| !queued_conflict()).map(|taken| {
        let target = entries
            .read()
            .iter()
            .find(|listed| listed.name == taken)
            .map(|listed| listed.kind);
        (taken, allow_replace(kind, target))
    });

    // A request can hang (a stalled network or mount): closing stays possible, and the listing
    // is refreshed since the request's outcome is then unknown.
    let mut close = move |()| {
        if busy() {
            *revision.write() += 1;
        }
        on_close.call(());
    };

    rsx! {
        Modal { labelled_by: TITLE_ID, on_close: close, return_focus,
            form { onsubmit: submit,
                h2 {
                    id: TITLE_ID,
                    class: "break-words text-sm font-medium text-gray-900 dark:text-white",
                    {t!("rename-title", name: entry.name.clone())}
                }
                label {
                    r#for: INPUT_ID,
                    class: "mt-4 block text-sm text-slate-700 dark:text-slate-300",
                    {t!("rename-name")}
                }
                input {
                    id: INPUT_ID,
                    r#type: "text",
                    class: INPUT,
                    autofocus: true,
                    autocomplete: "off",
                    spellcheck: false,
                    value: name(),
                    aria_invalid: message.is_some(),
                    aria_describedby: if message.is_some() { ERROR_ID },
                    onmounted: move |event| {
                        let input = event.data();
                        spawn(async move {
                            let _ = input.set_focus(true).await;
                            select_to(&input, select_end);
                        });
                    },
                    oninput: move |event| {
                        name.set(event.value());
                        problem.set(None);
                    },
                }
                if let Some(message) = message.clone() {
                    p {
                        id: ERROR_ID,
                        role: "alert",
                        class: "mt-2 text-sm text-red-600 dark:text-red-400",
                        {message}
                    }
                }
                div { class: "mt-5 flex justify-end gap-3",
                    button {
                        r#type: "button",
                        class: BUTTON,
                        onclick: move |_| close(()),
                        {t!("cancel")}
                    }
                    button { r#type: "submit", class: PRIMARY, disabled: busy(), {t!("rename-save")} }
                }
            }
        }
        if let Some((taken, allow_replace)) = prompt {
            ConflictPrompt {
                name: taken.clone(),
                folder,
                several: false,
                allow_replace,
                on_decide: move |(decision, _): (crate::conflict::Decision, bool)| {
                    conflict.set(None);
                    if let Some(policy) = decision.policy() {
                        send((taken.clone(), policy));
                    }
                },
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn select_to(input: &MountedData, end: u32) {
    use wasm_bindgen::JsCast as _;

    if let Some(input) = input
        .downcast::<web_sys::Element>()
        .and_then(|e| e.dyn_ref::<web_sys::HtmlInputElement>())
    {
        let _ = input.set_selection_range(0, end);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn select_to(_: &MountedData, _: u32) {}

/// Where the pre-selected part of the name ends, in UTF-16 units: before the extension of a file,
/// the whole name for a folder or a name without one (a leading or trailing dot does not count).
fn stem_end(name: &str, kind: EntryKind) -> u32 {
    let stem = match name.rfind('.') {
        Some(dot) if kind != EntryKind::Dir && dot > 0 && dot + 1 < name.len() => &name[..dot],
        _ => name,
    };
    u32::try_from(stem.encode_utf16().count()).unwrap_or(u32::MAX)
}

/// The name to rename `current` to, or `None` when the input leaves it as is; a name with
/// outer spaces submitted untouched is unchanged, not its trimmed form.
fn new_name<'a>(
    input: &'a str,
    current: &str,
    at_root: bool,
) -> Result<Option<&'a str>, NameError> {
    if input == current {
        return Ok(None);
    }
    validate_name(input, at_root).map(|name| (name != current).then_some(name))
}

fn final_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Replace is only offered for a file over a file.
fn allow_replace(source: EntryKind, target: Option<EntryKind>) -> bool {
    source == EntryKind::File && target == Some(EntryKind::File)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_selection() {
        for (name, kind, end) in [
            ("Film.mkv", EntryKind::File, 4),
            ("archive.tar.gz", EntryKind::File, 11),
            (".bashrc", EntryKind::File, 7),
            ("noext", EntryKind::File, 5),
            ("a.", EntryKind::File, 2),
            ("Films.2026", EntryKind::Dir, 10),
            ("pipe.fifo", EntryKind::Other, 4),
            ("été 😀.txt", EntryKind::File, 6),
        ] {
            assert_eq!(stem_end(name, kind), end, "{name:?}");
        }
    }

    #[test]
    fn untouched_names_are_not_renamed() {
        assert_eq!(new_name(" report.pdf", " report.pdf", false), Ok(None));
        assert_eq!(new_name("report.pdf ", "report.pdf ", false), Ok(None));
        assert_eq!(new_name("report.pdf ", "report.pdf", false), Ok(None));
        assert_eq!(
            new_name(" notes.txt ", "report.pdf", false),
            Ok(Some("notes.txt"))
        );
        assert_eq!(new_name(" ", "report.pdf", false), Err(NameError::Empty));
    }

    #[test]
    fn final_name_is_the_last_segment() {
        assert_eq!(final_name("Films/2024/clip (1).mkv"), "clip (1).mkv");
        assert_eq!(final_name("notes.txt"), "notes.txt");
    }

    #[test]
    fn replace_only_a_file_by_a_file() {
        assert!(allow_replace(EntryKind::File, Some(EntryKind::File)));
        for (source, target) in [
            (EntryKind::File, Some(EntryKind::Dir)),
            (EntryKind::Dir, Some(EntryKind::File)),
            (EntryKind::Dir, Some(EntryKind::Dir)),
            (EntryKind::Other, Some(EntryKind::File)),
            (EntryKind::File, Some(EntryKind::Other)),
            (EntryKind::File, None),
        ] {
            assert!(!allow_replace(source, target), "{source:?} {target:?}");
        }
    }
}
