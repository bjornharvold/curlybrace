//! Creating the export file with owner-only permissions and removing a file that must not stay.
//!
//! There is never a temp file and never a rename: the file is created at the chosen path and is
//! either kept whole (the job committed) or removed. Paths, system error text and contents are
//! never logged and never carried in an error.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use super::path::invalid_path;
use crate::error::{CoreError, Result};

/// Removes the file on drop unless [`commit`](Self::commit) was called, so a cancel, an error or
/// a panic can never leave a partial file behind. Created only once the file exists, so a path
/// that could not be opened is never removed.
#[derive(Debug)]
pub(in crate::runtime) struct FileGuard {
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
    pub(in crate::runtime) fn create(path: &Path) -> Result<(File, Self)> {
        let existed = fs::symlink_metadata(path).is_ok();
        let file = open_checked(path)?;
        let mut guard = Self {
            path: path.to_path_buf(),
            armed: true,
        };
        match prepare(&file, path) {
            Ok(()) => Ok((file, guard)),
            // It was already there and is still intact (`prepare` fails before it truncates):
            // leave it alone.
            Err(error) if existed => {
                guard.armed = false;
                Err(error)
            }
            // This call created it: remove it for real, and say so if that fails too.
            Err(error) if guard.remove() => Err(error),
            Err(_) => Err(CoreError::ExportFileNotRemoved),
        }
    }

    /// Keeps the file.
    pub(in crate::runtime) fn commit(mut self) {
        self.armed = false;
    }

    /// Removes the file now; true when it is gone.
    pub(in crate::runtime) fn remove(mut self) -> bool {
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
    super::test_hooks::inject(path)?;
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
