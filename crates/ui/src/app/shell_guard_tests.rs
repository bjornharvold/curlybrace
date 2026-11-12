//! The Shell tab's index-miss guard inside the app: a Run whose script holds a readable query is
//! planned first, a plan that would scan shows the `IndexMiss` notice, and every other answer
//! lets the run go on exactly as without a guard.
use crate::app::index_write_tests::{browsing, with_flags};
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::state::AppState;
use crate::viewmodel::shell::{ShellNotice, ShellState, ShellStatus};
use crate::viewmodel::shell_queries::read_calls;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::guard::{GUARD_MIN_DOCS, GuardFinding, GuardReport, GuardTarget};
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::shell::Outcome;
use curlybrace_core::store::Namespace;
use curlybrace_core::tools::{ToolChoice, ToolKind};
use std::ops::Range;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// One readable call that is worth checking.
pub(super) const QUERY: &str = "db.people.find({ age: 1 })";

/// A runnable stand-in for `mongosh` in `dir` (never run by these tests).
pub(super) fn fake_mongosh(dir: &Path) -> PathBuf {
    let path = dir.join("mongosh");
    std::fs::write(&path, "#!/bin/sh\necho 2.3.0\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

pub(super) fn shell(s: AppState, action: ShellAction) -> (AppState, Vec<Command>) {
    update(s, Input::Shell(action))
}

pub(super) fn event(s: AppState, ev: Event) -> (AppState, Vec<Command>) {
    update(s, Input::Event(ev))
}

pub(super) fn edit(s: AppState, script: &str, selection: Option<Range<usize>>) -> AppState {
    let action = ShellAction::Edit {
        script: script.into(),
        selection,
    };
    shell(s, action).0
}

/// `cfg` open and connected with the guard on, its shell holding `script` and its mongosh
/// override set to `exe`.
pub(super) fn ready(cfg: ConnectionConfig, exe: &Path, script: &str) -> (AppState, Uuid) {
    let (s, id) = browsing(cfg);
    let tools = s.tools.choose_override(
        id,
        ToolKind::Mongosh,
        ToolChoice::Executable(exe.to_path_buf()),
    );
    let mut s = AppState { tools, ..s };
    s.explain.guard = true;
    (edit(s, script, None), id)
}

pub(super) fn plain() -> ConnectionConfig {
    with_flags(false, false)
}

pub(super) fn production() -> ConnectionConfig {
    with_flags(true, false)
}

pub(super) fn shell_of(s: &AppState, id: Uuid) -> &ShellState {
    s.shell.get(id).expect("the session's shell")
}

/// The scripts of the shell runs among `commands`.
pub(super) fn runs(commands: &[Command]) -> Vec<(u64, String)> {
    commands
        .iter()
        .filter_map(|c| match c {
            Command::ShellRun { run_id, script, .. } => Some((*run_id, script.clone())),
            _ => None,
        })
        .collect()
}

/// The single guard check `commands` holds: its session, request, targets and size floor.
pub(super) fn the_check(commands: &[Command]) -> (Uuid, u64, Vec<GuardTarget>, Option<u64>) {
    match commands {
        [
            Command::GuardCheck {
                session,
                request,
                targets,
                min_docs,
            },
        ] => (*session, *request, targets.clone(), *min_docs),
        other => panic!("expected one GuardCheck, got {other:?}"),
    }
}

pub(super) fn answered(
    s: AppState,
    id: Uuid,
    request: u64,
    outcome: Result<GuardReport, CoreError>,
) -> (AppState, Vec<Command>) {
    let ev = Event::GuardChecked {
        session: id,
        request,
        outcome,
    };
    event(s, ev)
}

pub(super) fn clean() -> GuardReport {
    GuardReport::default()
}

/// A report that `people` in `db` would be scanned, with `unchecked` explains it could not do.
pub(super) fn scan(db: &str, unchecked: usize) -> GuardReport {
    let target = targets_of(QUERY, db).remove(0);
    GuardReport {
        findings: vec![GuardFinding {
            ns: target.ns,
            method: target.method,
            query: target.query,
            kind: VerdictKind::CollectionScan,
            reasons: Vec::new(),
            suggestion: None,
        }],
        unchecked,
    }
}

/// The targets the reader makes of `text` when it starts in `db`.
pub(super) fn targets_of(text: &str, db: &str) -> Vec<GuardTarget> {
    read_calls(text, db)
        .calls
        .into_iter()
        .map(|call| GuardTarget {
            ns: Namespace::new(call.db, call.collection),
            method: call.method.to_string(),
            query: call.query,
        })
        .collect()
}

pub(super) fn checking(request: u64) -> ShellStatus {
    ShellStatus::Checking { request }
}

/// The shell of `id` stopped at an `IndexMiss` notice after a Run of `script` (whole).
pub(super) fn missed(cfg: ConnectionConfig, exe: &Path) -> (AppState, Uuid) {
    let (s, id) = ready(cfg, exe, QUERY);
    let (s, commands) = shell(s, ShellAction::Run);
    let (_, request, _, _) = the_check(&commands);
    let (s, commands) = answered(s, id, request, Ok(scan("test", 0)));
    assert!(commands.is_empty(), "{commands:?}");
    (s, id)
}

#[test]
fn guard_off_runs_as_today() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, id) = ready(plain(), &fake_mongosh(dir.path()), QUERY);
    s.explain.guard = false;

    let (s, commands) = shell(s, ShellAction::Run);

    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    assert_eq!(s.guard_request, 0);
}

