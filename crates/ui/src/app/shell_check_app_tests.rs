//! The pre-run syntax check inside the app: a visibly broken script is not run, `RunAnyway`
//! passes every other guard of a Run, and `GoToProblem` selects the problem.
use crate::app::index_write_tests::{browsing, with_flags};
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::state::{AppState, ConnStatus};
use crate::app::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::shell::{ShellNotice, ShellState, ShellStatus, ToolProblem};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event, ShellRefusal};
use curlybrace_core::shell::Outcome;
use curlybrace_core::tools::{ServerVersion, ToolChoice, ToolError, ToolKind, ToolVersion};
use std::ops::Range;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const BROKEN: &str = "db.a.find()\ndb.b.find(";

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

fn edit(s: AppState, script: &str, selection: Option<Range<usize>>) -> AppState {
    let action = ShellAction::Edit {
        script: script.into(),
        selection,
    };
    shell(s, action).0
}

/// `cfg` open and connected, its shell holding `BROKEN` and its mongosh override set to `exe`.
fn ready(cfg: ConnectionConfig, exe: &Path) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let tools = s.tools.choose_override(
        id,
        ToolKind::Mongosh,
        ToolChoice::Executable(exe.to_path_buf()),
    );
    (edit(AppState { tools, ..s }, BROKEN, None), id)
}

fn plain() -> ConnectionConfig {
    with_flags(false, false)
}

fn shell_of(s: &AppState, id: Uuid) -> &ShellState {
    s.shell.get(id).expect("the session's shell")
}

/// The scripts of the shell runs among `commands`.
fn runs(commands: &[Command]) -> Vec<(u64, String)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::ShellRun { run_id, script, .. } => Some((*run_id, script.clone())),
            _ => None,
        })
        .collect()
}

fn has_syntax_problem(s: &AppState, id: Uuid) -> bool {
    matches!(
        shell_of(s, id).notice,
        Some(ShellNotice::SyntaxProblem { .. })
    )
}

#[test]
fn run_of_a_broken_script_sends_nothing_and_shows_the_problem() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty(), "{commands:?}");
    assert!(has_syntax_problem(&s, id));
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
}

#[test]
fn a_broken_script_on_production_asks_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert!(s.write_warning.is_none());
    assert!(has_syntax_problem(&s, id));
}

#[test]
fn run_anyway_runs_the_broken_script() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert_eq!(runs(&commands), vec![(1, BROKEN.to_string())]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    assert_eq!(shell_of(&s, id).notice, None);
}

#[test]
fn run_anyway_on_production_asks_and_the_confirmed_run_is_not_checked_again() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::RunAnyway);
    assert!(commands.is_empty());
    let parked = s.write_warning.as_ref().expect("the warning");
    assert_eq!(parked.kind, WriteKind::ShellRun);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(runs(&commands), vec![(1, BROKEN.to_string())]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
}

#[test]
fn a_confirmed_run_sends_the_parked_text_even_if_the_editor_broke_since() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, false), &fake_mongosh(dir.path()));
    let s = edit(s, "db.a.find()", None);
    let (s, _) = shell(s, ShellAction::Run);
    let s = edit(s, BROKEN, None);

    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));

    assert_eq!(runs(&commands), vec![(1, "db.a.find()".to_string())]);
    assert!(!has_syntax_problem(&s, id));
}

#[test]
fn run_anyway_while_the_warning_is_up_parks_nothing_more() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = ready(with_flags(true, false), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::RunAnyway);
    let s = edit(s, "db.other.find(", None);

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert!(commands.is_empty());
    let parked = s.write_warning.as_ref().expect("the first warning");
    assert!(matches!(&parked.command, Command::ShellRun { script, .. } if script == BROKEN));
}

#[test]
fn run_anyway_is_refused_on_a_read_only_connection() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(true, true), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert!(commands.is_empty());
    assert!(s.write_warning.is_none());
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Refused(ShellRefusal::ReadOnly))
    );
}

#[test]
fn run_anyway_is_refused_when_not_connected() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let failed = Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    };
    let s = update(s, Input::Event(failed)).0;
    assert_eq!(s.status, ConnStatus::Failed);

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert!(commands.is_empty());
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Refused(ShellRefusal::NotConnected))
    );
}

#[test]
fn run_anyway_is_refused_without_a_usable_mongosh() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &dir.path().join("nope"));

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert!(commands.is_empty());
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::ToolProblem(ToolProblem::Missing(
            ToolError::Missing
        )))
    );
}

#[test]
fn run_anyway_is_refused_with_a_too_old_mongosh() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (tools, queue) = s.tools.take_queue();
    let request = queue
        .iter()
        .find(|r| r.kind == ToolKind::Mongosh)
        .expect("a queued check")
        .request;
    let old = ToolVersion::new(2, 0, 0);
    let tools = tools.probed(
        crate::app::tools_state::ProbeScope::Connection(id),
        ToolKind::Mongosh,
        request,
        Ok(old),
    );
    let server = ServerVersion::new(8, 0);
    let shells = Box::new((*s.shell).clone().with_server_version(id, server));
    let s = AppState {
        tools,
        shell: shells,
        ..s
    };

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert!(commands.is_empty());
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::ToolProblem(ToolProblem::TooOld {
            tool: old,
            server
        }))
    );
}

#[test]
fn a_good_selection_inside_a_broken_script_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = ready(plain(), &fake_mongosh(dir.path()));
    let s = edit(s, BROKEN, Some(0..11));

    let (_, commands) = shell(s, ShellAction::Run);

    assert_eq!(runs(&commands), vec![(1, "db.a.find()".to_string())]);
}

#[test]
fn go_to_problem_puts_the_caret_at_the_problem_and_sends_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = shell(s, ShellAction::GoToProblem);

    assert!(commands.is_empty());
    assert_eq!(shell_of(&s, id).selection, Some(21..21));
    assert!(has_syntax_problem(&s, id), "the notice stays");
}

#[test]
fn typing_or_formatting_clears_the_problem_but_moving_the_caret_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    let stopped = || {
        let (s, id) = ready(plain(), &exe);
        (shell(s, ShellAction::Run).0, id)
    };

    let (s, id) = stopped();
    let moved = edit(s, BROKEN, Some(0..2));
    assert!(has_syntax_problem(&moved, id));
    let (s, id) = stopped();
    let typed = edit(s, "db.a.find()\ndb.b.find()", None);
    assert_eq!(shell_of(&typed, id).notice, None);
    let (s, id) = stopped();
    let s = edit(s, "if (a) {\nb();", None);
    let (s, _) = shell(s, ShellAction::Run);
    assert!(has_syntax_problem(&s, id));
    let (formatted, _) = shell(s, ShellAction::Format);
    assert_eq!(shell_of(&formatted, id).notice, None);
}

#[test]
fn after_a_run_anyway_the_next_run_is_checked_again() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::RunAnyway);
    let finished = Event::ShellFinished {
        session: id,
        run_id: 1,
        outcome: Outcome::Incomplete,
    };
    let s = update(s, Input::Event(finished)).0;

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty());
    assert!(has_syntax_problem(&s, id));
}

#[test]
fn run_anyway_after_go_to_problem_sends_the_whole_original_script() {
    let dir = tempfile::tempdir().unwrap();
    let (s, _) = ready(plain(), &fake_mongosh(dir.path()));
    let (s, _) = shell(s, ShellAction::Run);
    let (s, _) = shell(s, ShellAction::GoToProblem);

    let (_, commands) = shell(s, ShellAction::RunAnyway);

    assert_eq!(runs(&commands), vec![(1, BROKEN.to_string())]);
}
