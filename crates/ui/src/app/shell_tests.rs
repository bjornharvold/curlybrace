//! The Shell tab inside the app state: tool resolution, guards, the production warning, event
//! routing and what happens to a session's shell when it disconnects, reconnects or closes.
use super::index_write_tests::{browsing, with_flags};
use super::reducer::{Input, update};
use super::shell_actions::ShellAction;
use super::state::{AppState, Banner, ConnStatus, Workspace};
use super::tools_state::ProbeScope;
use super::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::shell::{ShellNotice, ShellState, ShellStatus, ToolProblem};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event, ShellRefusal};
use curlybrace_core::shell::{Outcome, Stream};
use curlybrace_core::test_support::mock_objects::mock_connection;
use curlybrace_core::tools::{ServerVersion, ToolChoice, ToolError, ToolKind, ToolVersion};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const SCRIPT: &str = "db.people.find()";

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

fn event(s: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(ev))
}

fn edit(s: AppState, script: &str) -> AppState {
    let action = ShellAction::Edit {
        script: script.into(),
        selection: None,
    };
    shell(s, action).0
}

fn with_override(s: AppState, id: Uuid, exe: PathBuf) -> AppState {
    let tools = s
        .tools
        .choose_override(id, ToolKind::Mongosh, ToolChoice::Executable(exe));
    AppState { tools, ..s }
}

/// `cfg` open and connected, its shell holding `SCRIPT` and its mongosh override set to `exe`.
fn ready(cfg: ConnectionConfig, exe: &Path) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let s = with_override(s, id, exe.to_path_buf());
    (edit(s, SCRIPT), id)
}

fn plain() -> ConnectionConfig {
    with_flags(false, false)
}

fn shell_of(s: &AppState, id: Uuid) -> &ShellState {
    s.shell.get(id).expect("the session's shell")
}

/// What a shell command says, comparable (`Command` has no `PartialEq`).
#[derive(Debug, PartialEq)]
enum Sent {
    Run(Uuid, u64, PathBuf, String),
    Cancel(Uuid, u64),
    Other,
}

fn sent(commands: &[Command]) -> Vec<Sent> {
    commands
        .iter()
        .map(|c| match c {
            Command::ShellRun {
                session,
                run_id,
                exe,
                script,
            } => Sent::Run(*session, *run_id, exe.clone(), script.clone()),
            Command::ShellCancel { session, run_id } => Sent::Cancel(*session, *run_id),
            _ => Sent::Other,
        })
        .collect()
}

fn expected_run(id: Uuid, run_id: u64, exe: &Path) -> Sent {
    Sent::Run(id, run_id, exe.to_path_buf(), SCRIPT.into())
}

/// The detected version of the mongosh `id` uses, as a finished version check reports it.
fn detected(s: AppState, id: Uuid, version: ToolVersion) -> AppState {
    let (tools, queue) = s.tools.take_queue();
    let request = queue
        .iter()
        .find(|r| r.scope == ProbeScope::Connection(id) && r.kind == ToolKind::Mongosh)
        .expect("a queued check")
        .request;
    let tools = tools.probed(
        ProbeScope::Connection(id),
        ToolKind::Mongosh,
        request,
        Ok(version),
    );
    AppState { tools, ..s }
}

fn with_server(s: AppState, id: Uuid, server: ServerVersion) -> AppState {
    let shell = Box::new((*s.shell).clone().with_server_version(id, server));
    AppState { shell, ..s }
}

fn finished(id: Uuid, run_id: u64, outcome: Outcome) -> Event {
    Event::ShellFinished {
        session: id,
        run_id,
        outcome,
    }
}

fn output(id: Uuid, run_id: u64, text: &str) -> Event {
    Event::ShellOutput {
        session: id,
        run_id,
        stream: Stream::Stdout,
        text: text.into(),
    }
}

/// The active session's connection was lost (its root fails, like a failed connect).
fn lost(s: AppState, id: Uuid) -> AppState {
    event(s, lost_event(id)).0
}

fn lost_event(id: Uuid) -> Event {
    Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    }
}

/// Reconnects the failed session `id` and answers the connect.
fn reconnected(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    let ev = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    event(s, ev).0
}

#[test]
fn run_resolves_mongosh_with_the_connection_override() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = browsing(plain());
    let missing = dir.path().join("global-mongosh");
    let tools = s
        .tools
        .choose_global(ToolKind::Mongosh, ToolChoice::Executable(missing));
    let s = edit(
        with_override(AppState { tools, ..s }, id, exe.clone()),
        SCRIPT,
    );

    let (_, commands) = shell(s, ShellAction::Run);

    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
}

