mod folder;
mod hard_links;
mod scan;

use std::time::SystemTime;

use shared::api::usage::{UsageItem, UsageKind};

use super::{RelPath, StorageError, TransferProgress, Volume};

pub const USAGE_THREADS: usize = 4;
pub const USAGE_MAX_DIRS: usize = 200_000;
pub const USAGE_TOP_FILES: usize = 20;
/// File names a tree keeps in all, as the biggest files of its folders; once reached, further
/// folders count their files as `other_files` only.
pub const USAGE_MAX_NAMED_FILES: usize = 200_000;
pub const USAGE_VIEW_LIMIT: usize = 200;
/// Hard-linked files a scan tracks to count them once; past this, each name counts.
pub const USAGE_MAX_HARD_LINKS: usize = 100_000;

/// The result of a scan: one node per folder, the root first and every folder after its parent.
#[derive(Debug)]
pub struct UsageTree {
    root: RelPath,
    scanned_at: SystemTime,
    nodes: Vec<UsageNode>,
}

/// Before the bottom-up sum, the counts are the folder's own: its blocks, its files and whatever
/// was folded into it; after it, they include the sub-folders.
#[derive(Debug, Default)]
struct UsageNode {
    name: String,
    parent: usize,
    children: Vec<usize>,
    bytes: u64,
    files: u64,
    dirs: u64,
    incomplete: bool,
    collapsed: bool,
    top_files: Vec<(String, u64)>,
    other_files: u64,
    other_bytes: u64,
}

/// One folder of a [`UsageTree`], ready to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageNodeView {
    pub path: RelPath,
    pub bytes: u64,
    pub files: u64,
    pub dirs: u64,
    pub incomplete: bool,
    pub collapsed: bool,
    pub items: Vec<UsageItem>,
}

impl UsageTree {
    #[must_use]
    pub fn root(&self) -> &RelPath {
        &self.root
    }

    #[must_use]
    pub fn scanned_at(&self) -> SystemTime {
        self.scanned_at
    }

    /// Bytes, files and sub-folders under the root.
    #[must_use]
    pub fn totals(&self) -> (u64, u64, u64) {
        let root = &self.nodes[0];
        (root.bytes, root.files, root.dirs)
    }

    /// `path` is volume-relative and must be the root or under it; None if it is not a scanned
    /// folder.
    #[must_use]
    pub fn view(&self, path: &RelPath) -> Option<UsageNodeView> {
        let rest = if path == &self.root {
            ""
        } else if self.root.is_root() {
            path.as_str()
        } else {
            path.as_str()
                .strip_prefix(self.root.as_str())?
                .strip_prefix('/')?
        };
        let mut index = 0;
        for name in rest.split('/').filter(|name| !name.is_empty()) {
            index = *self.nodes[index]
                .children
                .iter()
                .find(|&&child| self.nodes[child].name == name)?;
        }
        let node = &self.nodes[index];
        Some(UsageNodeView {
            path: path.clone(),
            bytes: node.bytes,
            files: node.files,
            dirs: node.dirs,
            incomplete: node.incomplete,
            collapsed: node.collapsed,
            items: self.items(node),
        })
    }

    /// Sub-folders and kept files biggest first, then the folded files as one item, all cut to
    /// [`USAGE_VIEW_LIMIT`].
    fn items(&self, node: &UsageNode) -> Vec<UsageItem> {
        let dirs = node.children.iter().map(|&child| {
            let child = &self.nodes[child];
            UsageItem {
                name: child.name.clone(),
                kind: UsageKind::Dir,
                bytes: child.bytes,
                files: child.files,
                incomplete: child.incomplete,
            }
        });
        let files = node.top_files.iter().map(|(name, bytes)| UsageItem {
            name: name.clone(),
            kind: UsageKind::File,
            bytes: *bytes,
            files: 1,
            incomplete: false,
        });
        let mut items: Vec<UsageItem> = dirs.chain(files).collect();
        items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        let mut other = UsageItem {
            name: String::new(),
            kind: UsageKind::OtherFiles,
            bytes: node.other_bytes,
            files: node.other_files,
            incomplete: false,
        };
        let mut has_other = node.other_files > 0;
        let room = USAGE_VIEW_LIMIT - usize::from(has_other);
        if items.len() > room {
            for item in items.drain(USAGE_VIEW_LIMIT - 1..) {
                other.bytes += item.bytes;
                other.files += item.files;
                other.incomplete |= item.incomplete;
            }
            has_other = true;
        }
        if has_other {
            let at = items.partition_point(|item| item.bytes >= other.bytes);
            items.insert(at, other);
        }
        items
    }
}

