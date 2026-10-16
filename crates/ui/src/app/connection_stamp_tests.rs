//! The connect stamp against a config file that cannot be written. Memory and disk may only
//! differ together with a banner, exactly as after a failed save (see `connections::save`).
use super::connection_ops::mark_connected;
use super::connections::ConfigPath;
use super::persistence::{PersistError, PersistGuard};
use super::state::{AppState, Banner};
use curlybrace_core::connection::ConnectionConfig;

/// A tempdir whose directory refuses new files, or `None` where permissions do not bite
/// (not unix, or running as a user who ignores them).
#[cfg(unix)]
fn read_only_dir() -> Option<tempfile::TempDir> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(dir.path().join("probe")).is_ok() {
        restore(&dir);
        return None;
    }
    Some(dir)
}

#[cfg(unix)]
fn restore(dir: &tempfile::TempDir) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn a_failed_write_shows_the_banner_and_leaves_disk_and_guard_honest() {
    let Some(dir) = read_only_dir() else {
        eprintln!("skipped: directory permissions do not apply here");
        return;
    };
    let file = dir.path().join("connections.json");
    let cfg = ConnectionConfig::new("a", "mongodb://h");
    let id = cfg.id;
    let state = AppState::new(vec![cfg]);
    let path: ConfigPath = Ok(file.clone());

    let next = mark_connected(state, id, 42, &path);

    restore(&dir);
    assert!(
        matches!(next.banner, Some(Banner::Persist(PersistError::Write(_)))),
        "{:?}",
        next.banner
    );
    // Nothing reached the disk, and the guard neither claims success nor locks saving.
    assert!(!file.exists());
    assert_eq!(next.persist, PersistGuard::default());
    // Same rule as a failed save: memory carries the stamp (the connection did happen) and the
    // banner is what tells the person the file is behind.
    assert_eq!(next.conn.connections[0].last_connected, Some(42));
}
