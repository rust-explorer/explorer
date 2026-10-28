use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::CString;

use super::scan::keep_biggest;
use super::{USAGE_MAX_HARD_LINKS, UsageNode};

/// One name of a hard-linked file. `path`: its names from the scanned folder down. `name`: `None`
/// when it, or a folder on its path, is not UTF-8.
pub(super) struct HardLink {
    pub path: Vec<CString>,
    pub name: Option<String>,
    pub node: usize,
    pub collapsed: bool,
    pub bytes: u64,
}

impl HardLink {
    /// Counts the file into the folder that keeps it, before the bottom-up sum.
    pub fn credit(self, nodes: &mut [UsageNode]) {
        let node = &mut nodes[self.node];
        node.files += 1;
        node.bytes += self.bytes;
        if self.collapsed {
            return;
        }
        if let Some(name) = self.name {
            node.top_files.push((name, self.bytes));
            keep_biggest(node);
        } else {
            node.other_files += 1;
            node.other_bytes += self.bytes;
        }
    }
}

/// What [`HardLinks::offer`] made of a name.
pub(super) enum Offered {
    /// The first name of the file; it will be credited once, by [`HardLinks::settle`].
    New,
    Seen,
    /// The set is full: the caller credits this name itself.
    Full(HardLink),
}

/// A hard-linked file seen under several names is credited once, to the name whose path sorts
/// first, so that every folder's total stays the same from one scan to the next. At most `max`
/// files are tracked.
pub(super) struct HardLinks {
    links: HashMap<(u64, u64), HardLink>,
    max: usize,
}

impl Default for HardLinks {
    fn default() -> Self {
        Self::with_max(USAGE_MAX_HARD_LINKS)
    }
}

impl HardLinks {
    pub fn with_max(max: usize) -> Self {
        Self {
            links: HashMap::new(),
            max,
        }
    }

    pub fn offer(&mut self, id: (u64, u64), link: HardLink) -> Offered {
        let full = self.links.len() >= self.max;
        match self.links.entry(id) {
            Entry::Vacant(_) if full => Offered::Full(link),
            Entry::Vacant(entry) => {
                entry.insert(link);
                Offered::New
            }
            Entry::Occupied(mut entry) => {
                if link.path < entry.get().path {
                    entry.insert(link);
                }
                Offered::Seen
            }
        }
    }

    pub fn settle(self, nodes: &mut [UsageNode]) {
        for link in self.links.into_values() {
            link.credit(nodes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(name: &str, node: usize) -> HardLink {
        HardLink {
            path: vec![CString::new(name).unwrap()],
            name: Some(name.to_owned()),
            node,
            collapsed: false,
            bytes: 10,
        }
    }

    #[test]
    fn tracks_at_most_max_files() {
        let mut links = HardLinks::with_max(2);
        assert!(matches!(links.offer((1, 1), link("a", 0)), Offered::New));
        assert!(matches!(links.offer((1, 2), link("b", 0)), Offered::New));
        assert!(matches!(links.offer((1, 1), link("c", 0)), Offered::Seen));
        let Offered::Full(untracked) = links.offer((1, 3), link("d", 1)) else {
            panic!("a third file was tracked");
        };
        assert_eq!(links.links.len(), 2);

        let mut nodes = vec![UsageNode::default(), UsageNode::default()];
        untracked.credit(&mut nodes);
        links.settle(&mut nodes);
        assert_eq!((nodes[0].files, nodes[0].bytes), (2, 20));
        assert_eq!(nodes[1].top_files, [("d".to_owned(), 10)]);
    }
}