#[test]
fn a_script_with_no_readable_call_runs_as_today_even_with_unchecked_calls() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    for script in [
        "print(1)",
        "db.people.find(filter)",
        "db[name].find({ a: 1 })",
    ] {
        let (s, id) = ready(plain(), &exe, script);

        let (s, commands) = shell(s, ShellAction::Run);

        assert_eq!(runs(&commands), vec![(1, script.to_string())], "{script}");
        assert_eq!(commands.len(), 1, "{commands:?}");
        assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
        assert_eq!(s.guard_request, 0);
    }
}

#[test]
fn calls_with_neither_a_filter_nor_a_sort_are_not_worth_a_check() {
    let dir = tempfile::tempdir().unwrap();
    let script = "db.people.find()\ndb.people.deleteMany({})";
    let (s, _) = ready(plain(), &fake_mongosh(dir.path()), script);

    let (s, commands) = shell(s, ShellAction::Run);

    assert_eq!(runs(&commands), vec![(1, script.to_string())]);
    assert_eq!(s.guard_request, 0);
}

#[test]
fn a_readable_call_sends_only_the_check_and_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), QUERY);

    let (s, commands) = shell(s, ShellAction::Run);

    let (session, request, targets, min_docs) = the_check(&commands);
    assert_eq!((session, request), (id, 1));
    assert_eq!(targets, targets_of(QUERY, "test"));
    assert_eq!(targets[0].ns, Namespace::new("test", "people"));
    assert_eq!(min_docs, Some(GUARD_MIN_DOCS));
    assert_eq!(s.guard_request, 1);
    let sh = shell_of(&s, id);
    assert_eq!(sh.status, checking(1));
    assert!(sh.is_busy() && !sh.is_running() && !sh.can_run());
    assert_eq!(sh.notice, None);
}

#[test]
fn the_database_starts_as_the_connection_uri_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig {
        uri: "mongodb://localhost:27017/shop?retryWrites=true".into(),
        ..plain()
    };
    let script = "db.orders.find({ a: 1 })\nuse other\ndb.items.find({ b: 1 })";
    let (s, _) = ready(cfg, &fake_mongosh(dir.path()), script);

    let (_, commands) = shell(s, ShellAction::Run);

    let (_, _, targets, _) = the_check(&commands);
    let names: Vec<Namespace> = targets.into_iter().map(|t| t.ns).collect();
    assert_eq!(
        names,
        vec![
            Namespace::new("shop", "orders"),
            Namespace::new("other", "items")
        ]
    );
}

