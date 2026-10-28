use std::collections::BTreeSet;

/// A file chosen in the browser, `relative` to the drop or the picked folder.
#[derive(Debug, Clone, PartialEq)]
pub struct PickedFile {
    pub relative: String,
    pub size: u64,
    pub modified_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BatchPlan {
    pub volume: String,
    pub files: Vec<FileSpec>,
    /// Empty folders to create; ancestors of other folders are left out.
    pub folders: Vec<String>,
    /// Items left out because their name can't be used.
    pub skipped: usize,
    /// Items left out because the browser couldn't read them.
    pub unreadable: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileSpec {
    pub volume: String,
    pub path: String,
    pub size: u64,
    pub modified_ms: Option<i64>,
}

/// Destinations for files and empty folders dropped into `base` of `volume`.
pub fn plan_batch(
    volume: &str,
    base: &str,
    files: Vec<PickedFile>,
    empty_dirs: Vec<String>,
) -> BatchPlan {
    let mut skipped = 0;
    let mut specs = Vec::with_capacity(files.len());
    for file in files {
        match normalize(&file.relative) {
            Some(relative) => specs.push(FileSpec {
                volume: volume.to_owned(),
                path: join(base, &relative),
                size: file.size,
                modified_ms: file.modified_ms,
            }),
            None => skipped += 1,
        }
    }
    let mut folders = BTreeSet::new();
    for dir in empty_dirs {
        let Some(relative) = normalize(&dir) else {
            skipped += 1;
            continue;
        };
        let path = join(base, &relative);
        let prefix = format!("{path}/");
        if !specs.iter().any(|spec| spec.path.starts_with(&prefix)) {
            folders.insert(path);
        }
    }
    let leaves: Vec<String> = folders
        .iter()
        .filter(|path| {
            let prefix = format!("{path}/");
            !folders.iter().any(|other| other.starts_with(&prefix))
        })
        .cloned()
        .collect();
    BatchPlan {
        volume: volume.to_owned(),
        files: specs,
        folders: leaves,
        skipped,
        unreadable: 0,
    }
}

/// Drops repeated slashes; `None` for `.`, `..` or a missing name.
fn normalize(relative: &str) -> Option<String> {
    if relative.ends_with('/') {
        return None;
    }
    let components: Vec<&str> = relative.split('/').filter(|c| !c.is_empty()).collect();
    if components.is_empty() || components.iter().any(|c| matches!(*c, "." | "..")) {
        return None;
    }
    Some(components.join("/"))
}

fn join(base: &str, relative: &str) -> String {
    if base.is_empty() {
        relative.to_owned()
    } else {
        format!("{base}/{relative}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picked(relative: &str) -> PickedFile {
        PickedFile {
            relative: relative.to_owned(),
            size: 1,
            modified_ms: Some(5),
        }
    }

    fn paths(plan: &BatchPlan) -> Vec<&str> {
        plan.files.iter().map(|file| file.path.as_str()).collect()
    }

    #[test]
    fn plan_batch_joins_base_and_keeps_only_empty_folders() {
        let plan = plan_batch(
            "media",
            "Films",
            vec![picked("Album/a.jpg"), picked("Album/sub/b.jpg")],
            vec!["Album".into(), "Album/empty".into(), "Other".into()],
        );
        assert_eq!(paths(&plan), ["Films/Album/a.jpg", "Films/Album/sub/b.jpg"]);
        assert_eq!(plan.folders, ["Films/Album/empty", "Films/Other"]);
        assert_eq!(plan.skipped, 0);
        assert_eq!(plan.volume, "media");
        assert_eq!(
            plan.files[0],
            FileSpec {
                volume: "media".into(),
                path: "Films/Album/a.jpg".into(),
                size: 1,
                modified_ms: Some(5),
            }
        );

        let plan = plan_batch(
            "media",
            "",
            vec![picked("a.jpg"), picked("Album/b.jpg")],
            vec!["Empty".into(), "Empty".into(), "Album".into()],
        );
        assert_eq!(paths(&plan), ["a.jpg", "Album/b.jpg"]);
        assert_eq!(plan.folders, ["Empty"]);
    }

    #[test]
    fn plan_batch_creates_a_dropped_tree_of_empty_folders() {
        let plan = plan_batch(
            "media",
            "Films",
            Vec::new(),
            vec![
                "Empty".into(),
                "Empty/a".into(),
                "Empty/b".into(),
                "Empty/b/c".into(),
            ],
        );
        assert!(plan.files.is_empty());
        assert_eq!(plan.folders, ["Films/Empty/a", "Films/Empty/b/c"]);
    }

    #[test]
    fn plan_batch_skips_dot_components() {
        let plan = plan_batch(
            "media",
            "",
            vec![picked("../x"), picked("a/./b"), picked("a//b.txt")],
            Vec::new(),
        );
        assert_eq!(plan.skipped, 2);
        assert_eq!(paths(&plan), ["a/b.txt"]);
    }
}
