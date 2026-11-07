//! The tool paths inside the running app: they are saved when they change, checked on a worker
//! thread, and follow a connection that is deleted or duplicated. Every file lives in a tempdir.
use super::persistence::Recovery;
use super::tool_settings::{load, save};
use super::tools_state::{ProbeScope, ProbeStatus, ToolsAction};
use super::*;
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::tools::{ToolChoice, ToolError, ToolKind, ToolSettings, ToolVersion};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

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
    .with_tools_path(Ok(dir.path().join("tools.json")))
    // Nothing on the developer's machine may be run by a test.
    .with_tool_path_var(None);
    (app, events, commands)
}

fn fake_tool(dir: &tempfile::TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn exe(path: &std::path::Path) -> ToolChoice {
    ToolChoice::Executable(path.to_path_buf())
}

fn choose_global(app: &mut CurlybraceApp, kind: ToolKind, choice: ToolChoice) {
    app.dispatch(Input::Tools(ToolsAction::ChooseGlobal { kind, choice }));
}

/// Pumps the app until the check for `scope`/`kind` is no longer running.
fn finished(app: &mut CurlybraceApp, scope: ProbeScope, kind: ToolKind) -> ProbeStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.pump();
        match app.state().tools.status(scope, kind) {
            Some(ProbeStatus::Checking { .. }) | None if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Some(status) => return status.clone(),
            None => panic!("no check was started"),
        }
    }
}

#[test]
fn a_choice_is_on_disk_before_the_next_launch() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(&dir, "mongosh", "echo 2.3.1");
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    choose_global(&mut app, ToolKind::Mongosh, exe(&tool));
    let again = load(&dir.path().join("tools.json"), &persistence::back_up);
    assert_eq!(again.settings.global(ToolKind::Mongosh), exe(&tool));
}

#[test]
fn settings_found_at_launch_are_in_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let settings = ToolSettings::default().with_global(ToolKind::Mongodump, exe(dir.path()));
    save(&dir.path().join("tools.json"), &settings).unwrap();
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    assert_eq!(app.state().tools.settings, settings);
}

#[test]
fn an_unusable_file_at_launch_shows_the_notice_and_blocks_saving() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tools.json");
    std::fs::write(&path, "][").unwrap();
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::Tools { backed_up: true }))
    );
    assert_eq!(app.state().tools.settings, ToolSettings::default());
}

#[test]
fn a_refused_save_raises_the_banner_and_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tools.json");
    std::fs::write(&path, "][").unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    // Take the backup away from the guard: a blocked guard is what a failed backup leaves.
    app.tools_guard = persistence::PersistGuard::blocked();
    app.dispatch(Input::DismissBanner);
    choose_global(&mut app, ToolKind::Mongosh, ToolChoice::Path);
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::Tools { backed_up: false }))
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "][");
}

#[test]
fn a_choice_is_checked_and_the_version_shown() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(&dir, "any", "echo 'mongodump version: 100.10.0'");
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    choose_global(&mut app, ToolKind::Mongodump, exe(&tool));
    assert!(matches!(
        app.state()
            .tools
            .status(ProbeScope::Global, ToolKind::Mongodump),
        Some(ProbeStatus::Checking { .. })
    ));
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongodump),
        ProbeStatus::Done(Ok(ToolVersion::new(100, 10, 0)))
    );
}

#[test]
fn a_missing_file_is_reported_without_running_anything() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    choose_global(&mut app, ToolKind::Mongosh, exe(&dir.path().join("nope")));
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongosh),
        ProbeStatus::Done(Err(ToolError::Missing))
    );
}

#[test]
fn a_file_that_is_not_executable_is_never_run() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let tool = fake_tool(
        &dir,
        "tool",
        &format!("touch '{}'; echo 1.0.0", marker.display()),
    );
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o644)).unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    choose_global(&mut app, ToolKind::Mongosh, exe(&tool));
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongosh),
        ProbeStatus::Done(Err(ToolError::NotExecutable))
    );
    assert!(!marker.exists());
}

