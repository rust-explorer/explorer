use shared::api::browse::EntryKind;

/// What a row's menu can do with its entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Download,
    Rename,
    MoveTo,
    CopyTo,
    Delete,
}

impl RowAction {
    /// Whether the feature behind the action is built yet; the others stay out of the menu.
    pub fn available(self) -> bool {
        matches!(self, Self::Download | Self::Rename)
    }
}

/// The actions for an entry of `kind`, in menu order; a read-only volume only lets things out.
pub fn row_actions(kind: EntryKind, read_only: bool) -> Vec<RowAction> {
    use RowAction::{CopyTo, Delete, Download, MoveTo, Rename};
    match (kind, read_only) {
        (EntryKind::File | EntryKind::Dir, false) => vec![Download, Rename, MoveTo, CopyTo, Delete],
        (EntryKind::File | EntryKind::Dir, true) => vec![Download, CopyTo],
        (EntryKind::Other, false) => vec![Rename, Delete],
        (EntryKind::Other, true) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RowAction::{CopyTo, Delete, Download, MoveTo, Rename};

    #[test]
    fn actions_by_kind_and_access() {
        let all = [Download, Rename, MoveTo, CopyTo, Delete];
        for (kind, read_only, expected) in [
            (EntryKind::File, false, &all[..]),
            (EntryKind::Dir, false, &all[..]),
            (EntryKind::Other, false, &[Rename, Delete][..]),
            (EntryKind::File, true, &[Download, CopyTo][..]),
            (EntryKind::Dir, true, &[Download, CopyTo][..]),
            (EntryKind::Other, true, &[][..]),
        ] {
            assert_eq!(
                row_actions(kind, read_only),
                expected,
                "{kind:?} {read_only}"
            );
        }
    }

    #[test]
    fn only_built_actions_are_available() {
        assert!(Download.available());
        assert!(Rename.available());
        for action in [MoveTo, CopyTo, Delete] {
            assert!(!action.available(), "{action:?}");
        }
    }
}
