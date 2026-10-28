const MAX_PATH_LEN: usize = 4096;
pub(super) const MAX_COMPONENT_LEN: usize = 255;

/// Where uploads are staged, at each volume root; no `RelPath` can start with it.
pub const STAGING_DIR: &str = ".explorer-uploads";

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
        if components.first() == Some(&STAGING_DIR) {
            return Err(invalid("reserved name"));
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

    /// `None` for the root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_owned()),
            None => Self::root(),
        })
    }

    /// Appends a single entry name, validated like any other path.
    pub fn join(&self, name: &str) -> Result<Self, InvalidPath> {
        if name.is_empty() || name.contains('/') {
            return Err(invalid("not a single component"));
        }
        if self.is_root() {
            Self::parse(name)
        } else {
            Self::parse(&format!("{}/{name}", self.0))
        }
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
    fn rel_path_join() {
        assert_eq!(
            RelPath::root().join("a").unwrap(),
            RelPath::parse("a").unwrap()
        );
        let parent = RelPath::parse("a/b").unwrap();
        assert_eq!(parent.join("c d").unwrap().as_str(), "a/b/c d");
        for name in ["", "..", ".", "x/y", "a\nb"] {
            assert!(parent.join(name).is_err(), "{name:?}");
        }
        let near_limit = RelPath::parse(&"a/".repeat(2047)).unwrap();
        assert!(near_limit.join("bcde").is_err());
    }

    #[test]
    fn rel_path_accepts_limits() {
        let max_component = "x".repeat(255);
        assert!(RelPath::parse(&max_component).is_ok());
        let max_path = "a/".repeat(2047) + "bc";
        assert_eq!(max_path.len(), 4096);
        assert!(RelPath::parse(&max_path).is_ok());
    }

    #[test]
    fn rel_path_rejects_staging_dir() {
        for raw in [
            ".explorer-uploads",
            ".explorer-uploads/x",
            "/.explorer-uploads",
        ] {
            assert!(RelPath::parse(raw).is_err(), "{raw:?}");
        }
        assert_eq!(
            RelPath::parse(".explorer-uploads/x").unwrap_err().reason,
            "reserved name"
        );
        assert!(RelPath::root().join(STAGING_DIR).is_err());
        for raw in [
            "a/.explorer-uploads",
            ".explorer-uploads2",
            "x/.explorer-uploads/y",
        ] {
            assert!(RelPath::parse(raw).is_ok(), "{raw:?}");
        }
    }

    #[test]
    fn rel_path_parent() {
        assert_eq!(
            RelPath::parse("a/b").unwrap().parent(),
            Some(RelPath::parse("a").unwrap())
        );
        assert_eq!(RelPath::parse("a").unwrap().parent(), Some(RelPath::root()));
        assert_eq!(RelPath::root().parent(), None);
    }
}
