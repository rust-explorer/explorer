use std::cmp::Ordering;
use std::ops::Range;

use shared::api::{Entry, EntryKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Size,
    Modified,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sort {
    pub key: SortKey,
    pub ascending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            ascending: true,
        }
    }
}

impl Sort {
    /// The sort after clicking the `key` column header.
    #[must_use]
    pub fn toggled(self, key: SortKey) -> Self {
        Self {
            key,
            ascending: self.key != key || !self.ascending,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileType {
    Video,
    Image,
    Audio,
    Archive,
    Document,
    Other,
}

pub fn file_type(name: &str) -> FileType {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return FileType::Other;
    };
    if stem.is_empty() {
        return FileType::Other;
    }
    match extension.to_ascii_lowercase().as_str() {
        "mkv" | "mp4" | "m4v" | "avi" | "mov" | "webm" | "wmv" | "mpg" | "mpeg" => FileType::Video,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "heic" | "heif" | "bmp" | "tif"
        | "tiff" | "svg" | "raw" | "dng" => FileType::Image,
        "mp3" | "flac" | "wav" | "ogg" | "oga" | "opus" | "m4a" | "aac" | "wma" => FileType::Audio,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "iso" => {
            FileType::Archive
        }
        "txt" | "md" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "srt" | "vtt" | "ass" | "csv"
        | "xls" | "xlsx" | "ods" | "ppt" | "pptx" | "odp" | "epub" => FileType::Document,
        _ => FileType::Other,
    }
}

/// Case-insensitive, with digit runs compared by value: `file2 < file10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    folded_cmp(a, b).then_with(|| a.cmp(b))
}

fn folded_cmp(mut a: &str, mut b: &str) -> Ordering {
    loop {
        let (Some(char_a), Some(char_b)) = (a.chars().next(), b.chars().next()) else {
            return b.is_empty().cmp(&a.is_empty());
        };
        if char_a.is_ascii_digit() && char_b.is_ascii_digit() {
            let (digits_a, rest_a) = split_digits(a);
            let (digits_b, rest_b) = split_digits(b);
            let value_a = digits_a.trim_start_matches('0');
            let value_b = digits_b.trim_start_matches('0');
            let order = value_a
                .len()
                .cmp(&value_b.len())
                .then_with(|| value_a.cmp(value_b));
            if order.is_ne() {
                return order;
            }
            (a, b) = (rest_a, rest_b);
        } else {
            let order = char_a.to_lowercase().cmp(char_b.to_lowercase());
            if order.is_ne() {
                return order;
            }
            (a, b) = (&a[char_a.len_utf8()..], &b[char_b.len_utf8()..]);
        }
    }
}

fn split_digits(text: &str) -> (&str, &str) {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text.split_at(end)
}

/// Folders first in both directions; entries without the sorted value come last.
pub fn sorted(entries: &[Entry], sort: Sort) -> Vec<Entry> {
    let directed = |order: Ordering| {
        if sort.ascending {
            order
        } else {
            order.reverse()
        }
    };
    let mut sorted = entries.to_vec();
    sorted.sort_by(|a, b| {
        let group = (a.kind != EntryKind::Dir).cmp(&(b.kind != EntryKind::Dir));
        let by_name = || natural_cmp(&a.name, &b.name);
        group.then_with(|| match sort.key {
            SortKey::Name => directed(by_name()),
            SortKey::Size => missing_last(a.size, b.size, directed).then_with(by_name),
            SortKey::Modified => {
                missing_last(a.modified_ms, b.modified_ms, directed).then_with(by_name)
            }
        })
    });
    sorted
}

fn missing_last<T: Ord>(
    a: Option<T>,
    b: Option<T>,
    directed: impl Fn(Ordering) -> Ordering,
) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => directed(a.cmp(&b)),
        (a, b) => b.is_some().cmp(&a.is_some()),
    }
}

