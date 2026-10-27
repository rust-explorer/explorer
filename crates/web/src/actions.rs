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
        matches!(self, Self::Download | Self::Rename | Self::Delete)
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

/// The actions that apply to every selected entry; renaming is for one entry at a time.
pub fn selection_actions(kinds: &[EntryKind], read_only: bool) -> Vec<RowAction> {
    let Some((first, rest)) = kinds.split_first() else {
        return Vec::new();
    };
    let mut actions = row_actions(*first, read_only);
    for kind in rest {
        let allowed = row_actions(*kind, read_only);
        actions.retain(|action| allowed.contains(action));
    }
    actions.retain(|action| *action != RowAction::Rename);
    actions
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
        for action in [Download, Rename, Delete] {
            assert!(action.available(), "{action:?}");
        }
        for action in [MoveTo, CopyTo] {
            assert!(!action.available(), "{action:?}");
        }
    }

    #[test]
    fn selection_actions_are_the_common_ones_without_rename() {
        use EntryKind::{Dir, File, Other};
        for (kinds, read_only, expected) in [
            (
                &[File, Dir][..],
                false,
                &[Download, MoveTo, CopyTo, Delete][..],
            ),
            (&[File][..], false, &[Download, MoveTo, CopyTo, Delete][..]),
            (&[File, Dir][..], true, &[Download, CopyTo][..]),
            (&[File, Other][..], false, &[Delete][..]),
            (&[Other][..], false, &[Delete][..]),
            (&[File, Other][..], true, &[][..]),
            (&[][..], false, &[][..]),
        ] {
            assert_eq!(
                selection_actions(kinds, read_only),
                expected,
                "{kinds:?} {read_only}"
            );
        }
    }
}
