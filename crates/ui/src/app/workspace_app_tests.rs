//! Writing `workspace.json`: when it is written, when it is not, and what a failure says.
use super::*;
use crate::app::AggregateAction;
use crate::app::persistence::{PersistError, Recovery};
use crate::app::pipelines_wiring_tests::app_in;
use crate::app::reducer::Input;
use crate::app::state::Banner;
use crate::app::tools_state::ToolsAction;
use crate::log_capture::{capture, records_mentioning};
use crate::viewmodel::{FindAction, SidebarAction};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Query};
use eframe::egui;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub(in crate::app) struct Rig {
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
pub(in crate::app) fn rig(with_path: bool) -> Rig {
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

#[test]
fn typing_in_a_stage_writes_once_after_a_second() {
    let mut rig = rig(true);
    rig.open("users");
    rig.app
        .dispatch(Input::Tab(crate::viewmodel::CollectionTab::Aggregate));
    rig.app
        .dispatch(Input::Aggregate(AggregateAction::AddStage("$match".into())));
    let t0 = Instant::now();
    rig.step(t0);
    let before = rig.writes();
    // Every edit makes a new preview request: that is not a Run.
    for i in 1..=3_u64 {
        let text = format!(r#"{{"$match": {{"n": {i}}}}}"#);
        rig.app
            .dispatch(Input::Aggregate(AggregateAction::Edit(0, text)));
        rig.step(t0 + Duration::from_millis(100 * i));
    }
    assert_eq!(rig.writes(), before, "typing waits");
    rig.step(t0 + Duration::from_millis(1100));
    assert_eq!(rig.writes(), before, "a second after the LAST change");
    rig.step(t0 + Duration::from_millis(1500));
    assert_eq!(rig.writes(), before + 1, "then once");
    rig.step(t0 + Duration::from_secs(9));
    assert_eq!(rig.writes(), before + 1);
}

#[test]
fn a_frame_in_which_nothing_changed_builds_no_snapshot() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    let built = rig.app.state().restore.sync.snapshots;
    assert!(built >= 1);
    for i in 1..=5_u64 {
        rig.step(t0 + Duration::from_millis(i));
    }
    assert_eq!(rig.app.state().restore.sync.snapshots, built);
    // A change builds one again.
    rig.app.state_mut().find.text.filter = "{}".into();
    rig.step(t0 + Duration::from_millis(10));
    assert_eq!(rig.app.state().restore.sync.snapshots, built + 1);
}

#[test]
fn a_runtime_that_stops_leaves_the_saved_tabs_in_the_file() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    assert_eq!(rig.on_disk().tabs.len(), 1);
    rig.commands.close();
    rig.open("orders");
    assert!(
        matches!(
            rig.app.state().banner,
            Some(Banner::Failure {
                context: "runtime",
                ..
            })
        ),
        "the runtime stopped"
    );
    rig.step(t0 + Duration::from_secs(1));
    rig.step(t0 + Duration::from_secs(2));
    rig.app.save_workspace_now();
    let names: Vec<String> = rig
        .on_disk()
        .tabs
        .into_iter()
        .map(|t| t.collection)
        .collect();
    assert_eq!(
        names,
        ["orders"],
        "the tab the strip had when the runtime stopped"
    );
}

fn backups(dir: &tempfile::TempDir) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("workspace.json."))
        .collect();
    found.sort();
    found
}

#[test]
fn turning_restore_off_deletes_the_backup_copies() {
    let mut rig = rig(true);
    for name in [
        "workspace.json.corrupt-1700000000",
        "workspace.json.skipped-1700000001-1",
        "workspace.json.unreadable-1",
        "workspace.json.repaired-1",
        "workspace.json.keep",
    ] {
        std::fs::write(rig.dir.path().join(name), b"SAVED QUERY TEXT").unwrap();
    }
    rig.app
        .dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    rig.step(Instant::now());
    assert_eq!(backups(&rig.dir), ["workspace.json.keep"]);
    assert_eq!(rig.app.state().banner, None);
}

