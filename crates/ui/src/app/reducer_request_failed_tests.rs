//! A find, an explain and an estimated count are numbered reads now, so their failures arrive as
//! `RequestFailed { request }`. The UI handles them exactly as it handled the bare `Failed` they
//! used to be (#159, slice 1): the same banner, the same loading flags, the same inline outcome.
use super::tab_characterisation_script::browsing_one;
use super::{Input, update};
use crate::app::state::{AppState, Banner, ConnStatus};
use crate::app::{AggregateAction, VerdictState};
use crate::viewmodel::AggregateState;
use crate::viewmodel::FindAction;
use crate::viewmodel::aggregate_run::CountState;
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::{Namespace, Query};
use uuid::Uuid;

/// The failure of the request `state` awaits for the read `context` names.
fn request_failed(
    state: &AppState,
    session: Uuid,
    context: &'static str,
    error: CoreError,
) -> Input {
    let request = match context {
        "find" => state.issued_request,
        "explain_find" => state.verdict_request,
        _ => state.aggregate.run.count_request,
    };
    Input::Event(Event::RequestFailed {
        session,
        context,
        request,
        error,
    })
}

fn failed(session: Uuid, context: &'static str, error: CoreError) -> Input {
    Input::Event(Event::Failed {
        session,
        context,
        error,
    })
}

/// A find in flight: the state after `Next` on a first page that has more.
fn find_in_flight() -> (AppState, Uuid) {
    let (state, a, _) = browsing_one();
    let (state, commands) = update(state, Input::Find(FindAction::Next));
    assert!(matches!(commands.as_slice(), [Command::Find { .. }]));
    (state, a)
}

#[test]
fn a_find_request_failure_raises_the_same_banner_as_a_find_failure() {
    let boom = || CoreError::Database("boom".into());
    let (state, a) = find_in_flight();
    let input = request_failed(&state, a, "find", boom());
    let (via_request, commands) = update(state, input);
    let (state, a) = find_in_flight();
    let (via_failed, _) = update(state, failed(a, "find", boom()));
    assert!(commands.is_empty());
    assert!(matches!(
        via_request.banner,
        Some(Banner::Failure {
            context: "find",
            ..
        })
    ));
    assert_eq!(via_request.banner, via_failed.banner);
    assert_eq!(via_request.loading_page, via_failed.loading_page);
    assert!(!via_request.loading_page);
    assert_eq!(
        via_request.page.as_ref().map(|p| p.docs.len()),
        Some(3),
        "the previous page stays"
    );
}

#[test]
fn a_lost_server_on_a_find_request_fails_the_root_like_before() {
    let (state, a) = find_in_flight();
    let input = request_failed(&state, a, "find", CoreError::Unreachable);
    let (state, _) = update(state, input);
    assert_eq!(state.status, ConnStatus::Failed);
    assert_eq!(state.failure, Some(CoreError::Unreachable));
    // Slice 2 (T8): the tab keeps its collection through a lost connection (it was cleared).
    assert!(state.selected_ns().is_some());
    assert!(state.page.is_none() && !state.loading_page);
}

#[test]
fn a_superseded_find_request_is_consumed_like_before() {
    let (state, a) = find_in_flight();
    let input = request_failed(&state, a, "find", CoreError::Superseded);
    let (state, commands) = update(state, input);
    assert!(commands.is_empty());
    assert!(state.banner.is_none());
    assert!(state.loading_page, "a superseded answer settles nothing");
}

#[test]
fn an_explain_request_failure_leaves_the_run_without_a_verdict_like_before() {
    let (state, a, _) = browsing_one();
    let (state, commands) = update(
        state,
        Input::Find(FindAction::Run(Query {
            filter: doc! { "n": 1 },
            ..Query::default()
        })),
    );
    // With the guard on, the guard check goes first; answer it as unavailable so the run goes on.
    let state = match commands.as_slice() {
        [Command::GuardCheck { request, .. }] => {
            let request = *request;
            update(
                state,
                Input::Event(Event::GuardChecked {
                    session: a,
                    request,
                    outcome: Err(CoreError::Database("no plan".into())),
                }),
            )
            .0
        }
        _ => state,
    };
    assert!(matches!(state.verdict, VerdictState::Pending { .. }));
    let input = request_failed(&state, a, "explain_find", CoreError::Database("x".into()));
    let (state, commands) = update(state, input);
    assert!(commands.is_empty());
    assert!(matches!(state.verdict, VerdictState::Unavailable { .. }));
    assert!(state.banner.is_none(), "an explain never raises a banner");
    assert_eq!(state.status, ConnStatus::Connected);
}

#[test]
fn a_lost_server_on_an_explain_request_does_not_fail_the_root_like_before() {
    let (state, a, _) = browsing_one();
    let input = request_failed(&state, a, "explain_find", CoreError::Unreachable);
    let (state, _) = update(state, input);
    assert_eq!(state.status, ConnStatus::Connected);
    assert!(state.banner.is_none());
}

/// A connected embedded session with `app.people` selected and a pipeline ending in `$out`.
fn run_ready() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(Namespace::new("app", "people"));
    s.aggregate = Box::new(
        AggregateState::default()
            .add_stage("$match")
            .edit(0, r#"{ "$match": {} }"#)
            .add_stage("$match")
            .edit(1, r#"{ "$out": "dest" }"#),
    );
    s
}

#[test]
fn an_estimated_count_request_failure_reads_count_unavailable_like_before() {
    let waiting = || {
        let (s, commands) = update(run_ready(), Input::Aggregate(AggregateAction::RunClicked));
        assert!(
            matches!(commands.as_slice(), [Command::EstimatedCount { .. }]),
            "the dialog asks for the count: {commands:?}"
        );
        let loading = s.aggregate.run.dialog.as_ref().map(|d| d.count);
        assert_eq!(loading, Some(CountState::Loading));
        s
    };
    let error = || CoreError::Database("x".into());
    let s = waiting();
    let input = request_failed(&s, EMBEDDED_SESSION, "estimated_count", error());
    let (via_request, _) = update(s, input);
    let (via_failed, _) = update(
        waiting(),
        failed(EMBEDDED_SESSION, "estimated_count", error()),
    );
    let count = |s: &AppState| s.aggregate.run.dialog.as_ref().map(|d| d.count);
    assert_eq!(count(&via_request), Some(CountState::Unavailable));
    assert_eq!(count(&via_request), count(&via_failed));
    assert!(
        via_request.banner.is_none(),
        "a missing count raises no banner"
    );
}
