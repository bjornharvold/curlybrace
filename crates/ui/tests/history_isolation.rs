//! The UI test harnesses spawn a real runtime; none may record index history in the developer's
//! real config directory. Fast and offline.
mod support;

use curlybrace_core::runtime::{default_history_path, default_snapshot_dir};
use curlybrace_ui::app::host_key::InteractivePrompt;
use std::sync::Arc;
use std::time::Duration;
use support::fixture::harness_runtime_config;

#[test]
fn ui_test_harnesses_never_use_the_real_history_path() {
    let dir = tempfile::tempdir().unwrap();
    let (prompt, _rx) = InteractivePrompt::channel(Arc::new(|| {}), Duration::from_secs(5));
    let config = harness_runtime_config(dir.path(), prompt);
    let history = config.history_path.expect("the harness picks its own path");
    assert_ne!(Some(history.clone()), default_history_path());
    assert!(history.starts_with(dir.path()));
}

/// Undo copies hold the person's documents in clear text: a test harness must never leave them in
/// the developer's real config directory.
#[test]
fn ui_test_harnesses_never_use_the_real_snapshot_dir() {
    let dir = tempfile::tempdir().unwrap();
    let (prompt, _rx) = InteractivePrompt::channel(Arc::new(|| {}), Duration::from_secs(5));
    let config = harness_runtime_config(dir.path(), prompt);
    let snapshots = config.snapshot_dir.expect("the harness picks its own path");
    assert_ne!(Some(snapshots.clone()), default_snapshot_dir());
    assert!(snapshots.starts_with(dir.path()));
}
