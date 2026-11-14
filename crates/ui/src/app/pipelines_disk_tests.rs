//! The saved pipelines file when it changes under a window: reloads, cleanups that follow, a
//! file that cannot be read and a file of a newer version. Every file lives in a tempdir.
use super::persistence;
use super::pipelines_store::{self, load};
use super::pipelines_wiring_tests::{app_in, pipeline_file, stage, store_with};
use super::*;
use std::os::unix::fs::PermissionsExt;
use uuid::Uuid;

/// Window `a` has saved an entry for `id`; window `b` (started before) then reloads it and adds
/// one of its own for `other`, so `a` no longer knows what is on disk.
fn two_windows(
    dir: &tempfile::TempDir,
    cfg: ConnectionConfig,
    other: Uuid,
) -> (CurlybraceApp, CurlybraceApp) {
    let id = cfg.id;
    let (mut a, _ea, _ca) = app_in(dir, vec![cfg]);
    let (mut b, _eb, _cb) = app_in(dir, Vec::new());
    a.state_mut()
        .persist_pipelines(store_with(id, "from a"))
        .unwrap();
    let refused = b.state_mut().persist_pipelines(store_with(other, "x"));
    assert_eq!(
        refused,
        Err(pipelines_store::PipelinesSaveError::ChangedOnDisk)
    );
    let theirs = b
        .state()
        .pipelines
        .saved(other, "from b", None, stage(), 2, false)
        .unwrap();
    b.state_mut().persist_pipelines(theirs).unwrap();
    (a, b)
}

#[test]
fn a_delete_refused_by_a_change_on_disk_is_cleaned_up_after_the_reload() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (id, other) = (cfg.id, Uuid::new_v4());
    let (mut a, _b) = two_windows(&dir, cfg, other);
    a.dispatch_connection(ConnectionAction::Delete(id));
    assert!(a.state().connection(id).is_none());
    assert!(
        !matches!(a.state().banner, Some(Banner::PipelinesPersist(_))),
        "the cleanup went through"
    );
    let on_disk = load(&pipeline_file(&dir), &persistence::back_up).settings;
    assert!(on_disk.list(id).is_empty(), "no dead entries are left");
    assert_eq!(on_disk.list(other)[0].name, "from b");
    assert_eq!(a.state().pipelines, on_disk);
}

#[test]
fn a_duplicate_refused_by_a_change_on_disk_still_gets_its_copies() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let (id, other) = (cfg.id, Uuid::new_v4());
    let (mut a, _b) = two_windows(&dir, cfg, other);
    a.dispatch_connection(ConnectionAction::Duplicate(id));
    let copy = a
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != id)
        .expect("the copy")
        .id;
    assert!(!matches!(
        a.state().banner,
        Some(Banner::PipelinesPersist(_))
    ));
    let on_disk = load(&pipeline_file(&dir), &persistence::back_up).settings;
    assert_eq!(on_disk.list(copy)[0].name, "from a");
    assert_eq!(on_disk.list(id).len(), 1);
    assert_eq!(on_disk.list(other)[0].name, "from b");
}

fn backups(dir: &tempfile::TempDir) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("pipelines.json."))
        })
        .collect()
}

#[test]
fn a_file_turned_to_garbage_is_backed_up_then_replaced_by_the_next_save() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut()
        .persist_pipelines(store_with(conn, "mine"))
        .unwrap();
    std::fs::write(pipeline_file(&dir), "{ garbage").unwrap();

    let first = app.state_mut().persist_pipelines(store_with(conn, "next"));
    assert_eq!(
        first,
        Err(pipelines_store::PipelinesSaveError::UnreadableOnDisk)
    );
    assert_eq!(
        std::fs::read_to_string(pipeline_file(&dir)).unwrap(),
        "{ garbage",
        "the file is left as it was"
    );
    let kept = backups(&dir);
    assert_eq!(kept.len(), 1);
    assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "{ garbage");
    assert_eq!(app.state().pipelines.list(conn)[0].name, "mine");

    let again = app
        .state()
        .pipelines
        .saved(conn, "next", None, stage(), 2, false)
        .unwrap();
    app.state_mut().persist_pipelines(again).unwrap();
    let loaded = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(loaded.recovery, None);
    assert_eq!(loaded.settings.list(conn).len(), 2);
    assert_eq!(backups(&dir).len(), 1, "no second copy was needed");
    assert_eq!(
        pipelines_store::PipelinesSaveError::UnreadableOnDisk.i18n_key(),
        "aggregate.saved.error.unreadable_on_disk"
    );
}

