/// Why a file or folder name is refused before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameError {
    Empty,
    Slash,
    Dot,
    Reserved,
    TooLong,
}

const MAX_NAME_BYTES: usize = 255;
const RESERVED: &str = ".explorer-uploads";

/// The trimmed name when the server can accept it; the reserved name is only refused at the
/// volume root, where it lives.
pub fn validate_name(name: &str, at_root: bool) -> Result<&str, NameError> {
    let name = name.trim();
    if name.is_empty() {
        Err(NameError::Empty)
    } else if name.contains('/') {
        Err(NameError::Slash)
    } else if name == "." || name == ".." {
        Err(NameError::Dot)
    } else if at_root && name == RESERVED {
        Err(NameError::Reserved)
    } else if name.len() > MAX_NAME_BYTES {
        Err(NameError::TooLong)
    } else {
        Ok(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_accepts_plain_names() {
        assert_eq!(validate_name("  a  ", false), Ok("a"));
        assert_eq!(validate_name("Films 2026", true), Ok("Films 2026"));
    }

    #[test]
    fn refuses_blank_names() {
        assert_eq!(validate_name("", false), Err(NameError::Empty));
        assert_eq!(validate_name("   ", false), Err(NameError::Empty));
    }

    #[test]
    fn refuses_separators_and_dots() {
        assert_eq!(validate_name("a/b", false), Err(NameError::Slash));
        assert_eq!(validate_name(".", false), Err(NameError::Dot));
        assert_eq!(validate_name("..", false), Err(NameError::Dot));
        assert_eq!(validate_name("...", false), Ok("..."));
    }

    #[test]
    fn reserved_name_only_at_the_root() {
        assert_eq!(
            validate_name(".explorer-uploads", true),
            Err(NameError::Reserved)
        );
        assert_eq!(
            validate_name(".explorer-uploads", false),
            Ok(".explorer-uploads")
        );
    }

    #[test]
    fn limits_the_name_to_255_bytes() {
        assert_eq!(
            validate_name(&"a".repeat(256), false),
            Err(NameError::TooLong)
        );
        assert_eq!(
            validate_name(&"a".repeat(255), false).map(str::len),
            Ok(255)
        );
        assert_eq!(
            validate_name(&"é".repeat(128), false),
            Err(NameError::TooLong)
        );
    }
}
