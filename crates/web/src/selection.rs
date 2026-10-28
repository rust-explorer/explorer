use std::collections::{BTreeSet, HashSet};

/// The checked entries of a folder, by name, and where a shift-click range starts.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Selection {
    names: BTreeSet<String>,
    anchor: Option<String>,
}

impl Selection {
    /// Flips `name` and makes it the range anchor.
    pub fn toggle(&mut self, name: &str) {
        if !self.names.remove(name) {
            self.names.insert(name.to_owned());
        }
        self.anchor = Some(name.to_owned());
    }

    /// Selects the rows between the anchor and `name`, both included, in display order; without
    /// a usable anchor it is a plain toggle.
    pub fn select_range(&mut self, rows: &[&str], name: &str) {
        let position = |wanted: &str| rows.iter().position(|row| *row == wanted);
        let bounds = self
            .anchor
            .as_deref()
            .and_then(position)
            .zip(position(name));
        let Some((from, to)) = bounds else {
            self.toggle(name);
            return;
        };
        let (start, end) = (from.min(to), from.max(to));
        self.names
            .extend(rows[start..=end].iter().map(|row| (*row).to_owned()));
    }

    pub fn set_all(&mut self, rows: &[&str], selected: bool) {
        if selected {
            self.names.extend(rows.iter().map(|row| (*row).to_owned()));
        } else {
            self.names.clear();
            self.anchor = None;
        }
    }

    /// Whether something selected, or the anchor, is no longer listed.
    pub fn has_unlisted(&self, rows: &[&str]) -> bool {
        let listed = listed(rows);
        self.names
            .iter()
            .chain(&self.anchor)
            .any(|name| !listed.contains(name.as_str()))
    }

    /// Drops what is no longer listed.
    pub fn retain(&mut self, rows: &[&str]) {
        let listed = listed(rows);
        self.names.retain(|name| listed.contains(name.as_str()));
        if self.anchor.as_deref().is_some_and(|a| !listed.contains(a)) {
            self.anchor = None;
        }
    }

    pub fn clear(&mut self) {
        self.names.clear();
        self.anchor = None;
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }
}

fn listed<'a>(rows: &[&'a str]) -> HashSet<&'a str> {
    rows.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROWS: [&str; 5] = ["a", "b", "c", "d", "e"];

    fn names(selection: &Selection) -> Vec<&str> {
        selection.names.iter().map(String::as_str).collect()
    }

    #[test]
    fn toggle_flips_one_name() {
        let mut selection = Selection::default();
        selection.toggle("b");
        selection.toggle("a");
        assert_eq!(names(&selection), ["a", "b"]);
        assert!(selection.contains("a"));
        assert_eq!(selection.len(), 2);
        selection.toggle("a");
        assert_eq!(names(&selection), ["b"]);
        selection.toggle("b");
        assert!(selection.is_empty());
    }

    #[test]
    fn range_forward_from_the_anchor() {
        let mut selection = Selection::default();
        selection.toggle("b");
        selection.select_range(&ROWS, "d");
        assert_eq!(names(&selection), ["b", "c", "d"]);
    }

    #[test]
    fn range_backward_from_the_anchor() {
        let mut selection = Selection::default();
        selection.toggle("d");
        selection.select_range(&ROWS, "b");
        assert_eq!(names(&selection), ["b", "c", "d"]);
    }

    #[test]
    fn range_keeps_the_anchor_for_the_next_one() {
        let mut selection = Selection::default();
        selection.toggle("c");
        selection.select_range(&ROWS, "e");
        selection.select_range(&ROWS, "a");
        assert_eq!(names(&selection), ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn range_without_a_usable_anchor_toggles() {
        let mut selection = Selection::default();
        selection.select_range(&ROWS, "c");
        assert_eq!(names(&selection), ["c"]);

        // The anchor vanished from the listing.
        selection.toggle("a");
        selection.retain(&["b", "c"]);
        selection.select_range(&["b", "c"], "b");
        assert_eq!(names(&selection), ["b", "c"]);
    }

    #[test]
    fn set_all_selects_or_clears_the_rows() {
        let mut selection = Selection::default();
        selection.toggle("gone");
        selection.set_all(&ROWS, true);
        assert_eq!(selection.len(), 6);
        selection.set_all(&ROWS, false);
        assert!(selection.is_empty());
    }

    #[test]
    fn retain_drops_vanished_names() {
        let mut selection = Selection::default();
        selection.set_all(&ROWS, true);
        assert!(selection.has_unlisted(&["b", "d", "z"]));
        selection.retain(&["b", "d", "z"]);
        assert_eq!(names(&selection), ["b", "d"]);
        assert!(!selection.has_unlisted(&["b", "d", "z"]));
    }

    #[test]
    fn a_vanished_anchor_is_unlisted() {
        let mut selection = Selection::default();
        selection.toggle("a");
        selection.toggle("a");
        assert!(selection.is_empty());
        assert!(selection.has_unlisted(&["b"]));
        assert!(!selection.has_unlisted(&["a"]));
    }

    #[test]
    fn bookkeeping_scales_with_big_folders() {
        let rows: Vec<String> = (0..40_000).map(|i| format!("IMG_{i:06}.jpg")).collect();
        let rows: Vec<&str> = rows.iter().map(String::as_str).collect();
        let mut selection = Selection::default();
        selection.set_all(&rows, true);
        let started = std::time::Instant::now();
        assert!(!selection.has_unlisted(&rows));
        selection.retain(&rows[1..]);
        assert_eq!(selection.len(), rows.len() - 1);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn clear_empties() {
        let mut selection = Selection::default();
        selection.toggle("a");
        selection.clear();
        assert_eq!(selection, Selection::default());
    }
}
