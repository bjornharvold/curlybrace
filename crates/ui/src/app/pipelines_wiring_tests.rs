//! The saved pipelines inside the running app: the store follows a connection that is deleted
//! or duplicated, and a write that fails leaves the list as it was. Every file lives in a
//! tempdir.
use super::persistence::{PersistError, Recovery};
use super::pipelines_store::load;
use super::*;
use curlybrace_core::saved_pipeline::{SavedPipelines, SavedStage};
use curlybrace_core::secrets::MemorySecretStore;
use std::os::unix::fs::PermissionsExt;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

fn app_in(
    dir: &tempfile::TempDir,
    connections: Vec<ConnectionConfig>,
) -> (
    CurlybraceApp,
    UnboundedSender<Event>,
    UnboundedReceiver<Command>,
) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, rx) = tokio::sync::mpsc::unbounded_channel();
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        connections,
        Arc::new(MemorySecretStore::default()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")))
    .with_pipelines_path(Ok(dir.path().join("pipelines.json")));
    (app, events, commands)
}

fn pipeline_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("pipelines.json")
}

fn stage() -> Vec<SavedStage> {
    vec![SavedStage {
        text: r#"{"$match": {}}"#.into(),
        enabled: true,
    }]
}

fn store_with(conn: Uuid, name: &str) -> SavedPipelines {
    SavedPipelines::default()
        .saved(conn, name, None, stage(), 1, false)
        .unwrap()
}

#[test]
fn a_saved_list_is_on_disk_before_the_next_launch() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut()
        .persist_pipelines(store_with(conn, "totals"))
        .unwrap();
    assert_eq!(app.state().pipelines.list(conn).len(), 1);
    let again = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(again.settings, app.state().pipelines);
}

#[test]
fn a_new_app_loads_what_an_earlier_one_saved() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut first, _e, _c) = app_in(&dir, Vec::new());
    first
        .state_mut()
        .persist_pipelines(store_with(conn, "totals"))
        .unwrap();
    let (second, _e2, _c2) = app_in(&dir, Vec::new());
    assert_eq!(second.state().pipelines.list(conn)[0].name, "totals");
}

#[test]
fn a_corrupt_file_is_backed_up_with_a_recovery_banner() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(pipeline_file(&dir), "{ nope").unwrap();
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::Pipelines { backed_up: true }))
    );
    assert_eq!(app.state().pipelines, SavedPipelines::default());
}

#[test]
fn a_blocked_guard_refuses_the_save_and_leaves_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut().pipelines_guard = persistence::PersistGuard::blocked();
    let result = app.state_mut().persist_pipelines(store_with(conn, "x"));
    assert_eq!(result, Err(pipelines_store::PipelinesSaveError::Blocked));
    assert!(app.state().pipelines.list(conn).is_empty());
    assert!(!pipeline_file(&dir).exists());
}

#[test]
fn an_app_without_a_path_cannot_save_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    let conn = Uuid::new_v4();
    app.state_mut().pipelines_path = None;
    let result = app.state_mut().persist_pipelines(store_with(conn, "x"));
    assert_eq!(
        result,
        Err(pipelines_store::PipelinesSaveError::Persist(
            PersistError::ConfigDirUnknown
        ))
    );
    assert!(app.state().pipelines.list(conn).is_empty());
}

#[test]
fn a_failed_write_leaves_the_list_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("readonly");
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::write(folder.join("probe"), "x").is_ok() {
        return; // permissions not enforced (root): the failure cannot be produced
    }
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    let mut app = app.with_pipelines_path(Ok(folder.join("pipelines.json")));
    let conn = Uuid::new_v4();
    let result = app.state_mut().persist_pipelines(store_with(conn, "x"));
    assert!(matches!(
        result,
        Err(pipelines_store::PipelinesSaveError::Persist(
            PersistError::Write(_)
        ))
    ));
    assert!(app.state().pipelines.list(conn).is_empty());
}

#[test]
fn deleting_a_connection_removes_its_pipelines_only() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let other = ConnectionConfig::new("other", "mongodb://localhost:27018");
    let (id, other_id) = (cfg.id, other.id);
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg, other]);
    let both = store_with(id, "mine").with_connection_copied(id, other_id);
    app.state_mut().persist_pipelines(both).unwrap();
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(app.state().pipelines.list(id).is_empty());
    assert_eq!(app.state().pipelines.list(other_id).len(), 1);
    let again = load(&pipeline_file(&dir), &persistence::back_up);
    assert!(again.settings.list(id).is_empty());
    assert_eq!(again.settings.list(other_id).len(), 1);
}

#[test]
fn a_delete_that_was_not_written_keeps_the_pipelines() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    app.state_mut().persist = persistence::PersistGuard::blocked();
    app.state_mut()
        .persist_pipelines(store_with(id, "mine"))
        .unwrap();
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(
        app.state().connection(id).is_some(),
        "the delete was refused"
    );
    assert_eq!(app.state().pipelines.list(id).len(), 1);
}

#[test]
fn a_duplicate_starts_with_the_pipelines_of_its_source() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    app.state_mut()
        .persist_pipelines(store_with(id, "mine"))
        .unwrap();
    app.dispatch_connection(ConnectionAction::Duplicate(id));
    let copy = app
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != id)
        .expect("the copy")
        .id;
    assert_eq!(app.state().pipelines.list(copy)[0].name, "mine");
    assert_eq!(app.state().pipelines.list(id).len(), 1);
    let again = load(&pipeline_file(&dir), &persistence::back_up);
    assert_eq!(again.settings.list(copy).len(), 1);
}

#[test]
fn a_cleanup_that_cannot_be_written_is_reported_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    app.state_mut()
        .persist_pipelines(store_with(id, "mine"))
        .unwrap();
    app.state_mut().pipelines_guard = persistence::PersistGuard::blocked();
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(app.state().pipelines.list(id).is_empty());
    assert!(app.state().banner.is_some(), "the failed write is shown");
}