#[test]
fn a_backup_that_cannot_be_deleted_raises_the_write_failed_banner() {
    let mut rig = rig(true);
    // A folder where a copy should be: it cannot be removed as a file.
    let stuck = rig.dir.path().join("workspace.json.corrupt-5");
    std::fs::create_dir(&stuck).unwrap();
    rig.app
        .dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    rig.step(Instant::now());
    assert!(matches!(
        rig.app.state().banner,
        Some(Banner::WorkspacePersist(PersistError::Write(_)))
    ));
}

#[test]
fn a_connection_list_that_failed_to_load_leaves_the_workspace_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let id = Uuid::new_v4();
    let kept = br#"{"version":1,"restore":true,"tabs":[{"connection":"00000000-0000-0000-0000-000000000009","db":"app","collection":"users","sub_tab":"documents","view_mode":"table"}]}"#;
    std::fs::write(&path, kept).unwrap();
    let _ = id;
    for damage in [
        Recovery::Corrupt { backed_up: true },
        Recovery::Unreadable { backed_up: false },
    ] {
        let (app, _events, _commands) = app_in(&dir, Vec::new());
        let mut app = app
            .with_banner(Some(Banner::Recovery(damage)))
            .with_workspace_path(Ok(path.clone()));
        let t0 = Instant::now();
        app.save_workspace_if_changed(&egui::Context::default(), t0);
        app.save_workspace_now();
        assert_eq!(std::fs::read(&path).unwrap(), kept, "the file is untouched");
        assert_eq!(
            backups(&dir),
            Vec::<String>::new(),
            "and nothing was backed up"
        );
        assert!(app.state().restore.sync.unverified, "writes are off");
    }
}

#[test]
fn the_switch_says_so_when_saving_is_turned_off() {
    let mut rig = rig(true);
    rig.app.state_mut().restore.sync.guard = PersistGuard::blocked();
    rig.app
        .dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    let banner = rig.app.state().banner.clone();
    assert_eq!(
        banner,
        Some(Banner::WorkspacePersist(PersistError::Blocked))
    );
    let i18n = crate::I18n::english();
    let text = crate::app::banner::banner_text(&banner.unwrap(), &i18n).0;
    assert_eq!(text, i18n.t("persist.workspace_blocked"));
}

fn lost_list_app(dir: &tempfile::TempDir, file: &[u8]) -> (CurlybraceApp, PathBuf) {
    let path = dir.path().join("workspace.json");
    std::fs::write(&path, file).unwrap();
    let (app, _events, _commands) = app_in(dir, Vec::new());
    // Leak the channels' ends so the runtime looks alive.
    std::mem::forget((_events, _commands));
    let app = app
        .with_banner(Some(Banner::Recovery(Recovery::Corrupt {
            backed_up: true,
        })))
        .with_workspace_path(Ok(path.clone()));
    (app, path)
}

const OFF: &[u8] = br#"{"version":1,"restore":false,"tabs":[]}"#;
const ON_WITH_TAB: &[u8] = br#"{"version":1,"restore":true,"tabs":[{"connection":"00000000-0000-0000-0000-000000000009","db":"app","collection":"users","sub_tab":"documents","view_mode":"table"}]}"#;

#[test]
fn a_lost_connection_list_still_shows_what_the_file_says_about_the_setting() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = lost_list_app(&dir, OFF);
    assert!(
        !app.state().restore.restore,
        "the switch shows Off, as the file says"
    );
    let (app, _) = lost_list_app(&dir, ON_WITH_TAB);
    assert!(app.state().restore.restore);
}

