//! The Shell tab's guard among a Run's other checks: the read-only refusal comes first, Run
//! anyway after a syntax notice is still checked, and on production the confirmation follows
//! the check (after a clean answer and after Run anyway on an index notice).
use super::shell_guard_tests::{
    QUERY, answered, clean, fake_mongosh, missed, plain, production, ready, runs, shell, shell_of,
    the_check,
};
use crate::app::index_write_tests::with_flags;
use crate::app::reducer::{Input, update};
use crate::app::shell_actions::ShellAction;
use crate::app::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::shell::{ShellNotice, ShellStatus};
use curlybrace_core::runtime::ShellRefusal;

#[test]
fn run_anyway_after_a_syntax_notice_is_still_guarded() {
    let dir = tempfile::tempdir().unwrap();
    let script = "db.people.find({ age: 1 })\nfoo(";
    let (s, id) = ready(plain(), &fake_mongosh(dir.path()), script);
    let (s, commands) = shell(s, ShellAction::Run);
    assert!(commands.is_empty());
    assert!(matches!(
        shell_of(&s, id).notice,
        Some(ShellNotice::SyntaxProblem { .. })
    ));

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    let (_, request, _, _) = the_check(&commands);
    let (_, commands) = answered(s, id, request, Ok(clean()));
    assert_eq!(runs(&commands), vec![(1, script.to_string())]);
}

#[test]
fn on_production_a_clean_check_parks_the_run_in_the_write_warning() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(production(), &fake_mongosh(dir.path()), QUERY);
    let (s, commands) = shell(s, ShellAction::Run);
    let (_, _, _, min_docs) = the_check(&commands);
    assert_eq!(min_docs, None, "production warns whatever the size");

    let (s, commands) = answered(s, id, 1, Ok(clean()));

    assert!(commands.is_empty(), "{commands:?}");
    let parked = s.write_warning.as_ref().expect("the warning");
    assert_eq!(parked.kind, WriteKind::ShellRun);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Idle);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
}

#[test]
fn on_production_run_anyway_after_findings_still_asks() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = missed(production(), &fake_mongosh(dir.path()));

    let (s, commands) = shell(s, ShellAction::RunAnyway);

    assert!(commands.is_empty(), "{commands:?}");
    assert!(s.write_warning.is_some());
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(runs(&commands), vec![(1, QUERY.to_string())]);
    assert_eq!(shell_of(&s, id).status, ShellStatus::Running { run_id: 1 });
}

#[test]
fn a_read_only_connection_is_refused_before_any_check() {
    let dir = tempfile::tempdir().unwrap();
    let (s, id) = ready(with_flags(false, true), &fake_mongosh(dir.path()), QUERY);

    let (s, commands) = shell(s, ShellAction::Run);

    assert!(commands.is_empty(), "{commands:?}");
    assert_eq!(s.guard_request, 0);
    assert_eq!(
        shell_of(&s, id).notice,
        Some(ShellNotice::Refused(ShellRefusal::ReadOnly))
    );
}
