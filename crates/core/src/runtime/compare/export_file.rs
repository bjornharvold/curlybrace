//! The file side of an export: validating the chosen path, creating the file with owner-only
//! permissions, removing a file that must not stay ([`FileGuard`]), and replacing the header once
//! the true counts are known.
//!
//! There is never a temp file and never a rename: the file is created at the chosen path and is
//! either kept whole (the job committed) or removed. Paths, system error text and contents are
//! never logged and never carried in an error.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

const COPY_CHUNK: usize = 64 * 1024;

fn invalid_path() -> CoreError {
    CoreError::Unsupported("invalid export path".into())
}

/// Refuses a path the export must not touch: relative, without an existing directory as parent,
/// an existing directory, a symlink (it could point anywhere) or any other non-regular file.
/// A missing file is fine (it is created). Synchronous: two metadata calls.
pub(super) fn validate_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.file_name().is_none() {
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
    pub(super) fn create(path: &Path) -> Result<(File, Self)> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options
            .open(path)
            .map_err(|_| CoreError::ExportWriteFailed)?;
        let guard = Self {
            path: path.to_path_buf(),
            armed: true,
        };
        restrict(&file)?;
        Ok((file, guard))
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

/// Replaces the first `old_len` bytes of `file` by `header`, which must not be longer, and
/// shifts the rest of the file down in place (bounded memory, no second file). Returns the new
/// length. Used because the header holds counts that are only known after the last operation.
pub(super) fn replace_header(file: &mut File, old_len: u64, header: &[u8]) -> io::Result<u64> {
    let total = file.metadata()?.len();
    let new_len = header.len() as u64;
    let shift = old_len
        .checked_sub(new_len)
        .filter(|_| old_len <= total)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(header)?;
    if shift > 0 {
        shift_down(file, old_len, total, shift)?;
        file.set_len(total - shift)?;
    }
    file.flush()?;
    Ok(total - shift)
}

/// Copies `from..total` to `from - shift..`, front to back: a chunk is always written below the
/// bytes still to be read.
fn shift_down(file: &mut File, from: u64, total: u64, shift: u64) -> io::Result<()> {
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut at = from;
    while at < total {
        let want = COPY_CHUNK.min((total - at) as usize);
        file.seek(SeekFrom::Start(at))?;
        let read = file.read(&mut buf[..want])?;
        if read == 0 {
            break;
        }
        file.seek(SeekFrom::Start(at - shift))?;
        file.write_all(&buf[..read])?;
        at += read as u64;
    }
    Ok(())
}
