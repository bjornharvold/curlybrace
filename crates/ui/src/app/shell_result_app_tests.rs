//! The structured result of a shell run inside the app: where `ShellResult` events go, the
//! output-mode and tree actions, and what a refusal or a lost connection does to the result.
use crate::app::index_write_tests::{browsing, with_flags};
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::state::AppState;
use crate::viewmodel::shell::{ShellNotice, ShellState};
use crate::viewmodel::shell_result::{Availability, OutputMode, Reason, ResultState};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event, ShellRefusal};
use curlybrace_core::shell::{DocKind, Outcome, ShellResult};
use curlybrace_core::test_support::mock_objects::mock_connection;
use curlybrace_core::tools::{ToolChoice, ToolKind};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// A runnable stand-in for `mongosh` in `dir` (never run by these tests).
fn fake_mongosh(dir: &Path) -> PathBuf {
    let path = dir.join("mongosh");
    std::fs::write(&path, "#!/bin/sh\necho 2.3.0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn shell(s: AppState, action: ShellAction) -> (AppState, Vec<Command>) {
    update(s, Input::Shell(action))
}

fn event(s: AppState, ev: Event) -> AppState {
    update(s, Input::Event(ev)).0
}

fn shell_of(s: &AppState, id: Uuid) -> &ShellState {
    s.shell.get(id).expect("the session's shell")
}

fn with_exe(s: AppState, id: Uuid, exe: &Path) -> AppState {
    let tools = s.tools.choose_override(
        id,
        ToolKind::Mongosh,
        ToolChoice::Executable(exe.to_path_buf()),
    );
    let s = AppState { tools, ..s };
    let edit = ShellAction::Edit {
        script: "db.people.find()".into(),
        selection: None,
    };
    shell(s, edit).0
}

fn people() -> ShellResult {
    ShellResult::Documents {
        kind: DocKind::Cursor,
        docs: vec![
            doc! { "_id": 1_i32, "addr": { "city": "Oslo" } },
            doc! { "_id": 2_i32 },
        ],
        more: false,
        total: None,
    }
}

fn result(session: Uuid, run_id: u64) -> Event {
    Event::ShellResult {
        session,
        run_id,
        result: people(),
    }
}

fn finished(session: Uuid, run_id: u64) -> Event {
    Event::ShellFinished {
        session,
        run_id,
        outcome: Outcome::Exited(0),
    }
}

/// A connected session whose shell is running its first run.
fn running(dir: &Path) -> (AppState, Uuid) {
    let (s, id) = browsing(with_flags(false, false));
    let s = with_exe(s, id, &fake_mongosh(dir));
    let (s, _) = shell(s, ShellAction::Run);
    (s, id)
}

/// A connected session whose first run returned `people()` and finished cleanly.
fn with_result(dir: &Path) -> (AppState, Uuid) {
    let (s, id) = running(dir);
    let s = event(s, result(id, 1));
    (event(s, finished(id, 1)), id)
}

fn connect_and_answer(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    event(
        s,
        Event::Connected {
            session: id,
            epoch,
            topology: None,
        },
    )
}

#[test]
fn the_result_event_updates_this_sessions_state() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = running(dir.path());
    assert_eq!(shell_of(&s, id).result.state, ResultState::Pending);

    let s = event(s, result(id, 1));
    let sh = shell_of(&s, id);
    assert_eq!(sh.result.state, ResultState::Ready);
    assert_eq!(sh.result.result.as_deref(), Some(&people()));
    let s = event(s, finished(id, 1));
    assert_eq!(shell_of(&s, id).table_docs().len(), 2);

    let s = event(s, result(id, 1));
    assert_eq!(
        shell_of(&s, id).result.state,
        ResultState::Ready,
        "late copy"
    );
}

#[test]
fn a_result_event_for_another_session_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let a_cfg = with_flags(false, false);
    let b_cfg = ConnectionConfig {
        name: "other".into(),
        ..mock_connection()
    };
    let (a, b) = (a_cfg.id, b_cfg.id);
    let s = connect_and_answer(AppState::new(vec![a_cfg, b_cfg]), a);
    let s = connect_and_answer(s, b);
    let (s, _) = update(s, Input::Activate(a));
    let (s, _) = shell(with_exe(s, a, &exe), ShellAction::Run);
    let (s, _) = update(s, Input::Activate(b));
    let (s, _) = shell(with_exe(s, b, &exe), ShellAction::Run);
    let before_a = shell_of(&s, a).clone();

    let s = event(s, result(b, 1));
    assert_eq!(shell_of(&s, a), &before_a);
    assert_eq!(shell_of(&s, b).result.state, ResultState::Ready);

    let tabs = s.shell.clone();
    let s = event(s, result(Uuid::new_v4(), 1));
    assert_eq!(s.shell, tabs, "an unknown session changes nothing");
}

