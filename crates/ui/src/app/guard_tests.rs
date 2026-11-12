//! The query bar's index-miss guard: a Run with a filter or a sort is planned first, a plan that
//! misses an index holds the find back, and every way the check can fail lets the run go on.

use super::explain_actions::VerdictState;
use super::explain_tests::{act, browsing, ns, query_a};
use super::guard_actions::GuardState;
use super::reducer::{Input, update};
use super::state::{AppState, ConnStatus};
use crate::viewmodel::FindAction;
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::guard::{GUARD_MIN_DOCS, GuardFinding, GuardReport, GuardTarget};
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Query;
use uuid::Uuid;

/// Browsing `app.people` with the guard on.
pub(super) fn guarded() -> AppState {
    let mut s = browsing();
    s.explain.guard = true;
    s
}

pub(super) fn query_b() -> Query {
    Query {
        filter: doc! { "age": 2 },
        ..Query::default()
    }
}

pub(super) fn sorted_only() -> Query {
    Query {
        sort: Some(doc! { "age": 1 }),
        ..Query::default()
    }
}

pub(super) fn scan_report(query: Query) -> GuardReport {
    GuardReport {
        findings: vec![GuardFinding {
            ns: ns(),
            method: "find".into(),
            query,
            kind: VerdictKind::CollectionScan,
            reasons: Vec::new(),
            suggestion: None,
        }],
        unchecked: 0,
    }
}

pub(super) fn checked(
    state: AppState,
    request: u64,
    outcome: Result<GuardReport, CoreError>,
) -> (AppState, Vec<Command>) {
    checked_in(state, EMBEDDED_SESSION, request, outcome)
}

pub(super) fn checked_in(
    state: AppState,
    session: Uuid,
    request: u64,
    outcome: Result<GuardReport, CoreError>,
) -> (AppState, Vec<Command>) {
    let event = Event::GuardChecked {
        session,
        request,
        outcome,
    };
    update(state, Input::Event(event))
}

fn request_failed(
    state: AppState,
    context: &'static str,
    request: u64,
    error: CoreError,
) -> (AppState, Vec<Command>) {
    let event = Event::RequestFailed {
        session: EMBEDDED_SESSION,
        context,
        request,
        error,
    };
    update(state, Input::Event(event))
}

/// The commands of a run that goes on: the find, then the badge's explain.
pub(super) fn assert_find_and_explain(cmds: &[Command], query: &Query) {
    match cmds {
        [
            Command::Find { query: q1, .. },
            Command::ExplainFind { query: q2, .. },
        ] => {
            assert_eq!((q1, q2), (query, query));
        }
        other => panic!("{other:?}"),
    }
}

pub(super) fn checking(request: u64, query: Query) -> GuardState {
    GuardState::Checking { request, query }
}

#[test]
fn guard_off_sends_the_find_as_today() {
    let (s, cmds) = act(browsing(), FindAction::Run(query_a()));
    assert_find_and_explain(&cmds, &query_a());
    assert_eq!(*s.guard, GuardState::None);
    assert_eq!(s.guard_request, 0);
}

#[test]
fn guard_on_with_a_filter_sends_only_the_check() {
    let before = guarded();
    let loading = before.loading_page;
    let (s, cmds) = act(before, FindAction::Run(query_a()));
    let expected = Command::GuardCheck {
        session: EMBEDDED_SESSION,
        request: 1,
        targets: vec![GuardTarget {
            ns: ns(),
            method: "find".into(),
            query: query_a(),
        }],
        min_docs: Some(GUARD_MIN_DOCS),
    };
    assert_eq!(format!("{cmds:?}"), format!("{:?}", vec![expected]));
    assert_eq!(*s.guard, checking(1, query_a()));
    assert_eq!(s.guard_request, 1);
    // The find is not sent yet: the page on screen and its loading flag stay as they were.
    assert_eq!(s.loading_page, loading);
    assert_eq!(s.issued_query, Query::default());
    assert_eq!(s.verdict, VerdictState::None);
}

#[test]
fn a_sort_alone_is_checked() {
    let (s, cmds) = act(guarded(), FindAction::Run(sorted_only()));
    assert!(
        matches!(cmds.as_slice(), [Command::GuardCheck { .. }]),
        "{cmds:?}"
    );
    assert_eq!(*s.guard, checking(1, sorted_only()));
}

