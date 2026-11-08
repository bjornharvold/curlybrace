//! The verdict of the current run: issued with `Run`, kept across paging, dropped when stale.

use super::explain_actions::{VerdictState, run, same_run, shown};
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::{FindAction, SidebarAction};
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{Verdict, VerdictKind};
use curlybrace_core::runtime::EMBEDDED_SESSION;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

pub(super) fn ns() -> Namespace {
    Namespace::new("app", "people")
}

fn other_ns() -> Namespace {
    Namespace::new("app", "pets")
}

pub(super) fn browsing() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into(), "pets".into()])
        .toggled("app")
        .selecting(ns());
    s
}

pub(super) fn query_a() -> Query {
    Query {
        filter: doc! { "age": 1 },
        ..Query::default()
    }
}

fn query_b() -> Query {
    Query {
        filter: doc! { "age": 2 },
        ..Query::default()
    }
}

pub(super) fn verdict() -> Verdict {
    Verdict {
        kind: VerdictKind::CollectionScan,
        index_name: None,
        keys_examined: 0,
        docs_examined: 10,
        returned: 1,
        plan_only: false,
        reasons: Vec::new(),
        suggestion: None,
        namespace: None,
    }
}

pub(super) fn act(state: AppState, action: FindAction) -> (AppState, Vec<Command>) {
    update(state, Input::Find(action))
}

pub(super) fn explained(
    state: AppState,
    ns: Namespace,
    query: Query,
    outcome: Result<Verdict, CoreError>,
) -> AppState {
    let event = Event::Explain {
        session: EMBEDDED_SESSION,
        ns,
        query,
        outcome,
    };
    update(state, Input::Event(event)).0
}

/// A page showing, so that `Next` is allowed.
fn with_next_page(state: AppState, query: Query) -> AppState {
    AppState {
        page: Some(Page {
            docs: Vec::new(),
            skip: query.skip,
            has_more: true,
        }),
        page_query: query,
        ..state
    }
}

fn pending(query: Query) -> VerdictState {
    VerdictState::Pending { query }
}

#[test]
fn run_issues_find_and_explain_and_sets_pending() {
    let (s, cmds) = act(browsing(), FindAction::Run(query_a()));
    assert_eq!(s.verdict, pending(query_a()));
    match cmds.as_slice() {
        [
            Command::Find {
                ns: n1,
                query: q1,
                session: s1,
            },
            Command::ExplainFind {
                ns: n2,
                query: q2,
                session: s2,
            },
        ] => {
            assert_eq!((n1, q1, s1), (&ns(), &query_a(), &EMBEDDED_SESSION));
            assert_eq!((n2, q2, s2), (&ns(), &query_a(), &EMBEDDED_SESSION));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn paging_keeps_the_verdict_and_issues_no_explain() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    let s = with_next_page(s, query_a());
    let (s, cmds) = act(s, FindAction::Next);
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }]),
        "{cmds:?}"
    );
    assert!(matches!(s.verdict, VerdictState::Ready { .. }));
}

#[test]
fn clear_and_open_reset_the_verdict_without_explaining() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let (cleared, cmds) = act(s, FindAction::Clear);
    assert_eq!(cleared.verdict, VerdictState::None);
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }]),
        "{cmds:?}"
    );

    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let sidebar = SidebarAction::Open(other_ns());
    let (opened, cmds) = update(s, Input::Sidebar(sidebar));
    assert_eq!(opened.verdict, VerdictState::None);
    assert!(
        cmds.iter()
            .all(|c| !matches!(c, Command::ExplainFind { .. })),
        "{cmds:?}"
    );
}

#[test]
fn explain_for_the_current_run_is_stored() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert_eq!(
        s.verdict,
        VerdictState::Ready {
            query: query_a(),
            verdict: verdict()
        }
    );
}

#[test]
fn explain_for_an_older_run_is_dropped() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::Run(query_b()));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert_eq!(s.verdict, pending(query_b()));
}

#[test]
fn explain_for_another_collection_is_dropped() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let s = explained(s, other_ns(), query_a(), Ok(verdict()));
    assert_eq!(s.verdict, pending(query_a()));
}

#[test]
fn explain_error_becomes_unavailable_without_a_banner() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let loading = s.loading_page;
    let s = explained(
        s,
        ns(),
        query_a(),
        Err(CoreError::Database("timed out".into())),
    );
    assert_eq!(
        s.verdict,
        VerdictState::Unavailable {
            query: query_a(),
            error: CoreError::Database("timed out".into())
        }
    );
    assert!(s.banner.is_none() && s.failure.is_none());
    assert_eq!(s.loading_page, loading);
}

#[test]
fn explain_survives_paging_then_event_arrives() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let s = with_next_page(s, query_a());
    let (s, _) = act(s, FindAction::Next);
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert!(matches!(s.verdict, VerdictState::Ready { .. }));
}

#[test]
fn mutation_refresh_keeps_the_verdict() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    let event = Event::Mutated {
        session: EMBEDDED_SESSION,
        ns: ns(),
    };
    let (s, cmds) = update(s, Input::Event(event));
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }]),
        "{cmds:?}"
    );
    assert!(matches!(s.verdict, VerdictState::Ready { .. }));
}

fn ready_for(query: Query) -> VerdictState {
    VerdictState::Ready {
        query,
        verdict: verdict(),
    }
}

#[test]
fn run_with_explain_disabled_clears_the_previous_verdict_and_issues_only_find() {
    let s = AppState {
        verdict: ready_for(query_a()),
        ..browsing()
    };
    let (s, cmds) = run(s, query_b(), false);
    assert_eq!(s.verdict, VerdictState::None);
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }]),
        "{cmds:?}"
    );
}

