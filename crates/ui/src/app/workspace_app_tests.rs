//! Writing `workspace.json`: when it is written, when it is not, and what a failure says.
use super::*;
use crate::app::pipelines_wiring_tests::app_in;
use crate::app::reducer::Input;
use crate::app::state::Banner;
use crate::log_capture::{capture, records_mentioning};
use crate::viewmodel::{FindAction, SidebarAction};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Query};
use eframe::egui;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

struct Rig {
    app: CurlybraceApp,
    events: UnboundedSender<Event>,
    commands: UnboundedReceiver<Command>,
    dir: tempfile::TempDir,
    id: Uuid,
    ctx: egui::Context,
}

impl Rig {
    fn path(&self) -> PathBuf {
        self.dir.path().join("workspace.json")
    }

    fn writes(&self) -> u64 {
        self.app.state().restore.sync.writes
    }

    fn on_disk(&self) -> WorkspaceFile {
        serde_json::from_slice(&std::fs::read(self.path()).unwrap()).unwrap()
    }

    fn step(&mut self, at: Instant) {
        let ctx = self.ctx.clone();
        self.app.save_workspace_if_changed(&ctx, at);
    }

    fn open(&mut self, collection: &str) {
        let ns = Namespace::new("app", collection);
        self.app.dispatch(Input::Sidebar(SidebarAction::Open(ns)));
    }
}

/// A connected app with a workspace path in its own folder.
fn rig(with_path: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (app, events, commands) = app_in(&dir, vec![cfg]);
    let app = match with_path {
        true => app.with_workspace_path(Ok(dir.path().join("workspace.json"))),
        false => app,
    };
    let mut rig = Rig {
        app,
        events,
        commands,
        dir,
        id,
        ctx: egui::Context::default(),
    };
    rig.app.dispatch(Input::Connect(id));
    let epoch = rig.app.state().epoch_counter;
    rig.events
        .send(Event::Connected {
            session: id,
            epoch,
            topology: None,
        })
        .unwrap();
    rig.app.pump();
    rig
}

#[test]
fn new_app_never_writes_the_real_config_dir() {
    let mut rig = rig(false);
    rig.open("users");
    let now = Instant::now();
    rig.step(now);
    rig.app.save_workspace_now();
    assert!(rig.app.state().restore.sync.path.is_none());
    assert_eq!(rig.writes(), 0);
    assert!(!rig.path().exists());
    assert_eq!(rig.app.state().banner, None);
    // `launch` is what sets the real path, and it is the file beside the connection list.
    if let Ok(path) = workspace_file::default_workspace_path() {
        assert_eq!(path.file_name().unwrap(), "workspace.json");
    }
}

#[test]
fn nothing_is_written_when_nothing_changed() {
    let mut rig = rig(true);
    let t0 = Instant::now();
    rig.step(t0);
    rig.step(t0 + Duration::from_secs(5));
    assert_eq!(rig.writes(), 0, "an empty session is the file as it is");
    assert!(!rig.path().exists());
}

#[test]
fn structural_change_writes_once() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    assert_eq!(rig.writes(), 1, "written at once");
    assert_eq!(rig.on_disk().tabs.len(), 1);
    assert_eq!(rig.on_disk().tabs[0].collection, "users");
    rig.step(t0 + Duration::from_millis(10));
    rig.step(t0 + Duration::from_secs(10));
    assert_eq!(rig.writes(), 1, "and only once");
}

#[test]
fn typing_writes_after_a_second_only_once() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    assert_eq!(rig.writes(), 1);
    rig.app.state_mut().find.text.filter = r#"{"a": 1}"#.into();
    rig.step(t0);
    rig.step(t0 + Duration::from_millis(500));
    assert_eq!(rig.writes(), 1, "typing waits");
    // More typing starts the second again.
    rig.app.state_mut().find.text.filter = r#"{"a": 12}"#.into();
    rig.step(t0 + Duration::from_millis(900));
    rig.step(t0 + Duration::from_millis(1500));
    assert_eq!(rig.writes(), 1, "a second after the LAST change");
    rig.step(t0 + Duration::from_millis(1950));
    assert_eq!(rig.writes(), 2);
    let saved = rig.on_disk().tabs.remove(0).find.unwrap();
    assert_eq!(saved.filter, r#"{"a": 12}"#);
    rig.step(t0 + Duration::from_secs(30));
    assert_eq!(rig.writes(), 2, "once");
}

