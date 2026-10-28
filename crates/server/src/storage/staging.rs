use std::fmt::Write;
use std::io::{self, Seek, SeekFrom};
use std::time::SystemTime;

use cap_std::fs::{Dir, OpenOptions, OpenOptionsExt};
use rustix::fs::OFlags;

use super::{RelPath, StorageError};

const ID_LEN: usize = 32;

/// Names a staged upload: 32 lowercase hex characters, so it is always a plain file name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UploadId(String);

impl UploadId {
    #[must_use]
    pub fn random() -> Self {
        let bytes: [u8; ID_LEN / 2] = rand::random();
        let mut id = String::with_capacity(ID_LEN);
        for byte in bytes {
            write!(id, "{byte:02x}").expect("writing to a String cannot fail");
        }
        Self(id)
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        (s.len() == ID_LEN && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .then(|| Self(s.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The staging directory of a volume. Errors are reported against the volume root, since the
/// directory itself has no `RelPath`.
pub struct Staging {
    pub(super) dir: Dir,
}

fn map_err(err: io::Error) -> StorageError {
    StorageError::from_io(&RelPath::root(), err)
}

impl Staging {
    pub fn create(&self, id: &UploadId) -> Result<std::fs::File, StorageError> {
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(OFlags::NOFOLLOW.bits().cast_signed());
        let file = self.dir.open_with(id.as_str(), &options).map_err(map_err)?;
        Ok(file.into_std())
    }

    pub fn open_append(&self, id: &UploadId) -> Result<(std::fs::File, u64), StorageError> {
        let mut file = self.open_write(id)?;
        let len = file.seek(SeekFrom::End(0)).map_err(map_err)?;
        Ok((file, len))
    }

    pub fn len(&self, id: &UploadId) -> Result<Option<u64>, StorageError> {
        match self.dir.symlink_metadata(id.as_str()) {
            Ok(meta) => Ok(Some(meta.len())),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(map_err(err)),
        }
    }

    pub fn remove(&self, id: &UploadId) -> Result<(), StorageError> {
        match self.dir.remove_file(id.as_str()) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(map_err(err)),
            _ => Ok(()),
        }
    }

    /// Every entry with its modification time; entries that vanish meanwhile or have a non UTF-8
    /// name are skipped.
    pub fn files(&self) -> Result<Vec<(String, SystemTime)>, StorageError> {
        let mut files = Vec::new();
        for entry in self.dir.entries().map_err(map_err)? {
            let entry = entry.map_err(map_err)?;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let modified = match entry.metadata().and_then(|meta| meta.modified()) {
                Ok(modified) => modified.into_std(),
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => return Err(map_err(err)),
            };
            files.push((name, modified));
        }
        Ok(files)
    }

    pub(super) fn open_write(&self, id: &UploadId) -> Result<std::fs::File, StorageError> {
        let mut options = OpenOptions::new();
        options
            .write(true)
            .custom_flags(OFlags::NOFOLLOW.bits().cast_signed());
        let file = self.dir.open_with(id.as_str(), &options).map_err(map_err)?;
        Ok(file.into_std())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_id_parse() {
        let id = UploadId::random();
        assert_eq!(id.as_str().len(), 32);
        assert_eq!(UploadId::parse(id.as_str()), Some(id.clone()));
        assert_ne!(UploadId::random(), id);
        assert!(UploadId::parse("0123456789abcdef0123456789abcdef").is_some());

        for raw in [
            "../x",
            &"a".repeat(31),
            &"a".repeat(33),
            "0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcdeg",
            "",
        ] {
            assert_eq!(UploadId::parse(raw), None, "{raw:?}");
        }
    }
}