#[test]
fn a_connection_override_is_checked_with_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let global = fake_tool(&dir, "global", "echo 2.0.0");
    let mine = fake_tool(&dir, "mine", "echo 3.1.4");
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    choose_global(&mut app, ToolKind::Mongosh, exe(&global));
    app.dispatch(Input::Tools(ToolsAction::ChooseOverride {
        connection: id,
        kind: ToolKind::Mongosh,
        choice: exe(&mine),
    }));
    assert_eq!(
        finished(&mut app, ProbeScope::Connection(id), ToolKind::Mongosh),
        ProbeStatus::Done(Ok(ToolVersion::new(3, 1, 4)))
    );
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongosh),
        ProbeStatus::Done(Ok(ToolVersion::new(2, 0, 0)))
    );
}

#[test]
fn deleting_a_connection_drops_its_overrides_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    app.dispatch(Input::Tools(ToolsAction::ChooseOverride {
        connection: id,
        kind: ToolKind::Mongosh,
        choice: ToolChoice::Path,
    }));
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(!app.state().tools.settings.overrides(id, ToolKind::Mongosh));
    let again = load(&dir.path().join("tools.json"), &persistence::back_up);
    assert_eq!(again.settings, ToolSettings::default());
}

#[test]
fn a_delete_that_was_not_written_keeps_the_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    app.state_mut().persist = persistence::PersistGuard::blocked();
    app.dispatch(Input::Tools(ToolsAction::ChooseOverride {
        connection: id,
        kind: ToolKind::Mongosh,
        choice: ToolChoice::Path,
    }));
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(
        app.state().connection(id).is_some(),
        "the delete was refused"
    );
    assert!(app.state().tools.settings.overrides(id, ToolKind::Mongosh));
}

#[test]
fn a_duplicate_starts_with_the_overrides_of_its_source() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    let tool = fake_tool(&dir, "mine", "echo 1.0.0");
    app.dispatch(Input::Tools(ToolsAction::ChooseOverride {
        connection: id,
        kind: ToolKind::Mongosh,
        choice: exe(&tool),
    }));
    app.dispatch_connection(ConnectionAction::Duplicate(id));
    let copy = app
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != id)
        .expect("the copy")
        .id;
    assert_eq!(
        app.state()
            .tools
            .settings
            .effective(ToolKind::Mongosh, Some(copy)),
        exe(&tool)
    );
    let again = load(&dir.path().join("tools.json"), &persistence::back_up);
    assert!(again.settings.overrides(copy, ToolKind::Mongosh));
}

/// A picker that answers at once with a fixed file (or a cancel).
struct FixedPicker(Option<PathBuf>);

impl file_picker::FilePicker for FixedPicker {
    fn save_file(&self, _suggested_name: &str, _extension: &str) -> Option<PathBuf> {
        None
    }

    fn pick_executable(&self) -> Option<PathBuf> {
        self.0.clone()
    }
}

fn with_picker(app: CurlybraceApp, answer: Option<PathBuf>) -> CurlybraceApp {
    app.with_file_picker(Arc::new(FixedPicker(answer)), Arc::new(|| {}))
}

fn pumped_until(app: &mut CurlybraceApp, done: impl Fn(&AppState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app.state()) {
        assert!(Instant::now() < deadline, "the dialog never answered");
        app.pump();
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_file_chosen_in_the_dialog_becomes_the_choice_and_is_checked() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(&dir, "picked", "echo 2.9.0");
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    let mut app = with_picker(app, Some(tool.clone()));
    app.dispatch(Input::Tools(ToolsAction::Browse {
        scope: ProbeScope::Global,
        kind: ToolKind::Mongosh,
    }));
    pumped_until(&mut app, |s| s.tools.browsing().is_none());
    assert_eq!(
        app.state().tools.settings.global(ToolKind::Mongosh),
        exe(&tool)
    );
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongosh),
        ProbeStatus::Done(Ok(ToolVersion::new(2, 9, 0)))
    );
}