#[test]
fn no_filter_and_no_sort_sends_the_find() {
    let query = Query {
        limit: 5,
        ..Query::default()
    };
    let (s, cmds) = act(guarded(), FindAction::Run(query.clone()));
    assert_find_and_explain(&cmds, &query);
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn no_collection_selected_sends_nothing_and_does_not_check() {
    let mut state = guarded();
    state.sidebar.selected = None;
    let (s, cmds) = act(state, FindAction::Run(query_a()));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn production_checks_without_a_minimum_size() {
    let mut state = guarded();
    state.production = true;
    let (_, cmds) = act(state, FindAction::Run(query_a()));
    match cmds.as_slice() {
        [Command::GuardCheck { min_docs, .. }] => assert_eq!(*min_docs, None),
        other => panic!("{other:?}"),
    }
}

#[test]
fn other_connections_check_with_the_minimum_size() {
    let (_, cmds) = act(guarded(), FindAction::Run(query_a()));
    match cmds.as_slice() {
        [Command::GuardCheck { min_docs, .. }] => assert_eq!(*min_docs, Some(1000)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_read_only_connection_still_checks() {
    let mut state = guarded();
    state.read_only = true;
    let (s, cmds) = act(state, FindAction::Run(query_a()));
    assert!(
        matches!(cmds.as_slice(), [Command::GuardCheck { .. }]),
        "{cmds:?}"
    );
    assert_eq!(*s.guard, checking(1, query_a()));
}

#[test]
fn a_failed_check_sends_the_find_and_the_badge_explain() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, cmds) = checked(s, 1, Err(CoreError::Database("timed out".into())));
    assert_find_and_explain(&cmds, &query_a());
    assert_eq!(*s.guard, GuardState::None);
    assert_eq!(s.issued_query, query_a());
    assert!(s.loading_page);
    assert_eq!(s.verdict, VerdictState::Pending { query: query_a() });
    assert!(s.banner.is_none(), "{:?}", s.banner);
}

#[test]
fn a_check_with_no_findings_sends_the_find_and_the_badge_explain() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, cmds) = checked(s, 1, Ok(GuardReport::default()));
    assert_find_and_explain(&cmds, &query_a());
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn the_badge_switch_off_still_skips_the_badge_explain_after_a_clean_check() {
    let mut state = guarded();
    state.explain.automatic = false;
    let (s, cmds) = act(state, FindAction::Run(query_a()));
    assert!(
        matches!(cmds.as_slice(), [Command::GuardCheck { .. }]),
        "{cmds:?}"
    );
    let (s, cmds) = checked(s, 1, Ok(GuardReport::default()));
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }]),
        "{cmds:?}"
    );
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn findings_show_the_warning_and_send_nothing() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, cmds) = checked(s, 1, Ok(scan_report(query_a())));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(
        *s.guard,
        GuardState::Warn {
            query: query_a(),
            report: scan_report(query_a()),
        }
    );
    assert!(!s.loading_page);
    assert!(s.banner.is_none());
}

#[test]
fn run_anyway_sends_exactly_one_find_and_does_not_check_again() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, cmds) = act(s, FindAction::GuardRunAnyway);
    match cmds.as_slice() {
        [
            Command::Find {
                ns: n, query: q, ..
            },
        ] => assert_eq!((n, q), (&ns(), &query_a())),
        other => panic!("{other:?}"),
    }
    assert_eq!(*s.guard, GuardState::None);
    assert_eq!(s.guard_request, 1);
    assert_eq!(s.issued_query, query_a());
    let (s, cmds) = act(s, FindAction::GuardRunAnyway);
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn run_anyway_while_checking_sends_nothing() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, cmds) = act(s, FindAction::GuardRunAnyway);
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, checking(1, query_a()));
}

#[test]
fn cancel_drops_the_warning_and_sends_nothing() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, cmds) = act(s, FindAction::GuardCancel);
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, GuardState::None);
    let (s, cmds) = act(s, FindAction::GuardRunAnyway);
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn an_answer_for_an_older_run_changes_nothing() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::Run(query_b()));
    assert_eq!(*s.guard, checking(2, query_b()));
    let (s, cmds) = checked(s, 1, Err(CoreError::Database("late".into())));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, checking(2, query_b()));
    let (s, cmds) = checked(s, 1, Ok(scan_report(query_a())));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, checking(2, query_b()));
    let (s, cmds) = checked(s, 2, Ok(GuardReport::default()));
    assert_find_and_explain(&cmds, &query_b());
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn an_answer_with_nothing_checking_changes_nothing() {
    let (s, cmds) = checked(guarded(), 1, Ok(scan_report(query_a())));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn a_new_run_replaces_the_warning() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = checked(s, 1, Ok(scan_report(query_a())));
    let (s, cmds) = act(s, FindAction::Run(query_b()));
    assert!(
        matches!(cmds.as_slice(), [Command::GuardCheck { request: 2, .. }]),
        "{cmds:?}"
    );
    assert_eq!(*s.guard, checking(2, query_b()));
    let (s, cmds) = act(s, FindAction::Run(Query::default()));
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }, ..]),
        "{cmds:?}"
    );
    assert_eq!(*s.guard, GuardState::None);
}

#[test]
fn a_check_that_could_not_start_lets_the_run_go_on_without_a_banner() {
    for error in [
        CoreError::Database("refused".into()),
        CoreError::Unreachable,
    ] {
        let (s, _) = act(guarded(), FindAction::Run(query_a()));
        let (s, cmds) = request_failed(s, "guard_check", 1, error);
        assert_find_and_explain(&cmds, &query_a());
        assert_eq!(*s.guard, GuardState::None);
        assert!(s.banner.is_none(), "{:?}", s.banner);
        assert_eq!(s.status, ConnStatus::Connected);
    }
}

#[test]
fn an_older_check_that_could_not_start_changes_nothing() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::Run(query_b()));
    let (s, cmds) = request_failed(s, "guard_check", 1, CoreError::Database("late".into()));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, checking(2, query_b()));
    assert!(s.banner.is_none(), "{:?}", s.banner);
}

#[test]
fn another_numbered_failure_with_the_same_number_is_not_the_guards() {
    let (s, _) = act(guarded(), FindAction::Run(query_a()));
    let s = AppState {
        indexes: crate::viewmodel::IndexesState {
            request: 1,
            loading: true,
            ..s.indexes
        },
        ..s
    };
    let (s, cmds) = request_failed(s, "list_indexes", 1, CoreError::Database("x".into()));
    assert!(cmds.is_empty(), "{cmds:?}");
    assert_eq!(*s.guard, checking(1, query_a()));
    assert!(s.banner.is_some(), "the index read fails as before");
}