#[test]
fn missing_mongosh_sets_a_tool_problem_and_sends_no_command() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &dir.path().join("nope"));

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    let sh = shell_of(&s, id);
    assert_eq!(
        sh.notice,
        Some(ShellNotice::ToolProblem(ToolProblem::Missing(
            ToolError::Missing
        )))
    );
    assert_eq!(sh.status, ShellStatus::Idle);
}

#[test]
fn a_path_choice_is_searched_in_the_tool_search_path() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = browsing(plain());
    let s = AppState {
        tool_search_path: Some(dir.path().as_os_str().to_owned()),
        ..edit(s, SCRIPT)
    };

    let (_, commands) = shell(s, ShellAction::Run);

    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
}

#[test]
fn too_old_mongosh_for_a_known_server_version_sets_a_tool_problem() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let old = ToolVersion::new(2, 0, 0);
    let server = ServerVersion::new(8, 0);
    let s = with_server(detected(s, id, old), id, server);

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::ToolProblem(ToolProblem::TooOld {
            tool: old,
            server
        }))
    );
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn unknown_server_version_is_not_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(plain(), &exe);
    let s = detected(s, id, ToolVersion::new(1, 0, 0));

    let (s, commands) = shell(s, ShellAction::Run);

    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
    assert_eq!(shell_of(&s, id).notice, None);
}

#[test]
fn a_new_enough_mongosh_for_a_known_server_runs() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(plain(), &exe);
    let s = with_server(
        detected(s, id, ToolVersion::new(2, 3, 0)),
        id,
        ServerVersion::new(8, 0),
    );

    let (_, commands) = shell(s, ShellAction::Run);

    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
}

#[test]
fn production_connection_asks_for_confirmation_on_every_run() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(with_flags(true, false), &exe);

    let (s, commands) = shell(s, ShellAction::Run);
    assert!(commands.is_empty());
    let parked = s.write_warning.as_ref().expect("the warning");
    assert_eq!(parked.kind, WriteKind::ShellRun);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
    let (s, _) = event(s, finished(id, 1, Outcome::Exited(0)));

    let (s, commands) = shell(s, ShellAction::Run);
    assert!(commands.is_empty(), "the second run is asked about too");
    assert_eq!(
        s.write_warning.as_ref().map(|p| p.kind),
        Some(WriteKind::ShellRun)
    );
    let (_, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(sent(&commands), vec![expected_run(id, 2, &exe)]);
}

#[test]
fn run_after_confirmation_emits_the_command_once() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(with_flags(true, false), &exe);
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
    assert!(s.write_warning.is_none());
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    assert!(s.pending.is_none(), "a shell run is no browsing mutation");

    let (s, again) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert!(again.is_empty());
    let (_, while_running) = shell(s, ShellAction::Run);
    assert!(while_running.is_empty());
}

#[test]
fn a_cancelled_production_warning_sends_nothing_and_stays_idle() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Cancel));

    assert!(commands.is_empty());
    assert!(s.write_warning.is_none());
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    assert_eq!(shell_of(&s, id).next_run_id, 1);
}

#[test]
fn a_confirmation_after_a_reconnect_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));
    let (mut s, _) = shell(s, ShellAction::Run);
    let parked = s.write_warning.clone();
    let s2 = reconnected(lost(s, id), id);
    s = AppState {
        write_warning: parked,
        ..s2
    };

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));

    assert!(commands.is_empty());
    assert_eq!(s.banner, Some(Banner::WriteNotSentConnectionChanged));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn read_only_connection_shows_the_refusal_and_sends_no_command() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, true), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert!(
        s.write_warning.is_none(),
        "read-only never reaches the warning"
    );
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Refused(ShellRefusal::ReadOnly))
    );
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn not_connected_run_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let s = lost(s, id);
    assert_eq!(s.status, ConnStatus::Failed);

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Refused(ShellRefusal::NotConnected))
    );
}

#[test]
fn run_emits_one_shell_run_command_with_the_resolved_exe() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(plain(), &exe);

    let (s, commands) = shell(s, ShellAction::Run);

    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    let (_, again) = shell(s, ShellAction::Run);
    assert!(again.is_empty(), "one run at a time");
}

#[test]
fn an_empty_script_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let s = edit(s, "   \n");

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).notice, Some(ShellNotice::EmptyScript));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn an_empty_script_on_production_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));
    let s = edit(s, "");

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert!(s.write_warning.is_none());
    assert_eq!(shell_of(&s, id).notice, Some(ShellNotice::EmptyScript));
}

