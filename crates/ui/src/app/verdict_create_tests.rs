//! Create index... in the query bar's verdict popup: the builder opens pre-filled with the shown
//! verdict's suggestion and the collation of the query it judged, or nothing happens.
use super::explain_actions::VerdictState;
use super::index_write_tests::{browsing, ns, plain, shown};
use super::queries_test_support::flags;
use super::reducer::{Input, update};
use super::state::{AppState, Banner};
use crate::viewmodel::FindAction;
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::explain::{SuggestedIndex, Verdict, VerdictKind};
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::pipeline_options::{CollationOption, PipelineOptions, Strength};
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Query;

fn query(collation: bool) -> Query {
    let collation = collation.then(|| CollationOption {
        locale: "en".into(),
        strength: Some(Strength::Secondary),
        case_level: None,
        case_first: None,
        numeric_ordering: None,
        alternate: None,
    });
    Query {
        filter: doc! { "status": "a" },
        options: PipelineOptions {
            collation,
            ..PipelineOptions::default()
        },
        ..Query::default()
    }
}

fn verdict(suggested: bool, namespace: Option<&str>) -> Verdict {
    Verdict {
        kind: VerdictKind::CollectionScan,
        index_name: None,
        keys_examined: 0,
        docs_examined: 10,
        returned: 1,
        plan_only: false,
        reasons: Vec::new(),
        suggestion: suggested.then(|| SuggestedIndex {
            keys: vec![("status".into(), 1), ("createdAt".into(), -1)],
        }),
        namespace: namespace.map(str::to_string),
    }
}

/// `s` with `state` as the verdict of the run on screen.
fn showing(s: AppState, query: &Query, state: impl FnOnce(Query) -> VerdictState) -> AppState {
    AppState {
        page_query: query.clone(),
        verdict: state(query.clone()),
        ..s
    }
}

fn ready(v: Verdict) -> impl FnOnce(Query) -> VerdictState {
    move |query| VerdictState::Ready { query, verdict: v }
}

fn create(s: AppState) -> (AppState, Vec<Command>) {
    update(s, Input::Find(FindAction::CreateSuggestedIndex))
}

#[test]
fn create_from_verdict_carries_keys_and_the_query_s_collation() {
    let (s, id) = plain();
    let q = query(true);
    let s = showing(s, &q, ready(verdict(true, Some("app.people"))));
    let (s, commands) = create(s);
    let builder = s.indexes.builder.as_ref().expect("the builder is open");
    assert_eq!(builder.ns, ns());
    let keys: Vec<(&str, KeyKind)> = builder
        .keys
        .iter()
        .map(|k| (k.field.as_str(), k.kind.clone()))
        .collect();
    assert_eq!(
        keys,
        [("status", KeyKind::Asc), ("createdAt", KeyKind::Desc)]
    );
    assert!(builder.collation_text.contains("\"en\""), "{builder:?}");
    assert_eq!(
        shown(&commands),
        shown(&[Command::EstimatedCount {
            session: id,
            ns: ns()
        }])
    );
}

#[test]
fn create_from_verdict_without_a_collation_has_none() {
    let (s, _) = plain();
    let q = query(false);
    let s = showing(s, &q, ready(verdict(true, None)));
    let (s, _) = create(s);
    let builder = s.indexes.builder.as_ref().expect("the builder is open");
    assert_eq!(builder.collation_text, "");
}

#[test]
fn create_from_verdict_without_suggestion_does_nothing() {
    let (s, _) = plain();
    let q = query(false);
    let s = showing(s, &q, ready(verdict(false, None)));
    let (s, commands) = create(s);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

#[test]
fn create_from_a_verdict_planned_on_another_collection_does_nothing() {
    // A view runs on its source collection: the badge offers no suggestion either.
    let (s, _) = plain();
    let q = query(false);
    let s = showing(s, &q, ready(verdict(true, Some("app.source_of_a_view"))));
    let (s, commands) = create(s);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

#[test]
fn create_from_a_pending_or_unavailable_verdict_does_nothing() {
    let q = query(false);
    for state in [
        (|query| VerdictState::Pending { query }) as fn(Query) -> VerdictState,
        |query| VerdictState::Unavailable {
            query,
            error: CoreError::Unreachable,
        },
        |_| VerdictState::None,
    ] {
        let (s, _) = plain();
        let (s, commands) = create(showing(s, &q, state));
        assert!(commands.is_empty());
        assert!(s.indexes.builder.is_none());
    }
}

#[test]
fn create_from_verdict_on_read_only_sends_nothing() {
    let (s, _) = browsing(flags(false, true));
    let q = query(false);
    let s = showing(s, &q, ready(verdict(true, None)));
    let (s, commands) = create(s);
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
    assert!(matches!(s.banner, Some(Banner::ReadOnly)));
}