#[test]
fn the_selection_is_what_is_read_and_from_the_same_database() {
    let dir = tempfile::tempdir().unwrap();
    let script = "use other\ndb.a.find({ x: 1 })\ndb.b.find({ y: 1 })";
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), script);
    let second = script.find("db.b").unwrap()..script.len();
    let s = edit(s, script, Some(second.clone()));

    let (s, commands) = shell(s, ShellAction::Run);

    let (_, request, targets, _) = the_check(&commands);
    assert_eq!(targets, targets_of("db.b.find({ y: 1 })", "test"));
    let (_, commands) = answered(s, id, request, Ok(clean()));
    assert_eq!(runs(&commands), vec![(1, script[second].to_string())]);
}

#[test]
fn a_clean_report_runs_the_checked_text() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), QUERY);
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = answered(s, id, 1, Ok(clean()));

    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(commands.len(), 1);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
}

#[test]
fn a_failed_check_runs_as_if_there_were_no_guard() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), QUERY);
    let (s, _) = shell(s, ShellAction::Run);

    let (s, commands) = answered(s, id, 1, Err(CoreError::Database("timed out".into())));

    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
    assert!(s.banner.is_none());
}

#[test]
fn a_check_that_could_not_start_runs_as_if_there_were_no_guard() {
    let dir = tempfile::tempdir().unwrap();
    let exe = fake_mongosh(dir.path());
    for error in [CoreError::Database("x".into()), CoreError::Unreachable] {
        let (s, id) = ready(plain(), &exe, QUERY);
        let (s, _) = shell(s, ShellAction::Run);
        let ev = Event::RequestFailed {
            session: id,
            context: "guard_check",
            request: 1,
            error,
        };

        let (s, commands) = event(s, ev);

        assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
        assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
        assert!(s.banner.is_none());
    }
}

#[test]
fn findings_show_the_notice_and_run_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = missed(plain(), &fake_mongosh(dir.path()));

    let sh = shell_of(&s, id);
    assert_eq!(sh.status, ShellStatus::Idle);
    assert_eq!(
        sh.notice,
        Some(ShellNotice::IndexMiss {
            report: scan("test", 0),
            unchecked: 0,
            ran: None,
        })
    );
    assert!(sh.can_run());
}

#[test]
fn run_anyway_runs_the_checked_text_once_without_a_new_check() {
    let dir = tempfile::tempdir().unwrap();
    let script = "print(0)\ndb.people.find({ age: 1 })";
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), script);
    let checked = 9..script.len();
    let s = edit(s, script, Some(checked.clone()));
    let (s, _) = shell(s, ShellAction::Run);
    let (s, _) = answered(s, id, 1, Ok(scan("test", 0)));
    // The caret moves (no text change): the notice and what it checked stay.
    let s = edit(s, script, Some(0..0));
    assert!(matches!(
        &shell_of(&s, id).notice,
        Some(ShellNotice::IndexMiss { ran: Some(r), .. }) if *r == checked
    ));

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert_eq!(runs(&commands), vec![(1, script[checked].to_string())]);
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(s.guard_request, 1);
    assert_eq!(shell_of(&s, id).notice, None);
    let finished = Event::ShellFinished {
        session: id,
        run_id: 1,
        outcome: Outcome::Exited(0),
    };
    let (s, _) = event(s, finished);
    let s = edit(s, script, None);
    let (_, commands) = shell(s, ShellAction::Run);
    let (_, request, _, _) = the_check(&commands);
    assert_eq!(request, 2, "the next Run is checked again");
}

#[test]
fn past_the_cap_calls_are_not_sent_and_count_as_unchecked() {
    let dir = tempfile::tempdir().unwrap();
    let mut script: String = (0..25)
        .map(|i| format!("db.c{i}.find({{ a: {i} }})\n"))
        .collect();
    script.push_str("db.x.find(filter)\n");
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), &script);

    let (s, commands) = shell(s, ShellAction::Run);

    let (_, request, targets, _) = the_check(&commands);
    assert_eq!(targets.len(), 20);
    assert_eq!(targets, targets_of(&script, "test")[..20].to_vec());
    let (s, _) = answered(s, id, request, Ok(scan("test", 2)));
    let Some(ShellNotice::IndexMiss { unchecked, .. }) = &shell_of(&s, id).notice else {
        panic!("{:?}", shell_of(&s, id).notice);
    };
    // 1 the reader could not read, 5 past the cap, 2 the check could not judge.
    assert_eq!(*unchecked, 8);
}
