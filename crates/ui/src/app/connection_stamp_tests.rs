//! The connect stamp against a config file that cannot be written. Memory and disk may only
//! differ together with a banner, exactly as after a failed save (see `connections::save`).
use super::connection_ops::mark_connected;
use super::connections::ConfigPath;
use super::persistence::{PersistError, PersistGuard};
use super::state::{AppState, Banner};
use curlybrace_core::connection::ConnectionConfig;

#[test]
fn a_failed_write_shows_the_banner_and_leaves_disk_and_guard_honest() {
    // The config path sits below a regular FILE, so creating the temp file fails with
    // NotADirectory for everyone, root included (no permission juggling, nothing to skip).
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"x").unwrap();
    let file = blocker.join("connections.json");
    let cfg = ConnectionConfig::new("a", "mongodb://h");
    let id = cfg.id;
    let state = AppState::new(vec![cfg]);
    let path: ConfigPath = Ok(file.clone());

    let next = mark_connected(state, id, 42, &path);

    assert!(
        matches!(next.banner, Some(Banner::Persist(PersistError::Write(_)))),
        "{:?}",
        next.banner
    );
    // Nothing reached the disk, and the guard neither claims success nor locks saving.
    assert!(!file.exists());
    assert_eq!(std::fs::read(&blocker).unwrap(), b"x");
    assert_eq!(next.persist, PersistGuard::default());
    // Same rule as a failed save: memory carries the stamp (the connection did happen) and the
    // banner is what tells the person the file is behind.
    assert_eq!(next.conn.connections[0].last_connected, Some(42));
}
