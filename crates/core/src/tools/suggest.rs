//! Where the tools usually are, for the settings screen to offer. Only looks at files; nothing
//! is ever run.
use super::{ToolChoice, ToolKind, resolve};
use std::path::{Path, PathBuf};

/// Folders package managers install command-line tools into. A GUI app started from the Finder
/// does not have these on its `PATH`, which is why they are offered by name.
pub const COMMON_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// The runnable files named like `kind` in `dirs`, in the order of the folders.
pub fn suggestions_in(kind: ToolKind, dirs: &[&Path]) -> Vec<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(kind.binary_name()))
        .filter(|path| resolve(kind, &ToolChoice::Executable(path.clone()), None).is_ok())
        .collect()
}

/// [`suggestions_in`] the common folders.
pub fn suggestions(kind: ToolKind) -> Vec<PathBuf> {
    let dirs: Vec<&Path> = COMMON_DIRS.iter().map(Path::new).collect();
    suggestions_in(kind, &dirs)
}

#[cfg(test)]
#[path = "suggest_tests.rs"]
mod tests;
