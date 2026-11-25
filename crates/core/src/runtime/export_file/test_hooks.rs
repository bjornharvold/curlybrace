//! Failure injection for the export file tests. Compiled only for tests.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{CoreError, Result};

static FAILING: Mutex<Vec<(PathBuf, bool)>> = Mutex::new(Vec::new());

/// `create` fails right after opening `path`; with `lock_parent` the parent directory also
/// becomes read-only first, so the removal fails too (the test restores it).
pub(in crate::runtime) fn fail_after_open(path: &Path, lock_parent: bool) {
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