#[test]
fn cancel_emits_shell_cancel_for_the_current_run() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);
    let (s, _) = event(s, finished(id, 1, Outcome::Exited(0)));
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = shell(s, ShellAction::Cancel);

    assert_eq!(sent(&commands), vec![Sent::Cancel(id, 2)]);
    assert_eq!(
        shell_of(&s, id).status,
        ShellStatus::Running { run_id: 2 },
        "running until the runner confirms"
    );
    let (s, _) = event(s, finished(id, 2, Outcome::Cancelled));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Finished(Outcome::Cancelled))
    );
}

#[test]
fn cancel_when_idle_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = ready(plain(), &fake_mongosh(dir.path()));

    let (_, commands) = shell(s, ShellAction::Cancel);

    assert!(commands.is_empty());
}

#[test]
fn output_and_refusals_reach_the_running_shell() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);

    let (s, _) = event(s, output(id, 1, "hello\n"));
    assert_eq!(
        shell_of(&s, id).line(0).map(|l| l.text.as_str()),
        Some("hello")
    );

    let refused = Event::ShellRefused {
        session: id,
        run_id: 1,
        reason: ShellRefusal::AlreadyRunning,
    };
    let (s, _) = event(s, refused);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Refused(ShellRefusal::AlreadyRunning))
    );
}

#[test]
fn events_for_other_sessions_do_not_touch_this_shell() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let a_cfg = plain();
    let b_cfg = ConnectionConfig {
        name: "other".into(),
        ..mock_connection()
    };
    let (a, b) = (a_cfg.id, b_cfg.id);
    let s = AppState::new(vec![a_cfg, b_cfg]);
    let s = connect_and_answer(s, a);
    let s = connect_and_answer(s, b);
    let s = with_override(with_override(s, a, exe.clone()), b, exe.clone());
    let (s, _) = update(s, Input::Activate(a));
    let (s, _) = shell(edit(s, SCRIPT), ShellAction::Run);
    let (s, _) = update(s, Input::Activate(b));
    let (s, _) = shell(edit(s, SCRIPT), ShellAction::Run);
    let before_a = shell_of(&s, a).clone();

    let (s, _) = event(s, output(b, 1, "only b\n"));
    let (s, _) = event(s, finished(b, 1, Outcome::Exited(0)));
    let (s, _) = event(s, output(Uuid::new_v4(), 1, "nobody\n"));

    assert_eq!(shell_of(&s, a), &before_a);
    assert_eq!(shell_of(&s, b).status, ShellStatus::Idle);
    assert_eq!(shell_of(&s, b).line_count(), 1);
}

fn connect_and_answer(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    let ev = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    event(s, ev).0
}

#[test]
fn disconnect_resets_the_shell_to_idle() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);

    let s = lost(s, id);

    let sh = shell_of(&s, id);
    assert_eq!(sh.status, ShellStatus::Idle);
    assert_eq!(sh.script, SCRIPT);
    assert_eq!(sh.next_run_id, 2);
    let (s, _) = event(s, finished(id, 1, Outcome::Cancelled));
    assert_eq!(shell_of(&s, id).notice, None, "the old run's end is stale");
}

#[test]
fn reconnect_preserves_the_draft_and_the_run_counter() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(plain(), &exe);
    let (s, _) = shell(s, ShellAction::Run);

    let s = reconnected(lost(s, id), id);

    let sh = shell_of(&s, id);
    assert_eq!(sh.status, ShellStatus::Idle);
    assert_eq!(sh.script, SCRIPT);
    assert_eq!(sh.next_run_id, 2);
    let (s, commands) = shell(s, ShellAction::Run);
    assert_eq!(sent(&commands), vec![expected_run(id, 2, &exe)]);
    let (s, _) = event(s, finished(id, 1, Outcome::Cancelled));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 2 });
}

#[test]
fn closing_a_session_drops_its_shell_state() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);
    assert!(s.shell.get(id).is_some());

    let (s, _) = update(s, Input::Disconnect(id));

    assert!(s.shell.get(id).is_none());
}

#[test]
fn a_reopened_session_never_reuses_a_closed_shells_run_ids() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(plain(), &exe);
    let (s, _) = shell(s, ShellAction::Run);
    let (s, _) = update(s, Input::Disconnect(id));

    let s = edit(connect_and_answer(s, id), SCRIPT);
    let (s, commands) = shell(s, ShellAction::Run);

    assert_eq!(sent(&commands), vec![expected_run(id, 2, &exe)]);
    let (s, _) = event(s, finished(id, 1, Outcome::Cancelled));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 2 });
}

