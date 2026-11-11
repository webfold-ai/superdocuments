//! A `FileProvider` that reads `\input` files from one directory tree on disk.
//!
//! This is the only part of the LaTeX reader that touches the file system, and it is used only
//! when a caller asks for it (`DirFiles::new`, or `document::convert_path` on a `.tex` file).
//! Everything it serves is inside the root directory it was given:
//!
//! - an absolute name, or a name whose `..` parts climb above the root, is refused;
//! - the resolved path is canonicalised and must still be under the canonical root, so a
//!   symbolic link that points outside the root is refused;
//! - only regular files are read, each at most `max_file_bytes`, and all of them together at
//!   most `max_total_bytes`.
//!
//! A refusal is recorded (`DirFiles::refused`) and the file is reported as missing, so the
//! parser adds its usual "not resolved" warning and the name lands in `Document::inputs`.

use std::cell::{Cell, RefCell};
use std::path::{Component, Path, PathBuf};

use super::FileProvider;

/// Largest single file `DirFiles` reads by default.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 8 << 20;
/// Most bytes `DirFiles` reads in total by default.
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 64 << 20;

/// Reads `\input`, `\include` and `\subfile` names relative to one directory, and nothing
/// outside it. Names resolve against the root (TeX resolves them against the directory of the
/// main file, not of the including file).
#[derive(Debug)]
pub struct DirFiles {
    root: PathBuf,
    exclude: Option<PathBuf>,
    max_file_bytes: u64,
    remaining: Cell<u64>,
    refused: RefCell<Vec<String>>,
}

impl DirFiles {
    /// A provider rooted at `root`, with the default size limits. Fails when `root` is not a
    /// directory that can be resolved.
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::with_limits(root, DEFAULT_MAX_FILE_BYTES, DEFAULT_MAX_TOTAL_BYTES)
    }

    /// A provider with explicit limits: the largest single file, and the most bytes in total.
    pub fn with_limits(
        root: impl AsRef<Path>,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> std::io::Result<Self> {
        let root = std::fs::canonicalize(root)?;
        if !root.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "the root of a DirFiles provider must be a directory",
            ));
        }
        Ok(Self {
            root,
            exclude: None,
            max_file_bytes,
            remaining: Cell::new(max_total_bytes),
            refused: RefCell::new(Vec::new()),
        })
    }

    /// Never serves `main` (the document being read), so a file cannot pull the main document
    /// into itself.
    pub fn excluding(mut self, main: impl AsRef<Path>) -> Self {
        self.exclude = std::fs::canonicalize(main).ok();
        self
    }

    /// The canonical root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every name that was refused, with the reason, in the order asked. A name that simply
    /// does not exist is not a refusal.
    pub fn refused(&self) -> Vec<String> {
        self.refused.borrow().clone()
    }

    fn refuse(&self, name: &str, why: &str) -> Option<String> {
        self.refused.borrow_mut().push(format!("{name}: {why}"));
        None
    }

    /// Reads a text file under the root by a relative name (also used for `.bib` and `.bbl`
    /// files). UTF-8 when valid, otherwise Latin-1. `None` when the file does not exist or
    /// was refused.
    pub fn read(&self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() || name.contains('\0') {
            return None;
        }
        // Resolve `.` and `..` by hand first: a name that climbs above the root is refused
        // whether or not anything exists there.
        let mut relative = PathBuf::new();
        for part in Path::new(name).components() {
            match part {
                Component::Normal(p) => relative.push(p),
                Component::CurDir => {}
                Component::ParentDir => {
                    if !relative.pop() {
                        return self.refuse(name, "the path leaves the root directory");
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return self.refuse(name, "absolute paths are not read");
                }
            }
        }
        if relative.as_os_str().is_empty() {
            return None;
        }
        // A missing file is not a refusal. Canonicalising follows symbolic links, so the
        // check below sees where the file really is.
        let path = std::fs::canonicalize(self.root.join(&relative)).ok()?;
        if !path.starts_with(&self.root) {
            return self.refuse(name, "the file resolves outside the root directory");
        }
        if self.exclude.as_deref() == Some(path.as_path()) {
            return self.refuse(name, "the main document is not read as an input");
        }
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() {
            return None;
        }
        if meta.len() > self.max_file_bytes {
            return self.refuse(name, "the file is over the size limit");
        }
        if meta.len() > self.remaining.get() {
            return self.refuse(name, "the total size limit was reached");
        }
        let bytes = std::fs::read(&path).ok()?;
        // The file may have grown since `metadata`.
        let len = bytes.len() as u64;
        if len > self.max_file_bytes || len > self.remaining.get() {
            return self.refuse(name, "the file is over the size limit");
        }
        self.remaining.set(self.remaining.get() - len);
        Some(match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(e) => e.into_bytes().iter().map(|&b| char::from(b)).collect(),
        })
    }
}

impl FileProvider for DirFiles {
    fn read_tex(&self, name: &str) -> Option<String> {
        self.read(name)
    }
}