/// Measures the space `path` takes, `du -x` style: allocated blocks, hard links once, symlinks
/// as links, other filesystems and the staging folder left out. A folder that cannot be read is
/// marked incomplete and skipped.
pub fn usage(
    volume: &Volume,
    path: &RelPath,
    progress: &(dyn TransferProgress + Sync),
) -> Result<UsageTree, StorageError> {
    usage_with_limits(volume, path, progress, UsageLimits::default())
}

/// What a tree may hold, which bounds its memory.
#[derive(Debug, Clone, Copy)]
pub struct UsageLimits {
    /// Folders beyond this are counted in their parent, which becomes collapsed.
    pub max_dirs: usize,
    pub max_named_files: usize,
}

impl Default for UsageLimits {
    fn default() -> Self {
        Self {
            max_dirs: USAGE_MAX_DIRS,
            max_named_files: USAGE_MAX_NAMED_FILES,
        }
    }
}

/// Like [`usage`], within `limits`.
pub fn usage_with_limits(
    volume: &Volume,
    path: &RelPath,
    progress: &(dyn TransferProgress + Sync),
    limits: UsageLimits,
) -> Result<UsageTree, StorageError> {
    let limits = UsageLimits {
        max_dirs: limits.max_dirs.max(1),
        ..limits
    };
    let mut nodes = scan::scan(volume, path, progress, limits)?;
    for index in (1..nodes.len()).rev() {
        let child = &nodes[index];
        let (parent, bytes, files, dirs, incomplete) = (
            child.parent,
            child.bytes,
            child.files,
            child.dirs,
            child.incomplete,
        );
        let parent = &mut nodes[parent];
        parent.bytes += bytes;
        parent.files += files;
        parent.dirs += dirs + 1;
        parent.incomplete |= incomplete;
    }
    Ok(UsageTree {
        root: path.clone(),
        scanned_at: SystemTime::now(),
        nodes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(children: usize, other_files: u64) -> UsageTree {
        let mut nodes = vec![UsageNode {
            other_files,
            other_bytes: other_files * 10,
            ..UsageNode::default()
        }];
        for i in 0..children {
            let index = nodes.len();
            nodes[0].children.push(index);
            nodes.push(UsageNode {
                name: format!("d{i:03}"),
                bytes: 1000 + i as u64,
                files: 1,
                ..UsageNode::default()
            });
        }
        UsageTree {
            root: RelPath::root(),
            scanned_at: SystemTime::now(),
            nodes,
        }
    }

    #[test]
    fn view_folds_items_beyond_the_limit() {
        let extra = 5;
        let view = tree(USAGE_VIEW_LIMIT + extra, 3)
            .view(&RelPath::root())
            .unwrap();
        assert_eq!(view.items.len(), USAGE_VIEW_LIMIT);
        assert!(view.items.windows(2).all(|w| w[0].bytes >= w[1].bytes));
        let other = view
            .items
            .iter()
            .find(|item| item.kind == UsageKind::OtherFiles)
            .unwrap();
        // The folded folders are the smallest: d000 to d005.
        let folded: u64 = (0..=extra as u64).map(|i| 1000 + i).sum();
        assert_eq!(other.files, 3 + extra as u64 + 1);
        assert_eq!(other.bytes, 30 + folded);
        // Together they outweigh any single folder.
        assert_eq!(view.items[0].kind, UsageKind::OtherFiles);
        assert_eq!(
            view.items[1].name,
            format!("d{:03}", USAGE_VIEW_LIMIT + extra - 1)
        );
    }

    #[test]
    fn view_fits_without_folding() {
        let view = tree(USAGE_VIEW_LIMIT - 1, 2)
            .view(&RelPath::root())
            .unwrap();
        assert_eq!(view.items.len(), USAGE_VIEW_LIMIT);
        assert_eq!(view.items.last().unwrap().kind, UsageKind::OtherFiles);
        assert_eq!(view.items.last().unwrap().files, 2);

        let view = tree(3, 0).view(&RelPath::root()).unwrap();
        assert_eq!(view.items.len(), 3);
        assert!(view.items.iter().all(|item| item.kind == UsageKind::Dir));
    }
}
