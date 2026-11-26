//! Read only published files. Every traversed directory participates in the
//! completeness check, including container directories where folders can move.
use {
    anyhow::{Context, Result, ensure},
    std::{
        collections::{BTreeMap, HashSet},
        fs,
        os::unix::fs::MetadataExt,
        path::{Path, PathBuf},
    },
};

pub(super) fn identity(m: &fs::Metadata) -> String {
    format!("{}:{}", m.dev(), m.ino())
}

pub(super) fn fingerprint(m: &fs::Metadata) -> String {
    format!(
        "{}:{}:{}:{}",
        identity(m),
        m.len(),
        m.mtime(),
        m.mtime_nsec()
    )
}

pub(super) struct Entry {
    pub path: PathBuf,
    pub folder: String,
    pub stem: String,
    pub flags: String,
    pub extension: String,
    pub fingerprint: String,
}

pub(super) struct Inventory {
    pub entries: Vec<Entry>,
    pub folders: BTreeMap<String, String>,
    directories: Vec<(PathBuf, String)>,
}

impl Inventory {
    pub fn read(root: &Path, device: &str) -> Result<Self> {
        let mut scan = Self {
            entries: Vec::new(),
            folders: BTreeMap::new(),
            directories: Vec::new(),
        };
        let meta = fs::symlink_metadata(root)?;
        ensure!(
            meta.is_dir() && identity(&meta) == device,
            "Maildir root changed"
        );
        scan.visit(root, root)?;
        scan.validate()?;
        ensure!(
            identity(&fs::symlink_metadata(root)?) == device,
            "Maildir root changed during scan"
        );
        Ok(scan)
    }

    pub fn validate(&self) -> Result<()> {
        for (path, stamp) in &self.directories {
            ensure!(
                fingerprint(&fs::symlink_metadata(path)?) == *stamp,
                "Maildir changed during scan"
            );
        }
        Ok(())
    }

    fn directory(&mut self, path: &Path) -> Result<fs::ReadDir> {
        let meta = fs::symlink_metadata(path)?;
        ensure!(meta.is_dir(), "Expected Maildir directory");
        self.directories.push((path.to_owned(), fingerprint(&meta)));
        Ok(fs::read_dir(path)?)
    }

    fn visit(&mut self, root: &Path, path: &Path) -> Result<()> {
        let mut children = Vec::new();
        let mut parts = HashSet::new();
        for entry in self.directory(path)? {
            let entry = entry?;
            let name = entry.file_name();
            if matches!(name.to_str(), Some("cur" | "new" | "tmp")) {
                ensure!(entry.file_type()?.is_dir(), "Invalid Maildir subdirectory");
                parts.insert(name);
            } else if entry.file_type()?.is_dir() {
                children.push(entry.path());
            }
        }
        if !parts.is_empty() {
            ensure!(
                parts.contains(std::ffi::OsStr::new("cur"))
                    && parts.contains(std::ffi::OsStr::new("new")),
                "Incomplete Maildir at {}",
                path.display()
            );
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("Maildir folder name must be UTF-8")?;
            let folder = if relative.is_empty() {
                "INBOX".to_owned()
            } else {
                relative
                    .split('/')
                    .map(|part| {
                        part.strip_prefix('.')
                            .map_or_else(|| part.to_owned(), |s| s.replace('.', "/"))
                    })
                    .collect::<Vec<_>>()
                    .join("/")
            };
            ensure!(
                self.folders
                    .insert(folder.clone(), relative.to_owned())
                    .is_none(),
                "Ambiguous Maildir folder name"
            );
            let mut stems = HashSet::new();
            for subdir in ["cur", "new"] {
                for item in self.directory(&path.join(subdir))? {
                    let item = item?;
                    if !item.file_type()?.is_file()
                        || item.file_name().as_encoded_bytes().starts_with(b".")
                    {
                        continue;
                    }
                    let name = item
                        .file_name()
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("Maildir filename must be UTF-8"))?;
                    let (stem, info) = name.split_once(":2,").unwrap_or((&name, ""));
                    ensure!(
                        !stem.is_empty() && !stem.contains(':') && stems.insert(stem.to_owned()),
                        "Ambiguous or unsupported Maildir filename"
                    );
                    let (flags, extension) =
                        info.find(',').map_or((info, ""), |n| info.split_at(n));
                    let meta = fs::symlink_metadata(item.path())?;
                    ensure!(meta.is_file(), "Maildir entry changed");
                    self.entries.push(Entry {
                        path: item.path(),
                        folder: folder.clone(),
                        stem: stem.into(),
                        flags: flags.into(),
                        extension: extension.into(),
                        fingerprint: fingerprint(&meta),
                    });
                }
            }
        }
        for child in children {
            self.visit(root, &child)?;
        }
        Ok(())
    }
}

/// Distinguish a removed folder from a disconnected or partial directory.
pub(super) fn confirm_removed(root: &str, relative: &str) -> Result<()> {
    let mut path = PathBuf::from(root);
    for part in Path::new(relative).components() {
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(meta.is_dir(), "Maildir folder path was replaced"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!(
        "Previously connected mailbox is incomplete: {}",
        path.display()
    )
}
