//! The file side of an export: validating the chosen path, creating the file with owner-only
//! permissions, removing a file that must not stay ([`FileGuard`]), and replacing the header once
//! the true counts are known.
//!
//! There is never a temp file and never a rename: the file is created at the chosen path and is
//! either kept whole (the job committed) or removed. Paths, system error text and contents are
//! never logged and never carried in an error.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

fn invalid_path() -> CoreError {
    CoreError::Unsupported("invalid export path".into())
}

/// Refuses a path the export must not touch: relative, without an existing directory as parent,
/// an existing directory, a symlink (it could point anywhere) or any other non-regular file.
/// A missing file is fine (it is created). Synchronous: two metadata calls.
pub(super) fn validate_path(path: &Path) -> Result<()> {
    let trailing_separator = path
        .as_os_str()
        .to_string_lossy()
        .ends_with(std::path::is_separator);
    if !path.is_absolute() || path.file_name().is_none() || trailing_separator {
        return Err(invalid_path());
    }
    let parent_is_dir = path
        .parent()
        .is_some_and(|p| fs::metadata(p).is_ok_and(|m| m.is_dir()));
    if !parent_is_dir {
        return Err(invalid_path());
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(invalid_path()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(invalid_path()),
    }
}

/// Removes the file on drop unless [`commit`](Self::commit) was called, so a cancel, an error or
/// a panic can never leave a partial file behind. Created only once the file exists, so a path
/// that could not be opened is never removed.
#[derive(Debug)]
pub(super) struct FileGuard {
    path: PathBuf,
    armed: bool,
}

impl FileGuard {
    /// Creates (or truncates) the file at `path`, readable and writable by its owner only, even
    /// when it existed before with wider permissions. The guard is armed from here on.
    ///
    /// On unix the open does not follow a symlink at `path` and does not block on a fifo; what
    /// was opened is then checked (a regular file with one link) BEFORE it is truncated, so a
    /// link to somebody else's file is never emptied. Anything else is an invalid path.
    pub(super) fn create(path: &Path) -> Result<(File, Self)> {
        let file = open_checked(path)?;
        let guard = Self {
            path: path.to_path_buf(),
            armed: true,
        };
        match prepare(&file, path) {
            Ok(()) => Ok((file, guard)),
            // The file exists now: remove it for real, and say so if that fails too.
            Err(error) if guard.remove() => Err(error),
            Err(_) => Err(CoreError::ExportFileNotRemoved),
        }
    }

    /// Keeps the file.
    pub(super) fn commit(mut self) {
        self.armed = false;
    }

    /// Removes the file now; true when it is gone.
    pub(super) fn remove(mut self) -> bool {
        self.armed = false;
        match fs::remove_file(&self.path) {
            Ok(()) => true,
            Err(e) => e.kind() == io::ErrorKind::NotFound,
        }
    }
}

impl Drop for FileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Restricts a freshly opened file to its owner and empties it.
fn prepare(file: &File, path: &Path) -> Result<()> {
    #[cfg(test)]
    test_hooks::inject(path)?;
    #[cfg(not(test))]
    let _ = path;
    restrict(file)?;
    file.set_len(0).map_err(|_| CoreError::ExportWriteFailed)
}

#[cfg(unix)]
fn open_checked(path: &Path) -> Result<File> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options.open(path).map_err(|e| match e.raw_os_error() {
        Some(libc::ELOOP) => invalid_path(),
        _ => CoreError::ExportWriteFailed,
    })?;
    let meta = file.metadata().map_err(|_| CoreError::ExportWriteFailed)?;
    if !meta.is_file() || meta.nlink() != 1 {
        return Err(invalid_path());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_checked(path: &Path) -> Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .open(path)
        .map_err(|_| CoreError::ExportWriteFailed)
}

#[cfg(unix)]
fn restrict(file: &File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| CoreError::ExportWriteFailed)
}

#[cfg(not(unix))]
fn restrict(_: &File) -> Result<()> {
    Ok(())
}

/// Overwrites the first `expected_len` bytes of `file` with `header`, which must have exactly
/// that length: a small in-place write, nothing is moved. Data is synced before (so the body is
/// on disk before the file claims to be complete) and after (before the caller keeps the file).
pub(super) fn overwrite_header(
    file: &mut File,
    expected_len: u64,
    header: &[u8],
) -> io::Result<()> {
    if header.len() as u64 != expected_len {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    file.sync_all()?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(header)?;
    file.flush()?;
    file.sync_all()
}

#[cfg(test)]
pub(super) mod test_hooks {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use crate::error::{CoreError, Result};

    static FAILING: Mutex<Vec<(PathBuf, bool)>> = Mutex::new(Vec::new());

    /// `create` fails right after opening `path`; with `lock_parent` the parent directory also
    /// becomes read-only first, so the removal fails too (the test restores it).
    pub(in super::super) fn fail_after_open(path: &Path, lock_parent: bool) {
        FAILING
            .lock()
            .unwrap()
            .push((path.to_path_buf(), lock_parent));
    }

    pub(super) fn inject(path: &Path) -> Result<()> {
        let hit = FAILING
            .lock()
            .unwrap()
            .iter()
            .find(|(p, _)| p == path)
            .cloned();
        let Some((_, lock_parent)) = hit else {
            return Ok(());
        };
        #[cfg(unix)]
        if lock_parent {
            use std::os::unix::fs::PermissionsExt;
            let parent = path.parent().unwrap();
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o500));
        }
        let _ = lock_parent;
        Err(CoreError::ExportWriteFailed)
    }
}
