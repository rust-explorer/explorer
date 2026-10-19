use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use fluent_syntax::ast::Entry;
use fluent_syntax::parser;

const REFERENCE: &str = "en-US";

fn locales_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("locales")
}

fn locale_files() -> Vec<(String, String)> {
    let mut files: Vec<_> = fs::read_dir(locales_dir())
        .expect("locales directory is readable")
        .map(|entry| entry.expect("locale entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ftl"))
        .map(|path| {
            let locale = path.file_stem().unwrap().to_string_lossy().into_owned();
            let source = fs::read_to_string(&path).expect("locale file is readable");
            (locale, source)
        })
        .collect();
    files.sort();
    files
}

fn message_ids(source: &str) -> BTreeSet<String> {
    let resource = match parser::parse(source) {
        Ok(resource) | Err((resource, _)) => resource,
    };
    resource
        .body
        .iter()
        .filter_map(|entry| match entry {
            Entry::Message(message) => Some(message.id.name.to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn no_locale_file_has_parse_errors() {
    let files = locale_files();
    assert!(!files.is_empty(), "no .ftl files found");
    let failures: Vec<String> = files
        .iter()
        .filter_map(|(locale, source)| {
            parser::parse(source.as_str())
                .err()
                .map(|(_, errors)| format!("{locale}: {errors:?}"))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "parse errors:\n{}",
        failures.join("\n")
    );
}

#[test]
fn all_locales_define_the_same_message_ids() {
    let files = locale_files();
    let reference = files
        .iter()
        .find(|(locale, _)| locale == REFERENCE)
        .map(|(_, source)| message_ids(source))
        .expect("en-US.ftl exists");

    let mut problems = Vec::new();
    for (locale, source) in files.iter().filter(|(locale, _)| locale != REFERENCE) {
        let ids = message_ids(source);
        let missing: Vec<_> = reference.difference(&ids).collect();
        let extra: Vec<_> = ids.difference(&reference).collect();
        if !missing.is_empty() || !extra.is_empty() {
            problems.push(format!("{locale}: missing {missing:?}, extra {extra:?}"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_api_error_code_has_a_message() {
    let files = locale_files();
    let (_, source) = files
        .iter()
        .find(|(locale, _)| locale == REFERENCE)
        .expect("en-US.ftl exists");
    let ids = message_ids(source);
    let missing: Vec<_> = [
        "volume_not_found",
        "path_not_found",
        "invalid_path",
        "not_a_directory",
        "is_a_directory",
        "permission_denied",
        "outside_volume",
        "range_not_satisfiable",
        "internal",
        "forbidden",
    ]
    .iter()
    .map(|code| format!("error-{code}"))
    .filter(|id| !ids.contains(id))
    .collect();
    assert!(missing.is_empty(), "missing {missing:?}");
}
