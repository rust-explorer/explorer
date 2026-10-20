use std::future::Future;

use js_sys::{Array, Function, Promise};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{DataTransfer, File, FileSystemDirectoryEntry, FileSystemEntry, FileSystemFileEntry};

/// What a drop handed over, taken before the browser empties the list.
enum Dropped {
    Entry(FileSystemEntry),
    File(File),
}

#[derive(Default)]
pub struct Collected {
    /// Each file with its path relative to the drop.
    pub files: Vec<(File, String)>,
    /// Every folder seen, relative to the drop.
    pub dirs: Vec<String>,
    /// Entries that could not be read, and folders whose listing failed.
    pub unreadable: usize,
}

/// Reads the dropped items now, then walks dropped folders in the returned future.
pub fn collect(data: &DataTransfer) -> impl Future<Output = Collected> + use<> {
    let items = data.items();
    let mut dropped = Vec::new();
    let mut unreadable = 0;
    for index in 0..items.length() {
        let Some(item) = items.get(index) else {
            continue;
        };
        if item.kind() != "file" {
            continue;
        }
        match item.webkit_get_as_entry() {
            Ok(Some(entry)) => dropped.push(Dropped::Entry(entry)),
            _ => match item.get_as_file() {
                Ok(Some(file)) => dropped.push(Dropped::File(file)),
                _ => unreadable += 1,
            },
        }
    }
    async move {
        let mut collected = Collected {
            unreadable,
            ..Collected::default()
        };
        let mut pending = Vec::new();
        for item in dropped {
            match item {
                Dropped::Entry(entry) => pending.push((entry.name(), entry)),
                Dropped::File(file) => collected.files.push((file.clone(), file.name())),
            }
        }
        while let Some((relative, entry)) = pending.pop() {
            if entry.is_file() {
                match file_of(&entry).await {
                    Some(file) => collected.files.push((file, relative)),
                    None => collected.unreadable += 1,
                }
            } else if entry.is_directory() {
                let (children, complete) = children_of(&entry).await;
                if !complete {
                    collected.unreadable += 1;
                }
                collected.dirs.push(relative.clone());
                pending.extend(
                    children
                        .into_iter()
                        .map(|child| (format!("{relative}/{}", child.name()), child)),
                );
            } else {
                collected.unreadable += 1;
            }
        }
        collected
    }
}

// Browsers hand out entries that are not instances of the `FileSystem*Entry` interfaces, so
// they are cast unchecked once `isFile` or `isDirectory` told what they are.

async fn file_of(entry: &FileSystemEntry) -> Option<File> {
    let entry: &FileSystemFileEntry = entry.unchecked_ref();
    call(|success, failure| {
        entry.file_with_callback_and_callback(success, failure);
        Ok(())
    })
    .await
    .ok()?
    .dyn_into()
    .ok()
}

/// A reader returns at most 100 entries per call; an empty batch ends the listing. Also tells
/// whether the listing ran to its end.
async fn children_of(entry: &FileSystemEntry) -> (Vec<FileSystemEntry>, bool) {
    let directory: &FileSystemDirectoryEntry = entry.unchecked_ref();
    let reader = directory.create_reader();
    let mut children = Vec::new();
    loop {
        let batch = call(|success, failure| {
            reader.read_entries_with_callback_and_callback(success, failure)
        })
        .await;
        let Some(batch) = batch.ok().filter(Array::is_array) else {
            return (children, false);
        };
        let batch: Array = batch.unchecked_into();
        if batch.length() == 0 {
            return (children, true);
        }
        children.extend(batch.iter().map(JsCast::unchecked_into::<FileSystemEntry>));
    }
}

/// Turns a success/error callback pair into a future: the callbacks are the promise's own, and
/// an error thrown while starting rejects it.
async fn call(
    start: impl FnOnce(&Function, &Function) -> Result<(), JsValue>,
) -> Result<JsValue, JsValue> {
    let mut start = Some(start);
    let promise = Promise::new(&mut |resolve, reject| {
        if let Some(start) = start.take()
            && let Err(err) = start(&resolve, &reject)
        {
            let _ = reject.call1(&JsValue::UNDEFINED, &err);
        }
    });
    JsFuture::from(promise).await
}
