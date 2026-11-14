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

pub(super) fn app_in(
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

pub(super) fn pipeline_file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("pipelines.json")
}

pub(super) fn stage() -> Vec<SavedStage> {
    vec![SavedStage {
        text: r#"{"$match": {}}"#.into(),
        enabled: true,
    }]
}

pub(super) fn store_with(conn: Uuid, name: &str) -> SavedPipelines {
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

#[test]
fn a_list_too_big_for_the_file_is_refused_and_nothing_changes() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.state_mut()
        .persist_pipelines(store_with(conn, "small"))
        .unwrap();
    let on_disk = std::fs::read(pipeline_file(&dir)).unwrap();
    let before = app.state().pipelines.clone();
    let big = (0..6).fold(before.clone(), |store, i| {
        let stage = SavedStage {
            text: "x".repeat(900_000),
            enabled: true,
        };
        store
            .saved(conn, &format!("big{i}"), None, vec![stage], 2, false)
            .unwrap()
    });
    let result = app.state_mut().persist_pipelines(big);
    assert_eq!(result, Err(pipelines_store::PipelinesSaveError::TooLarge));
    assert_eq!(app.state().pipelines, before);
    assert_eq!(std::fs::read(pipeline_file(&dir)).unwrap(), on_disk);
}

#[test]
fn a_second_window_cannot_overwrite_what_the_first_saved() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut a, _ea, _ca) = app_in(&dir, Vec::new());
    let (mut b, _eb, _cb) = app_in(&dir, Vec::new());
    a.state_mut()
        .persist_pipelines(store_with(conn, "x"))
        .unwrap();
    let on_disk = std::fs::read(pipeline_file(&dir)).unwrap();

    let refused = b.state_mut().persist_pipelines(store_with(conn, "y"));
    assert_eq!(
        refused,
        Err(pipelines_store::PipelinesSaveError::ChangedOnDisk)
    );
    assert_eq!(std::fs::read(pipeline_file(&dir)).unwrap(), on_disk);
    let names: Vec<_> = b
        .state()
        .pipelines
        .list(conn)
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(names, ["x"], "the window now shows what is on disk");

    let retried = b
        .state()
        .pipelines
        .saved(conn, "y", None, stage(), 2, false);
    b.state_mut().persist_pipelines(retried.unwrap()).unwrap();
    let again = load(&pipeline_file(&dir), &persistence::back_up);
    assert!(again.settings.find_by_name(conn, "x").is_some());
    assert!(again.settings.find_by_name(conn, "y").is_some());
    assert_eq!(
        pipelines_store::PipelinesSaveError::ChangedOnDisk.i18n_key(),
        "aggregate.saved.error.changed_on_disk"
    );
}

#[test]
fn a_window_keeps_saving_when_only_it_changes_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let (mut a, _e, _c) = app_in(&dir, Vec::new());
    a.state_mut()
        .persist_pipelines(store_with(conn, "x"))
        .unwrap();
    let more = a
        .state()
        .pipelines
        .saved(conn, "y", None, stage(), 2, false);
    a.state_mut().persist_pipelines(more.unwrap()).unwrap();
    assert_eq!(a.state().pipelines.list(conn).len(), 2);
}

#[test]
fn a_newer_file_is_left_untouched_and_saving_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let conn = Uuid::new_v4();
    let newer = r#"{"version": 2, "connections": {}}"#;
    std::fs::write(pipeline_file(&dir), newer).unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::PipelinesNewer))
    );
    let result = app.state_mut().persist_pipelines(store_with(conn, "x"));
    assert_eq!(
        result,
        Err(pipelines_store::PipelinesSaveError::NewerVersion)
    );
    assert!(app.state().pipelines.list(conn).is_empty());
    assert_eq!(std::fs::read_to_string(pipeline_file(&dir)).unwrap(), newer);
}

#[test]
fn startup_notices_are_all_shown_one_after_another() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("tools.json"), "{ not json").unwrap();
    std::fs::write(pipeline_file(&dir), r#"{"version": 2, "connections": {}}"#).unwrap();
    let connections = Banner::Recovery(Recovery::Corrupt { backed_up: true });
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    std::mem::forget(commands);
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        Arc::new(MemorySecretStore::default()),
    )
    .with_banner(Some(connections.clone()))
    .with_tools_path(Ok(dir.path().join("tools.json")))
    .with_pipelines_path(Ok(pipeline_file(&dir)));
    let mut shown = vec![app.state().banner.clone()];
    for _ in 0..3 {
        app.dispatch(Input::DismissBanner);
        shown.push(app.state().banner.clone());
    }
    assert_eq!(
        shown,
        vec![
            Some(connections),
            Some(Banner::Recovery(Recovery::Tools { backed_up: true })),
            Some(Banner::Recovery(Recovery::PipelinesNewer)),
            None,
        ],
        "in the order the files were loaded, none lost"
    );
}