#[test]
fn open_requires_a_connected_session() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::Open);
    assert!(commands.is_empty());
    assert_eq!(s.workspace, Workspace::Shell);

    let (s, _) = update(s, Input::Activate(id));
    assert_eq!(s.workspace, Workspace::Browse, "browse stays reachable");

    let s = lost(s, id);
    let (s, _) = shell(s, ShellAction::Open);
    assert_eq!(s.workspace, Workspace::Browse);

    let (none, _) = shell(AppState::default(), ShellAction::Open);
    assert_eq!(none.workspace, Workspace::Browse);
}

#[test]
fn edit_and_format_update_the_active_sessions_shell() {
    let (s, id) = browsing(plain());
    let s = edit(s, "{\na: 1\n}");

    let (s, commands) = shell(s, ShellAction::Format);

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).script, "{\n  a: 1\n}");
    let (none, _) = shell(AppState::default(), ShellAction::Format);
    assert!(none.shell.get(Uuid::nil()).is_none());
}

#[test]
fn a_lost_connection_mid_run_cancels_the_run_and_goes_idle() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(plain(), &exe);
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = event(s, lost_event(id));

    assert_eq!(sent(&commands), vec![Sent::Cancel(id, 1)]);
    let sh = shell_of(&s, id);
    assert_eq!(sh.status, ShellStatus::Idle);
    assert_eq!(sh.script, SCRIPT);
    assert_eq!(sh.next_run_id, 2);

    let (s, _) = event(s, output(id, 1, "late\n"));
    let (s, _) = event(s, finished(id, 1, Outcome::Cancelled));
    assert_eq!(shell_of(&s, id).notice, None, "the old run's end is stale");
    assert_eq!(shell_of(&s, id).line_count(), 0);
    let s = reconnected(s, id);
    let (s, commands) = shell(s, ShellAction::Run);
    assert_eq!(sent(&commands), vec![expected_run(id, 2, &exe)]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 2 });
}

#[test]
fn a_lost_connection_with_an_idle_shell_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);
    let (s, _) = event(s, finished(id, 1, Outcome::Exited(0)));

    let (s, commands) = event(s, lost_event(id));

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn a_lost_background_session_cancels_its_own_run_only() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let a_cfg = plain();
    let b_cfg = ConnectionConfig {
        name: "other".into(),
        ..mock_connection()
    };
    let (a, b) = (a_cfg.id, b_cfg.id);
    let s = AppState::new(vec![a_cfg, b_cfg]);
    let s = connect_and_answer(connect_and_answer(s, a), b);
    let s = with_override(with_override(s, a, exe.clone()), b, exe);
    let (s, _) = update(s, Input::Activate(a));
    let (s, _) = shell(edit(s, SCRIPT), ShellAction::Run);
    let (s, _) = update(s, Input::Activate(b));
    let (s, _) = shell(edit(s, SCRIPT), ShellAction::Run);

    let (s, commands) = event(s, lost_event(a));

    assert_eq!(sent(&commands), vec![Sent::Cancel(a, 1)]);
    assert_eq!(shell_of(&s, a).status, ShellStatus::Idle);
    assert_eq!(shell_of(&s, b).status, ShellStatus::Running { run_id: 1 });
}

#[test]
fn closing_a_running_session_cancels_its_run_before_disconnecting() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = update(s, Input::Disconnect(id));

    assert_eq!(sent(&commands), vec![Sent::Cancel(id, 1), Sent::Other]);
    assert!(matches!(commands[1], Command::Disconnect { .. }));
    assert!(s.shell.get(id).is_none());
}

#[test]
fn a_confirmed_run_sends_the_script_as_it_was_when_run_was_clicked() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let (s, id) = ready(with_flags(true, false), &exe);
    let (s, _) = shell(s, ShellAction::Run);
    let s = edit(s, "");

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));

    assert_eq!(sent(&commands), vec![expected_run(id, 1, &exe)]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    assert_eq!(shell_of(&s, id).notice, None);
    let (_, again) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert!(again.is_empty());
}

#[test]
fn a_second_run_click_while_the_warning_is_up_parks_nothing_more() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);
    let s = edit(s, "db.other.find()");

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    let parked = s.write_warning.as_ref().expect("the first warning");
    assert!(
        matches!(&parked.command, Command::ShellRun { script, .. } if script == SCRIPT),
        "the first parked run is kept, not replaced"
    );
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}
