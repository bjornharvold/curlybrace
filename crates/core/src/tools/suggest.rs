//! Where the tools usually are, for the settings screen to offer. Only looks at files; nothing
//! is ever run.
use super::{ToolChoice, ToolKind, resolve};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// Folders package managers install command-line tools into. A GUI app started from the Finder
/// does not have these on its `PATH`, so they are searched as well and offered by name.
#[cfg(target_os = "macos")]
pub const COMMON_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];
#[cfg(all(unix, not(target_os = "macos")))]
pub const COMMON_DIRS: &[&str] = &["/usr/local/bin", "/usr/bin"];
#[cfg(not(unix))]
pub const COMMON_DIRS: &[&str] = &[];

/// The runnable files named like `kind` in `dirs`, in the order of the folders.
pub fn suggestions_in(kind: ToolKind, dirs: &[&Path]) -> Vec<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(kind.file_name()))
        .filter(|path| resolve(kind, &ToolChoice::Executable(path.clone()), None).is_ok())
        .collect()
}

/// `base` followed by the folders of `extra` it does not already have, in that order. A
/// relative entry of `base` is kept as it is; callers pass it on only after filtering.
pub fn search_path(base: Option<&OsStr>, extra: &[&str]) -> OsString {
    let mut dirs: Vec<PathBuf> = base
        .map(|b| std::env::split_paths(b).collect())
        .unwrap_or_default();
    for dir in extra {
        let dir = PathBuf::from(dir);
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// `base` plus the common folders: what a tool is searched for in. An app started from the
/// Finder has a `PATH` without the Homebrew folder.
pub fn default_search_path(base: Option<&OsStr>) -> OsString {
    search_path(base, COMMON_DIRS)
}

/// [`suggestions_in`] the common folders.
pub fn suggestions(kind: ToolKind) -> Vec<PathBuf> {
    let dirs: Vec<&Path> = COMMON_DIRS.iter().map(Path::new).collect();
    suggestions_in(kind, &dirs)
}

#[cfg(test)]
#[path = "suggest_tests.rs"]
mod tests;
