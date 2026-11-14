//! A connection cleanup of the saved pipelines when the file is not what this window last wrote:
//! a file another window left unreadable is backed up and then replaced by the same cleanup, and
//! a cleanup that still fails says so in words that are true. Every file lives in a tempdir.
use super::persistence;
use super::pipelines_store::{self, load};
use super::pipelines_wiring_tests::{app_in, pipeline_file, stage, store_with};
use super::*;
use curlybrace_core::saved_pipeline::SavedPipelines;
use std::cell::Cell;
use std::os::unix::fs::PermissionsExt;
use uuid::Uuid;

fn backups(dir: &tempfile::TempDir) -> usize {
    std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| {
            let name = e.as_ref().unwrap().file_name();
            name.to_string_lossy().starts_with("pipelines.json.")
        })
        .count()
}

/// A window that saved one pipeline for `cfg` and one for `other`.
fn app_with_two(dir: &tempfile::TempDir, cfg: ConnectionConfig, other: Uuid) -> CurlybraceApp {
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(dir, vec![cfg]);
    let both = store_with(id, "mine")
        .saved(other, "theirs", None, stage(), 2, false)
        .unwrap();
    app.state_mut().persist_pipelines(both).unwrap();
    app
}

fn config() -> ConnectionConfig {
    ConnectionConfig::new("local", "mongodb://localhost:27017")
}

#[test]
fn a_delete_after_another_window_left_garbage_leaves_a_valid_file_without_the_entries() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config();
    let (id, other) = (cfg.id, Uuid::new_v4());
    let mut app = app_with_two(&dir, cfg, other);
    std::fs::write(pipeline_file(&dir), "{ garbage").unwrap();

    app.dispatch_connection(ConnectionAction::Delete(id));

    let loaded = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(loaded.recovery, None, "the file is valid again");
    assert!(loaded.settings.list(id).is_empty(), "no dead entries");
    assert_eq!(loaded.settings.list(other).len(), 1, "the others are kept");
    assert_eq!(app.state().pipelines, loaded.settings);
    assert_eq!(backups(&dir), 1, "the garbage was kept");
    assert_ne!(app.state().banner, Some(Banner::PipelinesCleanup));
}

#[test]
fn a_duplicate_after_another_window_left_garbage_writes_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config();
    let (id, other) = (cfg.id, Uuid::new_v4());
    let mut app = app_with_two(&dir, cfg, other);
    std::fs::write(pipeline_file(&dir), "{ garbage").unwrap();

    app.dispatch_connection(ConnectionAction::Duplicate(id));

    let loaded = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(loaded.recovery, None);
    let copies = loaded.settings.list(id).len() + loaded.settings.list(other).len();
    assert_eq!(copies, 2, "the originals stay");
    assert_eq!(app.state().pipelines, loaded.settings);
    assert_ne!(app.state().banner, Some(Banner::PipelinesCleanup));
}

/// A cleanup that removes `id`'s entries; on its `on`th call it also lets another window write.
fn cleanup_hit_by_a_writer(
    dir: &tempfile::TempDir,
    id: Uuid,
    on: usize,
) -> impl Fn(&SavedPipelines) -> SavedPipelines {
    let calls = Cell::new(0);
    let path = pipeline_file(dir);
    move |store| {
        if calls.replace(calls.get() + 1) == on {
            let rival = store_with(Uuid::new_v4(), "rival");
            pipelines_store::save(&path, &rival).unwrap();
        }
        store.without_connection(id)
    }
}

#[test]
fn a_cleanup_that_loses_twice_says_what_is_true() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config();
    let (id, other) = (cfg.id, Uuid::new_v4());
    let mut app = app_with_two(&dir, cfg, other);
    // Another window wrote once before the cleanup and writes again during its retry.
    let rival = store_with(id, "first rival");
    pipelines_store::save(&pipeline_file(&dir), &rival).unwrap();
    let cleanup = cleanup_hit_by_a_writer(&dir, id, 1);

    app.state_mut().apply_pipelines_cleanup(cleanup);

    assert_eq!(app.state().banner, Some(Banner::PipelinesCleanup));
    assert!(app.state().pipelines.list(id).is_empty(), "applied here");
    let text = I18n::english().t("aggregate.saved.cleanup_failed");
    assert!(!text.contains("Nothing was changed"), "{text}");
    assert!(text.contains("updated in this window"), "{text}");
}

#[test]
fn a_blocked_file_says_the_cleanup_failed_not_that_the_list_starts_empty() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config();
    let (id, other) = (cfg.id, Uuid::new_v4());
    let mut app = app_with_two(&dir, cfg, other);
    std::fs::write(pipeline_file(&dir), "{ garbage").unwrap();
    // Backups are made beside the file: a folder that takes no new files stops them.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(dir.path().join("probe"), "x").is_ok();
    app.state_mut()
        .apply_pipelines_cleanup(|store| store.without_connection(id));
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return; // permissions not enforced (root): the failure cannot be produced
    }
    assert_eq!(app.state().banner, Some(Banner::PipelinesCleanup));
    assert!(app.state().pipelines.list(id).is_empty());
    assert_eq!(
        std::fs::read_to_string(pipeline_file(&dir)).unwrap(),
        "{ garbage",
        "an unreadable file is never overwritten unseen"
    );
}
