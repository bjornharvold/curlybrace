//! Create index... on a row of the Queries view: the builder opens pre-filled, or nothing happens.
use super::index_write_tests::{ns, shown};
use super::queries_test_support::{event, flags, opened, queries};
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{IndexWrite, QueriesAction};
use curlybrace_core::explain::VerdictKind;
use curlybrace_core::history::{ShapeSource, ShapeSummary};
use curlybrace_core::indexes::KeyKind;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::shape::from_query;
use curlybrace_core::store::Query;
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
    queries(s, QueriesAction::CreateIndex { hash: hash.into() })
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
