/// The folder at each volume root where uploads and copies are staged; no client path can
/// start with it.
pub const RESERVED_NAME: &str = ".explorer-uploads";

/// Whether `name` is [`RESERVED_NAME`] as a filesystem that ignores case sees it: ASCII case,
/// and the long s that Unicode case folding turns into `s`.
#[must_use]
pub fn is_reserved_name(name: &str) -> bool {
    name.chars()
        .map(|c| {
            if c == 'ſ' {
                's'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .eq(RESERVED_NAME.chars())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_name_ignores_case() {
        for name in [
            ".explorer-uploads",
            ".EXPLORER-UPLOADS",
            ".Explorer-Uploads",
            ".explorer-uploadſ",
        ] {
            assert!(is_reserved_name(name), "{name}");
        }
        for name in [
            ".explorer-upload",
            "explorer-uploads",
            ".explorer-uploads2",
            "",
        ] {
            assert!(!is_reserved_name(name), "{name}");
        }
    }
}