#[test]
fn run_without_a_selected_collection_is_a_no_op_that_keeps_the_verdict() {
    let s = AppState {
        verdict: ready_for(query_a()),
        ..AppState::new(vec![])
    };
    let (s, cmds) = run(s, query_b(), false);
    assert_eq!(s.verdict, ready_for(query_a()));
    assert!(cmds.is_empty());
}

#[test]
fn explain_arriving_after_clear_is_dropped() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::Clear);
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert_eq!(s.verdict, VerdictState::None);
}

#[test]
fn explain_arriving_after_open_is_dropped() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(other_ns())));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert_eq!(s.verdict, VerdictState::None);
}

#[test]
fn same_run_ignores_skip() {
    let paged = Query {
        skip: 50,
        ..query_a()
    };
    assert!(same_run(&query_a(), &paged));
    assert!(!same_run(&query_a(), &query_b()));
    let limited = Query {
        limit: 7,
        ..query_a()
    };
    assert!(!same_run(&query_a(), &limited));
}

#[test]
fn explain_with_a_different_skip_is_still_accepted() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let paged = Query {
        skip: 50,
        ..query_a()
    };
    let s = explained(s, ns(), paged, Ok(verdict()));
    assert!(matches!(s.verdict, VerdictState::Ready { .. }));
}

#[test]
fn only_the_variant_name_of_an_error_is_logged() {
    use super::explain_actions::error_variant;
    assert_eq!(error_variant(&CoreError::Unauthorized), "Unauthorized");
    let leaky = CoreError::Database("status = \"secret-value\"".into());
    assert_eq!(error_variant(&leaky), "Database");
}

// ---- the verdict shown beside the page (a verdict never sits over another run's results) ----

fn page_of(state: AppState, query: Query) -> AppState {
    AppState {
        page: Some(Page {
            docs: Vec::new(),
            skip: 0,
            has_more: false,
        }),
        page_query: query,
        loading_page: false,
        ..state
    }
}

#[test]
fn a_ready_verdict_of_the_run_on_screen_is_shown() {
    let s = page_of(browsing(), query_a());
    let s = AppState {
        verdict: ready_for(query_a()),
        ..s
    };
    assert_eq!(shown(&s), ready_for(query_a()));
}

#[test]
fn a_ready_verdict_of_a_run_whose_page_has_not_arrived_is_not_shown_over_the_old_page() {
    // Q1's page is on screen; Q2 is running and its explain answered first.
    let (s, _) = act(page_of(browsing(), query_a()), FindAction::Run(query_b()));
    let s = explained(s, ns(), query_b(), Ok(verdict()));
    assert!(matches!(s.verdict, VerdictState::Ready { .. }));
    assert_eq!(shown(&s), pending(query_b()), "checking, not Q2's badge");
}

#[test]
fn a_verdict_is_hidden_when_its_find_failed_and_the_old_page_stays() {
    let (s, _) = act(page_of(browsing(), query_a()), FindAction::Run(query_b()));
    let s = explained(s, ns(), query_b(), Ok(verdict()));
    let s = AppState {
        loading_page: false,
        ..s
    };
    assert_eq!(shown(&s), VerdictState::None);
}

#[test]
fn an_unavailable_verdict_follows_the_same_rule() {
    let s = page_of(browsing(), query_a());
    let s = AppState {
        verdict: VerdictState::Unavailable {
            query: query_b(),
            error: CoreError::Unauthorized,
        },
        ..s
    };
    assert_eq!(shown(&s), VerdictState::None);
}

// ---- an explain failure never raises a banner ----

fn explain_failed(state: AppState) -> AppState {
    let event = Event::Failed {
        session: EMBEDDED_SESSION,
        context: "explain_find",
        error: CoreError::Database("boom".into()),
    };
    update(state, Input::Event(event)).0
}

#[test]
fn a_failed_explain_event_turns_the_pending_run_unavailable_without_a_banner() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let loading = s.loading_page;
    let s = explain_failed(s);
    assert_eq!(
        s.verdict,
        VerdictState::Unavailable {
            query: query_a(),
            error: CoreError::Database("boom".into())
        }
    );
    assert!(s.banner.is_none() && s.failure.is_none());
    assert_eq!(s.loading_page, loading);
}

#[test]
fn a_failed_explain_event_with_no_run_waiting_is_dropped_silently() {
    let s = explain_failed(browsing());
    assert_eq!(s.verdict, VerdictState::None);
    assert!(s.banner.is_none() && s.failure.is_none());
}

// ---- other sessions and the setting ----

#[test]
fn an_explain_for_a_background_session_lands_in_its_parked_view() {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let connected = |verdict| SessionView {
        status: ConnStatus::Connected,
        sidebar: browsing().sidebar,
        verdict,
        ..SessionView::default()
    };
    let s = AppState::new(vec![])
        .opened_with(a, connected(VerdictState::None))
        .opened_with(b, connected(pending(query_a())))
        .activated(a);
    let event = Event::Explain {
        session: b,
        ns: ns(),
        query: query_a(),
        outcome: Ok(verdict()),
    };
    let (s, _) = update(s, Input::Event(event));
    assert_eq!(s.active, Some(a));
    assert_eq!(s.verdict, VerdictState::None, "the foreground is untouched");
    assert_eq!(s.parked[&b].verdict, ready_for(query_a()));
}

#[test]
fn an_explain_arriving_after_the_setting_was_turned_off_is_dropped() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::SetAutomaticVerdict(false));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert_eq!(s.verdict, VerdictState::None);
}

#[test]
fn an_explain_arriving_after_off_then_on_is_dropped_too() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let (s, _) = act(s, FindAction::SetAutomaticVerdict(false));
    let (s, _) = act(s, FindAction::SetAutomaticVerdict(true));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert_eq!(s.verdict, VerdictState::None);
}