#[test]
fn turning_off_with_a_lost_connection_list_erases_the_text_and_the_backups() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path) = lost_list_app(&dir, ON_WITH_TAB);
    std::fs::write(dir.path().join("workspace.json.corrupt-1"), b"SAVED TEXT").unwrap();
    // A clean banner slot: the switch works here, so it raises no notice of its own.
    app.state_mut().banner = None;
    app.dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    assert_eq!(app.state().banner, None, "it works: nothing to say");
    app.save_workspace_if_changed(&egui::Context::default(), Instant::now());
    let file: WorkspaceFile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(file, WorkspaceFile::without_tabs(false));
    assert_eq!(backups(&dir), Vec::<String>::new());
}

#[test]
fn turning_on_with_a_lost_connection_list_says_saving_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, path) = lost_list_app(&dir, OFF);
    app.state_mut().banner = None;
    app.dispatch(Input::Tools(ToolsAction::SetRestore(true)));
    assert_eq!(
        app.state().banner,
        Some(Banner::WorkspacePersist(PersistError::Blocked))
    );
    app.save_workspace_if_changed(&egui::Context::default(), Instant::now());
    assert_eq!(std::fs::read(&path).unwrap(), OFF, "and nothing is written");
}

#[test]
fn the_switch_says_so_whenever_saving_is_off_even_without_a_place_to_save() {
    let mut rig = rig(false);
    rig.app
        .dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    assert_eq!(
        rig.app.state().banner,
        Some(Banner::WorkspacePersist(PersistError::Blocked))
    );
}

#[test]
fn a_partly_loaded_connection_list_keeps_saving_on_and_keeps_unknown_tabs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    std::fs::write(&path, ON_WITH_TAB).unwrap();
    let cfg = ConnectionConfig::new("a", "mongodb://localhost");
    let id = cfg.id;
    let (app, events, _commands) = app_in(&dir, vec![cfg]);
    let mut app = app
        .with_banner(Some(Banner::Recovery(Recovery::Skipped {
            count: 1,
            backed_up: true,
        })))
        .with_workspace_path(Ok(path.clone()));
    assert!(!app.state().restore.sync.unverified, "saving is on");
    assert_eq!(
        app.state().restore.dormant.len(),
        1,
        "the unknown tab waits"
    );
    assert_eq!(
        backups(&dir),
        Vec::<String>::new(),
        "nothing was skipped or backed up"
    );
    // A change of the open tabs writes, and the written file holds the unknown tab too.
    app.dispatch(Input::Connect(id));
    let epoch = app.state().epoch_counter;
    events
        .send(Event::Connected {
            session: id,
            epoch,
            topology: None,
        })
        .unwrap();
    app.pump();
    app.dispatch(Input::Sidebar(SidebarAction::Open(Namespace::new(
        "app", "newone",
    ))));
    let before = app.state().restore.sync.writes;
    app.save_workspace_if_changed(&egui::Context::default(), Instant::now());
    assert_eq!(
        app.state().restore.sync.writes,
        before + 1,
        "a write happened"
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert_ne!(written.as_bytes(), ON_WITH_TAB);
    assert!(written.contains("newone"), "the new tab: {written}");
    assert!(
        written.contains("00000000-0000-0000-0000-000000000009") && written.contains("users"),
        "and the unknown connection's tab is in what was written: {written}"
    );
    std::mem::forget((events, _commands));
}

#[test]
fn what_was_typed_after_a_failed_write_is_never_written_later() {
    let mut rig = rig(true);
    rig.open("users");
    let t0 = Instant::now();
    rig.step(t0);
    // The file can no longer be written; X is typed and its write fails.
    let blocker = rig.dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    rig.app.state_mut().restore.sync.path = Some(blocker.join("workspace.json"));
    rig.app.state_mut().find.text.filter = "X".into();
    rig.step(t0 + Duration::from_millis(100));
    rig.step(t0 + Duration::from_millis(1200));
    assert!(rig.app.state().restore.sync.failed.is_some(), "X failed");
    // Y is typed, then X is typed back; saving works again.
    rig.app.state_mut().find.text.filter = "Y".into();
    rig.step(t0 + Duration::from_millis(1300));
    rig.app.state_mut().find.text.filter = "X".into();
    rig.step(t0 + Duration::from_millis(1400));
    rig.app.state_mut().restore.sync.path = Some(rig.path());
    let before = rig.writes();
    rig.step(t0 + Duration::from_secs(5));
    rig.step(t0 + Duration::from_secs(9));
    let saved = rig.on_disk().tabs.remove(0).find;
    assert!(
        saved.as_ref().is_none_or(|f| f.filter != "Y"),
        "Y was written: {saved:?}"
    );
    assert_eq!(rig.writes(), before, "nothing was written");
}

