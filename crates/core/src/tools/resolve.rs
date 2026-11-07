//! Choosing the executable to run and checking it is a real, runnable file.
use super::ToolKind;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs::Metadata;
use std::path::{Path, PathBuf};

/// Where a tool comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    /// The first matching executable on `PATH`.
    Path,
    /// This file, chosen by the user.
    Executable(PathBuf),
}

/// Why a tool cannot be used. Static and free of paths, so it is safe to show and to log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolError {
    NotAbsolute,
    Missing,
    NotAFile,
    NotExecutable,
    NotOnPath,
    CouldNotRun,
    TimedOut,
    NoVersion,
}

impl ToolError {
    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::NotAbsolute => "tools.error_not_absolute",
            Self::Missing => "tools.error_missing",
            Self::NotAFile => "tools.error_not_a_file",
            Self::NotExecutable => "tools.error_not_executable",
            Self::NotOnPath => "tools.error_not_on_path",
            Self::CouldNotRun => "tools.error_could_not_run",
            Self::TimedOut => "tools.error_timed_out",
            Self::NoVersion => "tools.error_no_version",
        }
    }
}

/// The file to run for `choice`. `path_var` is the value of `PATH` (passed in, never read here,
/// so tests are independent of the machine). Nothing is executed.
pub fn resolve(
    kind: ToolKind,
    choice: &ToolChoice,
    path_var: Option<&OsStr>,
) -> Result<PathBuf, ToolError> {
    match choice {
        ToolChoice::Executable(path) => check(path),
        ToolChoice::Path => search(kind.binary_name(), path_var),
    }
}

/// An absolute path to an existing, executable, regular file (links are followed).
fn check(path: &Path) -> Result<PathBuf, ToolError> {
    if !path.is_absolute() {
        return Err(ToolError::NotAbsolute);
    }
    let meta = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ToolError::Missing,
        std::io::ErrorKind::PermissionDenied => ToolError::NotExecutable,
        _ => ToolError::CouldNotRun,
    })?;
    if !meta.is_file() {
        return Err(ToolError::NotAFile);
    }
    if !is_executable(&meta) {
        return Err(ToolError::NotExecutable);
    }
    Ok(path.to_path_buf())
}

/// The first usable `name` in the absolute `PATH` entries. A relative entry (`.` or an empty
/// one) would run whatever sits in the working directory, so it is skipped.
fn search(name: &str, path_var: Option<&OsStr>) -> Result<PathBuf, ToolError> {
    let var = path_var.ok_or(ToolError::NotOnPath)?;
    std::env::split_paths(var)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| check(&dir.join(name)).ok())
        .ok_or(ToolError::NotOnPath)
}

#[cfg(unix)]
fn is_executable(meta: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_meta: &Metadata) -> bool {
    true
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
