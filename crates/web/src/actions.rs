use shared::api::browse::EntryKind;

/// What a row's menu can do with its entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowAction {
    Download,
    Rename,
    MoveTo,
    CopyTo,
    /// Opens the space usage screen of a folder.
    AnalyzeUsage,
    /// Shows a folder's size in the listing.
    CalculateSize,
    Delete,
}

/// The actions for an entry of `kind`, in menu order; a read-only volume only lets things out,
/// and folders can be measured anywhere.
pub fn row_actions(kind: EntryKind, read_only: bool) -> Vec<RowAction> {
    use RowAction::{AnalyzeUsage, CalculateSize, CopyTo, Delete, Download, MoveTo, Rename};
    match (kind, read_only) {
        (EntryKind::File, false) => vec![Download, Rename, MoveTo, CopyTo, Delete],
        (EntryKind::Dir, false) => vec![
            Download,
            Rename,
            MoveTo,
            CopyTo,
            AnalyzeUsage,
            CalculateSize,
            Delete,
        ],
        (EntryKind::File, true) => vec![Download, CopyTo],
        (EntryKind::Dir, true) => vec![Download, CopyTo, AnalyzeUsage, CalculateSize],
        (EntryKind::Other, false) => vec![Rename, Delete],
        (EntryKind::Other, true) => Vec::new(),
    }
}

/// The actions that apply to every selected entry; renaming and measuring are for one entry at
/// a time.
pub fn selection_actions(kinds: &[EntryKind], read_only: bool) -> Vec<RowAction> {
    let Some((first, rest)) = kinds.split_first() else {
        return Vec::new();
    };
    let mut actions = row_actions(*first, read_only);
    for kind in rest {
        let allowed = row_actions(*kind, read_only);
        actions.retain(|action| allowed.contains(action));
    }
    actions.retain(|action| {
        !matches!(
            action,
            RowAction::Rename | RowAction::AnalyzeUsage | RowAction::CalculateSize
        )
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use RowAction::{AnalyzeUsage, CalculateSize, CopyTo, Delete, Download, MoveTo, Rename};

    #[test]
    fn actions_by_kind_and_access() {
        let file = [Download, Rename, MoveTo, CopyTo, Delete];
        let dir = [
            Download,
            Rename,
            MoveTo,
            CopyTo,
            AnalyzeUsage,
            CalculateSize,
            Delete,
        ];
        for (kind, read_only, expected) in [
            (EntryKind::File, false, &file[..]),
            (EntryKind::Dir, false, &dir[..]),
            (EntryKind::Other, false, &[Rename, Delete][..]),
            (EntryKind::File, true, &[Download, CopyTo][..]),
            (
                EntryKind::Dir,
                true,
                &[Download, CopyTo, AnalyzeUsage, CalculateSize][..],
            ),
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
    fn selection_actions_are_the_common_ones_without_rename() {
        use EntryKind::{Dir, File, Other};
        for (kinds, read_only, expected) in [
            (
                &[File, Dir][..],
                false,
                &[Download, MoveTo, CopyTo, Delete][..],
            ),
            (&[File][..], false, &[Download, MoveTo, CopyTo, Delete][..]),
            (&[Dir][..], false, &[Download, MoveTo, CopyTo, Delete][..]),
            (&[Dir, Dir][..], true, &[Download, CopyTo][..]),
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
