//! A deliberate change of slice 1 (#159) that a session with ONE tab can see, pinned here because
//! the transcripts do not cover it: a late failure or answer of an older find, explain or
//! estimated count (a request the tab has since replaced) is dropped. Before, a late `Failed` of
//! the older find raised a banner and ended the loading of the newer one, and an explain or count
//! answer for the same collection and query was accepted whatever its number. What a person sees
//! is the absence of a spurious banner or verdict; a lost server still fails the root.
use crate::app::VerdictState;
use crate::app::reducer::{Input, update};
use crate::app::state::ConnStatus;
use crate::app::tabs::test_support::{ev, users};
use crate::viewmodel::{FindAction, IndexesAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::Query;

use crate::app::reducer::tab_characterisation_script::browsing_one;

/// A browsing session with a newer find (and its explain) in flight; returns the older find's
/// number too.
fn replaced_find() -> (crate::app::state::AppState, uuid::Uuid, u64, u64) {
    let (mut s, a, sent) = browsing_one();
    let older = s.issued_request;
    assert!(sent.iter().any(|c| matches!(c, Command::Find { .. })));
    s.explain.guard = false;
    let (s, commands) = update(
        s,
        Input::Find(FindAction::Run(Query {
            filter: bson::doc! { "n": 1 },
            ..Query::default()
        })),
    );
    let explain = commands
        .iter()
        .find_map(|c| match c {
            Command::ExplainFind { request, .. } => Some(*request),
            _ => None,
        })
        .expect("an explain");
    assert_ne!(s.issued_request, older);
    (s, a, older, explain)
}

#[test]
fn a_late_failure_of_an_older_find_raises_no_banner_and_keeps_the_newer_wait() {
    let (s, a, older, _) = replaced_find();
    let s = ev(
        s,
        Event::RequestFailed {
            session: a,
            context: "find",
            request: older,
            error: CoreError::Database("late".into()),
        },
    );
    assert_eq!(s.banner, None);
    assert!(s.loading_page, "the newer find is still awaited");
}

#[test]
fn a_late_lost_server_of_an_older_find_still_fails_the_root() {
    let (s, a, older, _) = replaced_find();
    let s = ev(
        s,
        Event::RequestFailed {
            session: a,
            context: "find",
            request: older,
            error: CoreError::Unreachable,
        },
    );
    assert_eq!(s.status, ConnStatus::Failed);
}

#[test]
fn an_explain_with_an_old_number_is_dropped_and_the_current_one_is_taken() {
    let (s, a, older, explain) = replaced_find();
    let query = s.issued_query.clone();
    let late = |request| Event::Explain {
        session: a,
        ns: users(),
        query: query.clone(),
        outcome: Err(CoreError::Database("no plan".into())),
        request,
    };
    let s = ev(s, late(older));
    assert!(
        matches!(s.verdict, VerdictState::Pending { .. }),
        "the old number is dropped"
    );
    let s = ev(s, late(explain));
    assert!(
        matches!(s.verdict, VerdictState::Unavailable { .. }),
        "its own is taken"
    );
}

#[test]
fn an_estimated_count_with_an_old_number_is_dropped() {
    let (s, a, _, _) = replaced_find();
    let (s, commands) = update(s, Input::Indexes(IndexesAction::OpenBuilder));
    let request = commands
        .iter()
        .find_map(|c| match c {
            Command::EstimatedCount { request, .. } => Some(*request),
            _ => None,
        })
        .expect("a count");
    let count = |request| Event::EstimatedCount {
        session: a,
        ns: users(),
        count: 7,
        request,
    };
    let estimated = |s: &crate::app::state::AppState| {
        s.indexes
            .builder
            .as_ref()
            .and_then(|b| b.estimated_documents)
    };
    let s = ev(s, count(request - 1));
    assert_eq!(estimated(&s), None);
    let s = ev(s, count(request));
    assert_eq!(estimated(&s), Some(7));
}