#[test]
fn a_cancelled_dialog_changes_nothing_and_a_new_one_can_open() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    let mut app = with_picker(app, None);
    app.dispatch(Input::Tools(ToolsAction::Browse {
        scope: ProbeScope::Global,
        kind: ToolKind::Mongodump,
    }));
    pumped_until(&mut app, |s| s.tools.browsing().is_none());
    assert_eq!(app.state().tools.settings, ToolSettings::default());
    app.dispatch(Input::Tools(ToolsAction::Browse {
        scope: ProbeScope::Global,
        kind: ToolKind::Mongodump,
    }));
    assert!(app.state().tools.browsing().is_some());
    pumped_until(&mut app, |s| s.tools.browsing().is_none());
}

#[test]
fn a_connection_override_can_be_picked_for_a_saved_connection() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(&dir, "mine", "echo 1.2.3");
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (app, _events, _commands) = app_in(&dir, vec![cfg]);
    let mut app = with_picker(app, Some(tool.clone()));
    app.dispatch(Input::Tools(ToolsAction::Browse {
        scope: ProbeScope::Connection(id),
        kind: ToolKind::Mongorestore,
    }));
    pumped_until(&mut app, |s| s.tools.browsing().is_none());
    assert!(
        app.state()
            .tools
            .settings
            .overrides(id, ToolKind::Mongorestore)
    );
}

#[test]
fn the_gear_opens_the_dialog_and_checks_every_global_tool() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.apply_ui_event(panels::UiEvent::OpenToolSettings);
    assert!(app.state().tools.dialog().is_some());
    for kind in ToolKind::ALL {
        assert!(
            app.state().tools.status(ProbeScope::Global, kind).is_some(),
            "{kind:?} was not checked"
        );
    }
}

#[test]
fn closing_the_dialog_keeps_what_was_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(&dir, "kept", "echo 1.0.0");
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    app.apply_ui_event(panels::UiEvent::OpenToolSettings);
    choose_global(&mut app, ToolKind::Mongosh, exe(&tool));
    app.dispatch(Input::Tools(ToolsAction::CloseSettings));
    assert!(app.state().tools.dialog().is_none());
    assert_eq!(
        app.state().tools.settings.global(ToolKind::Mongosh),
        exe(&tool)
    );
}

#[test]
fn a_choice_made_in_the_connection_dialog_is_saved_for_that_connection() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (mut app, _events, _commands) = app_in(&dir, vec![cfg]);
    app.dispatch_connection(ConnectionAction::Tools(ToolsAction::ChooseOverride {
        connection: id,
        kind: ToolKind::Mongodump,
        choice: ToolChoice::Path,
    }));
    assert!(
        app.state()
            .tools
            .settings
            .overrides(id, ToolKind::Mongodump)
    );
    let again = load(&dir.path().join("tools.json"), &persistence::back_up);
    assert!(again.settings.overrides(id, ToolKind::Mongodump));
}

#[test]
fn a_path_choice_searches_only_the_path_the_app_was_given() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(&dir, "mongodump", "echo 100.10.0");
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    let mut app = app.with_tool_path_var(Some(dir.path().as_os_str().to_owned()));
    choose_global(&mut app, ToolKind::Mongodump, ToolChoice::Path);
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongodump),
        ProbeStatus::Done(Ok(ToolVersion::new(100, 10, 0)))
    );
    let _ = tool;
}

#[test]
fn without_a_path_nothing_is_found_and_nothing_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _events, _commands) = app_in(&dir, Vec::new());
    choose_global(&mut app, ToolKind::Mongosh, ToolChoice::Path);
    assert_eq!(
        finished(&mut app, ProbeScope::Global, ToolKind::Mongosh),
        ProbeStatus::Done(Err(ToolError::NotOnPath))
    );
}