#[test]
fn run_writes_the_pending_text_at_once() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    rig.app.state_mut().find.text.filter = r#"{"b": 2}"#.into();
    rig.step(t0 + Duration::from_millis(10));
    assert_eq!(rig.writes(), 1);
    rig.app
        .dispatch(Input::Find(FindAction::Run(Query::default())));
    rig.step(t0 + Duration::from_millis(20));
    assert_eq!(rig.writes(), 2, "Run does not wait for the second");
    assert_eq!(
        rig.on_disk().tabs[0]
            .find
            .as_ref()
            .map(|f| f.filter.as_str()),
        Some(r#"{"b": 2}"#)
    );
}

#[test]
fn exit_writes() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    rig.app.state_mut().find.text.filter = r#"{"c": 3}"#.into();
    rig.app.save_workspace_now();
    assert_eq!(rig.writes(), 2);
    assert_eq!(
        rig.on_disk().tabs[0]
            .find
            .as_ref()
            .map(|f| f.filter.as_str()),
        Some(r#"{"c": 3}"#)
    );
    rig.app.save_workspace_now();
    assert_eq!(rig.writes(), 2, "nothing new to write");
}

#[test]
fn blocked_guard_never_writes() {
    let mut rig = rig(true);
    rig.app.state_mut().restore.sync.guard = PersistGuard::blocked();
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    rig.step(t0 + Duration::from_secs(5));
    rig.app.save_workspace_now();
    assert_eq!(rig.writes(), 0);
    assert!(!rig.path().exists());
    assert_eq!(
        rig.app.state().banner,
        None,
        "a blocked file was already announced"
    );
}

#[test]
fn write_failure_raises_one_banner() {
    let mut rig = rig(true);
    // A folder that cannot be made: its parent is a file.
    let blocker = rig.dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    rig.app.state_mut().restore.sync.path = Some(blocker.join("workspace.json"));
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    assert_eq!(rig.writes(), 0);
    assert!(matches!(
        rig.app.state().banner,
        Some(Banner::WorkspacePersist(_))
    ));
    // The person dismisses it; the failure goes on without another banner, and without a retry
    // of the same content on every frame.
    rig.app.dispatch(Input::DismissBanner);
    rig.step(t0 + Duration::from_secs(1));
    rig.open("orders");
    rig.step(t0 + Duration::from_secs(2));
    assert_eq!(
        rig.app.state().banner,
        None,
        "one banner per failure streak"
    );
    // Saving works again: the streak ends, and the next failure speaks again.
    rig.app.state_mut().restore.sync.path = Some(rig.path());
    rig.open("things");
    rig.step(t0 + Duration::from_secs(3));
    assert_eq!(rig.writes(), 1);
    rig.app.state_mut().restore.sync.path = Some(blocker.join("workspace.json"));
    rig.open("more");
    rig.step(t0 + Duration::from_secs(4));
    assert!(matches!(
        rig.app.state().banner,
        Some(Banner::WorkspacePersist(_))
    ));
}

#[test]
fn no_log_line_contains_query_text() {
    capture();
    let sentinel = "SENTINEL_FILTER_VALUE_7f3a";
    let mut rig = rig(true);
    rig.open("users");
    rig.app.state_mut().find.text.filter = format!(r#"{{"name": "{sentinel}"}}"#);
    let t0 = Instant::now();
    rig.step(t0);
    rig.app.save_workspace_now();
    assert!(rig.writes() >= 1, "a write happened");
    // A failing write logs too.
    let blocker = rig.dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    rig.app.state_mut().restore.sync.path = Some(blocker.join("workspace.json"));
    rig.open("orders");
    rig.step(t0 + Duration::from_secs(2));
    rig.app.save_workspace_now();
    // So does reading a file that is not usable.
    std::fs::write(
        rig.dir.path().join("bad.json"),
        format!("not json {sentinel}"),
    )
    .unwrap();
    let _ = AppState::default().with_workspace_path(Ok(rig.dir.path().join("bad.json")));
    assert!(
        !records_mentioning("could not save the open tabs").is_empty(),
        "the failure was logged"
    );
    assert!(records_mentioning(sentinel).is_empty());
    let _ = (&rig.commands, rig.id);
}
