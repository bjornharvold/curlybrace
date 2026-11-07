//! The verdict of the current run: issued with `Run`, kept across paging, dropped when stale.

use super::explain_actions::VerdictState;
use super::reducer::{Input, update};
use super::state::{AppState, ConnStatus};
use crate::viewmodel::FindAction;
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{Verdict, VerdictKind};
use curlybrace_core::runtime::EMBEDDED_SESSION;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page, Query};

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

fn other_ns() -> Namespace {
    Namespace::new("app", "pets")
}

fn browsing() -> AppState {
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

fn query_a() -> Query {
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

fn verdict() -> Verdict {
    Verdict {
        kind: VerdictKind::CollectionScan,
        index_name: None,
        keys_examined: 0,
        docs_examined: 10,
        returned: 1,
        plan_only: false,
        reasons: Vec::new(),
        suggestion: None,
    }
}

fn act(state: AppState, action: FindAction) -> (AppState, Vec<Command>) {
    update(state, Input::Find(action))
}

fn explained(
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
    let sidebar = crate::viewmodel::SidebarAction::Open(other_ns());
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
