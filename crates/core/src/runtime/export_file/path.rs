//! The rules for the path a person chose for an export file.

use std::fs;
use std::io;
use std::path::Path;

use crate::error::{CoreError, Result};

pub(super) fn invalid_path() -> CoreError {
    CoreError::Unsupported("invalid export path".into())
}

/// Refuses a path the export must not touch: relative, without an existing directory as parent,
/// an existing directory, a symlink (it could point anywhere) or any other non-regular file.
/// A missing file is fine (it is created). Synchronous: two metadata calls.
pub(in crate::runtime) fn validate_path(path: &Path) -> Result<()> {
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
