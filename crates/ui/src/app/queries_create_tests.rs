//! Create index... on a row of the Queries view: the builder opens pre-filled, or nothing happens.
use super::index_write_tests::{ns, shown};
use super::queries_test_support::{event, flags, opened, queries};
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{BuilderProblem, IndexWrite, QueriesAction};
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ShapeSource, ShapeSummary};
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::shape::from_query;
use curlybrace_core::store::CollectionIndex;
use curlybrace_core::store::Query;
use curlybrace_core::test_support::mock_objects::mock_index_spec_named;
use uuid::Uuid;

fn missed(hash: &str, latest: VerdictKind) -> ShapeSummary {
    let query = Query {
        filter: bson::doc! { "email": 1 },
        sort: Some(bson::doc! { "createdAt": -1 }),
        ..Query::default()
    };
    ShapeSummary {
        hash: hash.into(),
        shape_json: from_query(&query).json(),
        source: ShapeSource::App,
        executions: 4,
        misses: 2,
        total_millis: 40,
        max_millis: 20,
        worst: VerdictKind::CollectionScan,
        latest,
        buckets: Vec::new(),
    }
}

/// The Queries view of a connection with these flags, holding `shapes`.
fn holding(production: bool, read_only: bool, shapes: Vec<ShapeSummary>) -> (AppState, Uuid) {
    let (s, id) = opened(flags(production, read_only));
    let request = s.indexes.queries.request;
    let arrived = Event::QueryShapes {
        session: id,
        ns: ns(),
        range: s.indexes.queries.range,
        shapes,
        request,
    };
    let (s, _) = event(s, arrived);
    (s, id)
}

fn create(s: AppState, hash: &str) -> (AppState, Vec<Command>) {
    create_from(s, hash, ShapeSource::App)
}

fn create_from(s: AppState, hash: &str, source: ShapeSource) -> (AppState, Vec<Command>) {
    let action = QueriesAction::CreateIndex {
        hash: hash.into(),
        source,
    };
    queries(s, action)
}

#[test]
fn create_index_ignores_rows_held_for_another_collection() {
    // Rows read for another collection must never pre-fill a builder for the one selected now.
    let (mut s, _) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    s.indexes.ns = Some(curlybrace_core::store::Namespace::new("app", "pets"));
    let (s, commands) = create(s, "h1");
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

#[test]
fn create_index_picks_the_row_by_hash_and_source() {
    // The same hash can be recorded from the app and from typed queries: two rows.
    let app = missed("h1", VerdictKind::CollectionScan);
    let typed = ShapeSummary {
        source: ShapeSource::Typed,
        shape_json: from_query(&Query {
            filter: bson::doc! { "sku": 1 },
            ..Query::default()
        })
        .json(),
        ..missed("h1", VerdictKind::CollectionScan)
    };
    let (s, _) = holding(false, false, vec![app, typed]);
    let (s, _) = create_from(s, "h1", ShapeSource::Typed);
    let builder = s.indexes.builder.as_ref().expect("the builder is open");
    let fields: Vec<&str> = builder.keys.iter().map(|k| k.field.as_str()).collect();
    assert_eq!(fields, ["sku"]);
}

#[test]
fn create_index_opens_the_builder_with_the_shape_s_keys() {
    let (s, id) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    let (s, commands) = create(s, "h1");
    let builder = s.indexes.builder.as_ref().expect("the builder is open");
    assert_eq!(builder.ns, ns());
    let keys: Vec<(&str, KeyKind)> = builder
        .keys
        .iter()
        .map(|k| (k.field.as_str(), k.kind.clone()))
        .collect();
    assert_eq!(
        keys,
        [("email", KeyKind::Asc), ("createdAt", KeyKind::Desc)]
    );
    assert_eq!(builder.name, "email_1_createdAt_-1");
    assert_eq!(builder.collation_text, "", "an app shape has no collation");
    // The index list was never loaded here, so it is asked for too (under the number it awaits).
    assert_eq!(
        shown(&commands),
        shown(&[
            Command::EstimatedCount {
                session: id,
                ns: ns()
            },
            Command::ListIndexes {
                session: id,
                ns: ns(),
                request: s.indexes.request
            },
        ])
    );
    assert!(s.indexes.loading);
}

fn listed(keys: &[(&str, KeyKind)]) -> CollectionIndex {
    CollectionIndex {
        spec: mock_index_spec_named("existing", keys),
        size_bytes: None,
    }
}

#[test]
fn the_builder_checks_clashes_against_a_list_loaded_by_the_command() {
    let (s, id) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    let (s, _) = create(s, "h1");
    let request = s.indexes.request;
    let existing = listed(&[("email", KeyKind::Asc), ("createdAt", KeyKind::Desc)]);
    let (s, _) = event(
        s,
        Event::Indexes {
            session: id,
            ns: ns(),
            indexes: vec![existing],
            request,
        },
    );
    let builder = s.indexes.builder.as_ref().expect("the builder stays open");
    assert!(matches!(
        builder.problem_among(&s.indexes.indexes),
        Some(BuilderProblem::SameKeysAsExisting(name)) if name == "existing"
    ));
}

#[test]
fn a_list_that_is_already_loaded_is_not_asked_for_again() {
    let (s, id) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    let held = s
        .indexes
        .clone()
        .with_indexes(vec![listed(&[("other", KeyKind::Asc)])]);
    let s = AppState { indexes: held, ..s };
    let (_, commands) = create(s, "h1");
    assert_eq!(
        shown(&commands),
        shown(&[Command::EstimatedCount {
            session: id,
            ns: ns()
        }])
    );
}

#[test]
fn create_index_on_a_read_only_connection_shows_the_read_only_banner_and_sends_nothing() {
    let (s, _) = holding(false, true, vec![missed("h1", VerdictKind::CollectionScan)]);
    let (s, commands) = create(s, "h1");
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
    assert!(matches!(s.banner, Some(Banner::ReadOnly)));
}

#[test]
fn create_index_for_a_shape_that_is_no_longer_a_miss_does_nothing() {
    let (s, _) = holding(false, false, vec![missed("h1", VerdictKind::IndexUsed)]);
    let (s, commands) = create(s, "h1");
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

#[test]
fn create_index_for_an_unknown_hash_does_nothing() {
    let (s, _) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    let (s, commands) = create(s, "nope");
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

#[test]
fn create_index_while_an_index_write_is_out_does_nothing() {
    let (mut s, _) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    s.indexes.writing = Some(IndexWrite::Create {
        name: "other_1".into(),
        replaces: None,
    });
    let (s, commands) = create(s, "h1");
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}

#[test]
fn create_index_while_disconnected_does_nothing() {
    let (mut s, _) = holding(
        false,
        false,
        vec![missed("h1", VerdictKind::CollectionScan)],
    );
    s.status = ConnStatus::Disconnected;
    let (s, commands) = create(s, "h1");
    assert!(commands.is_empty());
    assert!(s.indexes.builder.is_none());
}