#[test]
fn a_file_over_the_cap_is_treated_like_a_corrupt_one() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut()
        .persist_pipelines(store_with(conn, "mine"))
        .unwrap();
    let huge = " ".repeat(pipelines_store::MAX_FILE_BYTES + 1);
    std::fs::write(pipeline_file(&dir), &huge).unwrap();
    let first = app.state_mut().persist_pipelines(store_with(conn, "next"));
    assert_eq!(
        first,
        Err(pipelines_store::PipelinesSaveError::UnreadableOnDisk)
    );
    assert_eq!(backups(&dir).len(), 1);
    let again = app.state().pipelines.clone();
    app.state_mut().persist_pipelines(again).unwrap();
}

#[test]
fn an_unreadable_file_that_cannot_be_backed_up_is_never_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut()
        .persist_pipelines(store_with(conn, "mine"))
        .unwrap();
    std::fs::write(pipeline_file(&dir), "{ garbage").unwrap();
    // Backups are made beside the file: a folder that takes no new files stops them.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let writable = std::fs::write(dir.path().join("probe"), "x").is_ok();
    let first = app.state_mut().persist_pipelines(store_with(conn, "next"));
    let second = app.state_mut().persist_pipelines(store_with(conn, "next"));
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return; // permissions not enforced (root): the failure cannot be produced
    }
    assert_eq!(first, Err(pipelines_store::PipelinesSaveError::Blocked));
    assert_eq!(second, Err(pipelines_store::PipelinesSaveError::Blocked));
    assert_eq!(
        std::fs::read_to_string(pipeline_file(&dir)).unwrap(),
        "{ garbage"
    );
}

const NEWER_FILE: &str = r#"{"version": 2, "connections": {}}"#;

#[test]
fn the_newer_file_block_lifts_when_the_file_is_removed() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    std::fs::write(pipeline_file(&dir), NEWER_FILE).unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    let refused = app.state_mut().persist_pipelines(store_with(conn, "x"));
    assert_eq!(
        refused,
        Err(pipelines_store::PipelinesSaveError::NewerVersion)
    );
    std::fs::remove_file(pipeline_file(&dir)).unwrap();
    app.state_mut()
        .persist_pipelines(store_with(conn, "x"))
        .unwrap();
    let again = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(again.settings.list(conn).len(), 1);
}

#[test]
fn the_newer_file_block_lifts_when_the_file_becomes_version_one_but_reloads_first() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    std::fs::write(pipeline_file(&dir), NEWER_FILE).unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    // Another window (or the user) leaves a version 1 file with an entry of its own.
    pipelines_store::save(&pipeline_file(&dir), &store_with(conn, "theirs")).unwrap();
    let first = app.state_mut().persist_pipelines(store_with(conn, "mine"));
    assert_eq!(
        first,
        Err(pipelines_store::PipelinesSaveError::ChangedOnDisk)
    );
    assert_eq!(app.state().pipelines.list(conn)[0].name, "theirs");
    let next = app
        .state()
        .pipelines
        .saved(conn, "mine", None, stage(), 2, false)
        .unwrap();
    app.state_mut().persist_pipelines(next).unwrap();
    let again = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(again.settings.list(conn).len(), 2);
}

#[test]
fn a_newer_file_written_mid_session_refuses_the_save_and_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut()
        .persist_pipelines(store_with(conn, "mine"))
        .unwrap();
    std::fs::write(pipeline_file(&dir), NEWER_FILE).unwrap();
    let refused = app.state_mut().persist_pipelines(store_with(conn, "next"));
    let error = refused.unwrap_err();
    assert_eq!(error, pipelines_store::PipelinesSaveError::NewerVersion);
    assert_eq!(error.i18n_key(), "aggregate.saved.error.unsupported");
    assert_eq!(
        std::fs::read_to_string(pipeline_file(&dir)).unwrap(),
        NEWER_FILE
    );
    assert_eq!(app.state().pipelines.list(conn)[0].name, "mine");
    let text = I18n::english().t(error.i18n_key());
    assert!(
        !text.contains("not loaded"),
        "the list is loaded in this case: {text}"
    );
    assert!(text.contains("newer version") && text.contains("untouched"));
}

#[test]
fn a_file_that_is_still_newer_keeps_the_unsupported_message() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    std::fs::write(pipeline_file(&dir), NEWER_FILE).unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    for _ in 0..2 {
        let refused = app.state_mut().persist_pipelines(store_with(conn, "x"));
        let error = refused.unwrap_err();
        assert_eq!(error.i18n_key(), "aggregate.saved.error.unsupported");
    }
}