/// The rows to render: those intersecting the viewport, plus `overscan` on each side.
pub fn visible_range(
    scroll_top: f64,
    viewport: f64,
    row_height: f64,
    total: usize,
    overscan: usize,
) -> Range<usize> {
    if total == 0 || row_height <= 0.0 {
        return 0..0;
    }
    let scroll_top = scroll_top.max(0.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "float to usize saturates, and negative or NaN values become 0"
    )]
    let (first, last) = (
        (scroll_top / row_height).floor() as usize,
        ((scroll_top + viewport) / row_height).ceil() as usize,
    );
    let first = first.min(total);
    let start = first.saturating_sub(overscan);
    let end = last.max(first).saturating_add(overscan).min(total);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: EntryKind, size: Option<u64>, modified_ms: Option<i64>) -> Entry {
        Entry {
            name: name.to_owned(),
            kind,
            size,
            modified_ms,
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn natural_cmp_orders_numbers() {
        assert_eq!(natural_cmp("file2", "file10"), Ordering::Less);
        assert_eq!(natural_cmp("file10", "file2"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "B"), Ordering::Less);
        assert_eq!(natural_cmp("B", "c"), Ordering::Less);
        assert_eq!(natural_cmp("Ete", "Été"), Ordering::Less);
        assert_eq!(natural_cmp("Été", "Ete"), Ordering::Greater);
        assert_eq!(
            natural_cmp("x99999999999999999999999", "x100000000000000000000000"),
            Ordering::Less
        );
        assert_eq!(natural_cmp("a", "a1"), Ordering::Less);
        assert_eq!(natural_cmp("same", "same"), Ordering::Equal);
        // Names equal but for case or leading zeros still get a fixed order.
        assert_ne!(natural_cmp("a", "A"), Ordering::Equal);
        assert_eq!(natural_cmp("A", "a"), natural_cmp("a", "A").reverse());
        assert_ne!(natural_cmp("01", "1"), Ordering::Equal);
        assert_eq!(natural_cmp("1", "01"), natural_cmp("01", "1").reverse());
    }

    #[test]
    fn sorted_puts_dirs_first_in_both_directions() {
        let entries = [
            entry("b.txt", EntryKind::File, Some(1), None),
            entry("Zeta", EntryKind::Dir, None, None),
            entry("link", EntryKind::Other, None, None),
            entry("alpha", EntryKind::Dir, None, None),
            entry("a.txt", EntryKind::File, Some(1), None),
        ];
        let ascending = Sort::default();
        assert_eq!(
            names(&sorted(&entries, ascending)),
            ["alpha", "Zeta", "a.txt", "b.txt", "link"]
        );
        let descending = Sort {
            ascending: false,
            ..ascending
        };
        assert_eq!(
            names(&sorted(&entries, descending)),
            ["Zeta", "alpha", "link", "b.txt", "a.txt"]
        );
    }

    #[test]
    fn sorted_by_size_and_modified() {
        let entries = [
            entry("big", EntryKind::File, Some(300), Some(1)),
            entry("unknown", EntryKind::File, None, None),
            entry("small", EntryKind::File, Some(10), Some(3)),
            entry("dir", EntryKind::Dir, None, Some(2)),
            entry("mid b", EntryKind::File, Some(20), Some(2)),
            entry("mid a", EntryKind::File, Some(20), Some(2)),
        ];
        let size = |ascending| Sort {
            key: SortKey::Size,
            ascending,
        };
        assert_eq!(
            names(&sorted(&entries, size(true))),
            ["dir", "small", "mid a", "mid b", "big", "unknown"]
        );
        assert_eq!(
            names(&sorted(&entries, size(false))),
            ["dir", "big", "mid a", "mid b", "small", "unknown"]
        );
        let modified = |ascending| Sort {
            key: SortKey::Modified,
            ascending,
        };
        assert_eq!(
            names(&sorted(&entries, modified(true))),
            ["dir", "big", "mid a", "mid b", "small", "unknown"]
        );
        assert_eq!(
            names(&sorted(&entries, modified(false))),
            ["dir", "small", "mid a", "mid b", "big", "unknown"]
        );
    }

    #[test]
    fn sort_toggles_direction_or_switches_column() {
        let name = Sort::default();
        let name_desc = name.toggled(SortKey::Name);
        assert_eq!(
            name_desc,
            Sort {
                key: SortKey::Name,
                ascending: false
            }
        );
        assert_eq!(name_desc.toggled(SortKey::Name), name);
        assert_eq!(
            name_desc.toggled(SortKey::Size),
            Sort {
                key: SortKey::Size,
                ascending: true
            }
        );
    }

    #[test]
    fn visible_range_windows() {
        assert_eq!(visible_range(0.0, 440.0, 44.0, 20_000, 10), 0..20);
        assert_eq!(visible_range(44_000.0, 440.0, 44.0, 20_000, 10), 990..1020);
        // Halfway through a row, both partial rows are visible.
        assert_eq!(visible_range(22.0, 440.0, 44.0, 20_000, 0), 0..11);
        assert_eq!(
            visible_range(879_560.0, 440.0, 44.0, 20_000, 10),
            19_980..20_000
        );
        assert_eq!(visible_range(1e9, 440.0, 44.0, 20_000, 10), 19_990..20_000);
        assert_eq!(visible_range(0.0, 440.0, 44.0, 5, 10), 0..5);
        assert_eq!(visible_range(0.0, 440.0, 44.0, 0, 10), 0..0);
        // Before the first resize event, or after an elastic overscroll.
        assert_eq!(visible_range(0.0, 0.0, 44.0, 20_000, 10), 0..10);
        assert_eq!(visible_range(-50.0, 440.0, 44.0, 20_000, 10), 0..20);
    }

    #[test]
    fn file_type_by_extension() {
        assert_eq!(file_type("Dune.Part.Two.MKV"), FileType::Video);
        assert_eq!(file_type("affiche.jpg"), FileType::Image);
        assert_eq!(file_type("song.flac"), FileType::Audio);
        assert_eq!(file_type("backup.tar.gz"), FileType::Archive);
        assert_eq!(file_type("sous-titres.srt"), FileType::Document);
        assert_eq!(file_type("Makefile"), FileType::Other);
        assert_eq!(file_type(".mkv"), FileType::Other);
        assert_eq!(file_type("data.bin"), FileType::Other);
    }
}
