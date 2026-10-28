//! Pieces shared by the activity panel and its rows.

pub const ICON_BUTTON: &str = "inline-flex size-6 items-center justify-center rounded text-slate-400 hover:bg-slate-100 hover:text-slate-600 focus-visible:outline-2 focus-visible:outline-indigo-600 dark:hover:bg-slate-800 dark:hover:text-slate-300";

/// The last component of a volume-relative path.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_is_the_last_component() {
        assert_eq!(file_name("Films/2026/a.mkv"), "a.mkv");
        assert_eq!(file_name("a.mkv"), "a.mkv");
    }
}