#[test]
fn turning_restore_off_also_deletes_leftover_temp_files_and_only_those() {
    let mut rig = rig(true);
    let dir = rig.dir.path().to_path_buf();
    for name in [
        ".workspace.json.tmp-1-2-3",
        "connections.json",
        "workspace.json.keep",
        ".connections.json.tmp-9",
    ] {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
    rig.app
        .dispatch(Input::Tools(ToolsAction::SetRestore(false)));
    rig.step(Instant::now());
    let mut left: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "workspace.json" && n != "pipelines.json")
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            ".connections.json.tmp-9",
            "connections.json",
            "workspace.json.keep"
        ]
    );
}

#[test]
fn the_backup_of_an_oversized_file_is_read_at_most_to_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.json");
    let size = crate::app::workspace_file::MAX_FILE_BYTES + 5000;
    std::fs::write(&path, vec![b'x'; size]).unwrap();
    let (app, _e, _c) = app_in(&dir, Vec::new());
    std::mem::forget((_e, _c));
    let _app = app.with_workspace_path(Ok(path));
    let copy = backups(&dir)
        .into_iter()
        .find(|n| n.contains("corrupt"))
        .expect("a backup copy");
    let kept = std::fs::metadata(dir.path().join(copy)).unwrap().len();
    assert_eq!(kept, crate::app::workspace_file::MAX_FILE_BYTES as u64 + 1);
}

#[test]
fn aggregate_options_and_mode_survive_a_relaunch_through_the_real_file() {
    use curlybrace_core::pipeline_options::PipelineOptions;
    let options = PipelineOptions {
        comment: Some("agg".into()),
        allow_disk_use: Some(true),
        ..PipelineOptions::default()
    };
    let mut first = rig(true);
    first.open("users");
    first
        .app
        .dispatch(Input::Tab(crate::viewmodel::CollectionTab::Aggregate));
    for text in [r#"{"$match": {}}"#, r#"{"$out": "dest"}"#] {
        first
            .app
            .dispatch(Input::Aggregate(AggregateAction::AddStage("$match".into())));
        let at = first.app.state().aggregate.stages.len() - 1;
        first
            .app
            .dispatch(Input::Aggregate(AggregateAction::Edit(at, text.into())));
    }
    first.app.state_mut().aggregate.options = options.clone();
    first
        .app
        .dispatch(Input::Aggregate(AggregateAction::SetMode(
            crate::viewmodel::aggregate::Mode::Text,
        )));
    first.app.save_workspace_now();
    // A new app on the same folder: it reads the file the first one wrote.
    let id = first.id;
    let cfg = first.app.state().connection(id).cloned().unwrap();
    let (app, events, _commands) = app_in(&first.dir, vec![cfg]);
    let mut app = app.with_workspace_path(Ok(first.path()));
    app.dispatch(Input::Connect(id));
    let epoch = app.state().epoch_counter;
    events
        .send(Event::Connected {
            session: id,
            epoch,
            topology: None,
        })
        .unwrap();
    app.pump();
    let s = app.state();
    assert_eq!(s.aggregate.mode, crate::viewmodel::aggregate::Mode::Text);
    assert_eq!(s.aggregate.options, options);
    app.dispatch(Input::Aggregate(AggregateAction::RunClicked));
    assert!(
        app.state().aggregate.run.dialog.is_some(),
        "Run asks before the $out"
    );
    std::mem::forget(events);
}
