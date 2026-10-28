const MAX_PATH_LEN: usize = 4096;
const MAX_COMPONENT_LEN: usize = 255;

/// A volume-relative path whose components are all plain names: no `.`, `..`, NUL or control characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelPath(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid path: {reason}")]
pub struct InvalidPath {
    pub reason: &'static str,
}

fn invalid(reason: &'static str) -> InvalidPath {
    InvalidPath { reason }
}

impl RelPath {
    pub fn parse(raw: &str) -> Result<Self, InvalidPath> {
        if raw.starts_with('/') {
            return Err(invalid("absolute path"));
        }
        if raw.len() > MAX_PATH_LEN {
            return Err(invalid("path too long"));
        }
        if raw.contains('\0') {
            return Err(invalid("NUL character"));
        }
        if raw.chars().any(char::is_control) {
            return Err(invalid("control character"));
        }
        let mut components = Vec::new();
        for component in raw.split('/').filter(|c| !c.is_empty()) {
            if component == "." || component == ".." {
                return Err(invalid("dot component"));
            }
            if component.len() > MAX_COMPONENT_LEN {
                return Err(invalid("component too long"));
            }
            components.push(component);
        }
        Ok(Self(components.join("/")))
    }

    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|name| !name.is_empty())
    }

    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_path_accepts_and_normalises() {
        let root = RelPath::parse("").unwrap();
        assert!(root.is_root());
        assert_eq!(root, RelPath::root());
        assert_eq!(root.as_str(), "");
        assert_eq!(root.file_name(), None);

        for (raw, normalised, name) in [
            ("a/b", "a/b", "b"),
            ("a//b/", "a/b", "b"),
            (
                "Films/Dune Part Two.mkv",
                "Films/Dune Part Two.mkv",
                "Dune Part Two.mkv",
            ),
            ("é/😀 #?%.txt", "é/😀 #?%.txt", "😀 #?%.txt"),
            ("a\\b", "a\\b", "a\\b"),
        ] {
            let path = RelPath::parse(raw).unwrap();
            assert_eq!(path.as_str(), normalised, "{raw:?}");
            assert_eq!(path.file_name(), Some(name), "{raw:?}");
            assert!(!path.is_root(), "{raw:?}");
        }
    }

    #[test]
    fn rel_path_rejects() {
        let too_long = "a/".repeat(2048) + "b";
        assert_eq!(too_long.len(), 4097);
        let long_component = "x".repeat(256);

        for raw in [
            "/etc",
            "..",
            "a/../b",
            "./a",
            "a\0b",
            "a\nb",
            too_long.as_str(),
            long_component.as_str(),
        ] {
            assert!(RelPath::parse(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn rel_path_accepts_limits() {
        let max_component = "x".repeat(255);
        assert!(RelPath::parse(&max_component).is_ok());
        let max_path = "a/".repeat(2047) + "bc";
        assert_eq!(max_path.len(), 4096);
        assert!(RelPath::parse(&max_path).is_ok());
    }
}
