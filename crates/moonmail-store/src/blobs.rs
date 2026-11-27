use {
    moonmail_core::BlobStore,
    sha2::{Digest, Sha256},
    std::{
        fs,
        io::Write,
        path::{Path, PathBuf},
    },
};

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("blob storage failed")]
    Io(#[from] std::io::Error),
    #[error("invalid blob hash")]
    InvalidHash,
    #[error("blob failed its SHA-256 integrity check")]
    Integrity,
    #[error("could not read the original from msgvault")]
    External(#[from] rusqlite::Error),
    #[error("msgvault no longer holds this original")]
    ExternalMissing,
}

#[derive(Clone)]
pub struct FileBlobs {
    root: PathBuf,
    extension: &'static str,
    /// The archive root, for message originals kept by msgvault instead.
    external: Option<PathBuf>,
}

impl FileBlobs {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().join("messages"),
            extension: "eml",
            external: Some(root.as_ref().to_owned()),
        }
    }

    /// Draft files are immutable too, but use a separate namespace from MIME.
    pub fn attachments(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().join("attachments"),
            extension: "bin",
            external: None,
        }
    }

    pub fn draft_attachments(&self) -> Self {
        Self {
            root: self.root.with_file_name("attachments"),
            extension: "bin",
            external: None,
        }
    }

    fn path(&self, hash: &str) -> Result<PathBuf, BlobError> {
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(BlobError::InvalidHash);
        }
        Ok(self
            .root
            .join(&hash[..2])
            .join(format!("{hash}.{}", self.extension)))
    }
}

impl BlobStore for FileBlobs {
    type Error = BlobError;

    fn put(&self, bytes: &[u8]) -> Result<String, BlobError> {
        let hash = content_digest(bytes);
        let path = self.path(&hash)?;
        let parent = path.parent().ok_or(BlobError::InvalidHash)?;
        fs::create_dir_all(parent)?;
        if path.exists() {
            self.get(&hash)?;
            return Ok(hash);
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(bytes)?;
        temporary.as_file().sync_all()?;
        match temporary.persist_noclobber(&path) {
            Ok(_) => {},
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.get(&hash)?;
            },
            Err(error) => return Err(error.error.into()),
        }
        #[cfg(unix)]
        {
            fs::File::open(parent)?.sync_all()?;
            fs::File::open(&self.root)?.sync_all()?;
            // Also persist a newly created messages/attachments namespace before
            // metadata can commit a reference to its first blob.
            fs::File::open(self.root.parent().ok_or(BlobError::InvalidHash)?)?.sync_all()?;
        }
        Ok(hash)
    }

    fn get(&self, hash: &str) -> Result<Vec<u8>, BlobError> {
        let bytes = match fs::read(self.path(hash)?) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => match &self.external {
                Some(root) => crate::external_blobs::read(root, hash)?.ok_or(error)?,
                None => return Err(error.into()),
            },
            Err(error) => return Err(error.into()),
        };
        if content_digest(&bytes) != hash {
            return Err(BlobError::Integrity);
        }
        Ok(bytes)
    }
}

impl FileBlobs {
    /// Delete this file once msgvault returns the same bytes for `hash`.
    /// Returns whether a file was released.
    pub fn release(&self, hash: &str) -> Result<bool, BlobError> {
        let path = self.path(hash)?;
        let Some(root) = &self.external else {
            return Ok(false);
        };
        if !path.exists() {
            return Ok(false);
        }
        match crate::external_blobs::read(root, hash)? {
            Some(bytes) if content_digest(&bytes) == hash => {
                fs::remove_file(&path)?;
                Ok(true)
            },
            _ => Ok(false),
        }
    }
}

pub fn content_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
