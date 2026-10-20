use std::future::Future;

use js_sys::{Array, Function, Promise};
use wasm_bindgen::{JsCast as _, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{DataTransfer, File, FileSystemDirectoryEntry, FileSystemEntry, FileSystemFileEntry};

/// What a drop handed over, taken before the browser empties the list.
enum Dropped {
    Entry(FileSystemEntry),
    File(File),
}

/// Reads the dropped items now, then walks dropped folders in the returned future.
/// Gives the files with their path relative to the drop, and every folder seen.
/// Unreadable entries are skipped.
pub fn collect(
    data: &DataTransfer,
) -> impl Future<Output = (Vec<(File, String)>, Vec<String>)> + use<> {
    let items = data.items();
    let mut dropped = Vec::new();
    for index in 0..items.length() {
        let Some(item) = items.get(index) else {
            continue;
        };
        if item.kind() != "file" {
            continue;
        }
        match item.webkit_get_as_entry() {
            Ok(Some(entry)) => dropped.push(Dropped::Entry(entry)),
            _ => {
                if let Ok(Some(file)) = item.get_as_file() {
                    dropped.push(Dropped::File(file));
                }
            }
        }
    }
    async move {
        let mut files = Vec::new();
        let mut dirs = Vec::new();
        let mut pending = Vec::new();
        for item in dropped {
            match item {
                Dropped::Entry(entry) => pending.push((entry.name(), entry)),
                Dropped::File(file) => files.push((file.clone(), file.name())),
            }
        }
        while let Some((relative, entry)) = pending.pop() {
            if entry.is_file() {
                if let Some(file) = file_of(&entry).await {
                    files.push((file, relative));
                }
            } else if entry.is_directory() {
                let children = children_of(&entry).await;
                dirs.push(relative.clone());
                pending.extend(
                    children
                        .into_iter()
                        .map(|child| (format!("{relative}/{}", child.name()), child)),
                );
            }
        }
        (files, dirs)
    }
}

async fn file_of(entry: &FileSystemEntry) -> Option<File> {
    let entry: &FileSystemFileEntry = entry.dyn_ref()?;
    call(|success, failure| entry.file_with_callback_and_callback(success, failure))
        .await
        .ok()?
        .dyn_into()
        .ok()
}

/// A reader returns at most 100 entries per call; an empty batch ends the listing.
async fn children_of(entry: &FileSystemEntry) -> Vec<FileSystemEntry> {
    let Some(directory) = entry.dyn_ref::<FileSystemDirectoryEntry>() else {
        return Vec::new();
    };
    let reader = directory.create_reader();
    let mut children = Vec::new();
    loop {
        let batch = call(|success, failure| {
            let _ = reader.read_entries_with_callback_and_callback(success, failure);
        })
        .await;
        let Some(batch) = batch.ok().and_then(|batch| batch.dyn_into::<Array>().ok()) else {
            break;
        };
        if batch.length() == 0 {
            break;
        }
        children.extend(batch.iter().filter_map(|child| child.dyn_into().ok()));
    }
    children
}

/// Turns a success/error callback pair into a future: the callbacks are the promise's own.
async fn call(start: impl FnOnce(&Function, &Function)) -> Result<JsValue, JsValue> {
    let mut start = Some(start);
    let promise = Promise::new(&mut |resolve, reject| {
        if let Some(start) = start.take() {
            start(&resolve, &reject);
        }
    });
    JsFuture::from(promise).await
}
