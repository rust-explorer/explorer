use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::CString;

use super::UsageNode;
use super::scan::keep_biggest;

/// One name of a hard-linked file. `path`: its names from the scanned folder down.
pub(super) struct HardLink {
    pub path: Vec<CString>,
    pub name: String,
    pub node: usize,
    pub collapsed: bool,
    pub bytes: u64,
}

/// A hard-linked file seen under several names is credited once, to the name whose path sorts
/// first, so that every folder's total stays the same from one scan to the next.
#[derive(Default)]
pub(super) struct HardLinks(HashMap<(u64, u64), HardLink>);

impl HardLinks {
    /// True the first time the file is seen.
    pub fn offer(&mut self, id: (u64, u64), link: HardLink) -> bool {
        match self.0.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(link);
                true
            }
            Entry::Occupied(mut entry) => {
                if link.path < entry.get().path {
                    entry.insert(link);
                }
                false
            }
        }
    }

    /// Counts each file into the folder that keeps it, before the bottom-up sum.
    pub fn settle(self, nodes: &mut [UsageNode]) {
        for link in self.0.into_values() {
            let node = &mut nodes[link.node];
            node.files += 1;
            node.bytes += link.bytes;
            if !link.collapsed {
                node.top_files.push((link.name, link.bytes));
                keep_biggest(node);
            }
        }
    }
}