#[test]
fn set_output_mode_action_changes_the_mode() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = with_result(dir.path());
    assert_eq!(shell_of(&s, id).mode, OutputMode::Text);

    let (s, commands) = shell(s, ShellAction::SetOutputMode(OutputMode::Table));
    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).mode, OutputMode::Table);
    let (s, _) = shell(s, ShellAction::Run);
    assert_eq!(
        shell_of(&s, id).mode,
        OutputMode::Table,
        "kept for the next run"
    );
}

#[test]
fn tree_actions_reach_the_active_shell() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = with_result(dir.path());
    let addr = vec!["addr".to_string()];

    let (s, commands) = shell(s, ShellAction::TreeToggle(0, addr.clone()));
    assert!(commands.is_empty());
    assert!(shell_of(&s, id).result.tree.is_expanded(0, &addr));
    let (s, _) = shell(s, ShellAction::TreeCollapseAll);
    assert!(!shell_of(&s, id).result.tree.is_expanded(0, &addr));
    let (s, _) = shell(s, ShellAction::TreeExpandAll);
    assert!(shell_of(&s, id).result.tree.is_expanded(0, &addr));
}

fn lost(id: Uuid) -> Event {
    Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    }
}

#[test]
fn a_refused_run_leaves_no_result() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = with_result(dir.path());
    let (s, _) = shell(s, ShellAction::Run);
    assert_eq!(shell_of(&s, id).result.state, ResultState::Pending);
    let refused = Event::ShellRefused {
        session: id,
        run_id: 2,
        reason: ShellRefusal::AlreadyRunning,
    };
    let s = event(s, refused);
    let sh = shell_of(&s, id);
    assert_eq!(
        sh.result.result, None,
        "the started run cleared run 1's value"
    );
    assert_eq!(sh.result.state, ResultState::NoResult);
    assert_eq!(
        sh.availability(OutputMode::Table),
        Availability::Unavailable(Reason::RunFailed)
    );
}

/// Refused by the app before it starts: nothing ran, so the previous run's text and value stay.
#[test]
fn a_run_refused_before_it_starts_keeps_the_previous_result() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = with_result(dir.path());
    let before = shell_of(&s, id).clone();
    let s = AppState {
        read_only: true,
        ..s
    };
    let (s, commands) = shell(s, ShellAction::Run);
    assert!(commands.is_empty());
    let sh = shell_of(&s, id);
    assert_eq!(
        sh.notice,
        Some(ShellNotice::Refused(ShellRefusal::ReadOnly))
    );
    assert_eq!(sh.output, before.output);
    assert_eq!(sh.result, before.result);
    for mode in [OutputMode::Table, OutputMode::Tree] {
        assert_eq!(sh.availability(mode), Availability::Available);
    }
}

/// An idle shell keeps its value over a lost connection, as it keeps its text output.
#[test]
fn an_idle_connection_reset_keeps_the_result() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = with_result(dir.path());
    let (s, _) = shell(s, ShellAction::SetOutputMode(OutputMode::Tree));
    let before = shell_of(&s, id).result.clone();

    let s = event(s, lost(id));

    let sh = shell_of(&s, id);
    assert_eq!(sh.result, before);
    assert_eq!(sh.mode, OutputMode::Tree);
    assert_eq!(sh.availability(OutputMode::Tree), Availability::Available);
}

/// A run cut short by a lost connection ends unclean, and its late value is ignored.
#[test]
fn a_connection_reset_mid_run_fails_it_and_ignores_its_late_result() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = running(dir.path());
    let s = event(s, lost(id));
    assert_eq!(
        shell_of(&s, id).availability(OutputMode::Table),
        Availability::Unavailable(Reason::RunFailed)
    );
    let s = event(s, result(id, 1));
    assert_eq!(shell_of(&s, id).result.result, None);
}

#[test]
fn a_late_result_of_an_older_run_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = with_result(dir.path());
    let (s, _) = shell(s, ShellAction::Run);
    let pending = shell_of(&s, id).result.clone();
    assert_eq!((pending.run_id, pending.state), (2, ResultState::Pending));

    let s = event(s, result(id, 1));

    assert_eq!(shell_of(&s, id).result, pending);
}
