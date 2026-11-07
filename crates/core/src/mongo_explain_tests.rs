use bson::doc;

use super::*;
use crate::store::{Namespace, Query};

fn ns() -> Namespace {
    Namespace::new("db", "orders")
}

fn full_query() -> Query {
    Query {
        filter: doc! { "status": "open" },
        sort: Some(doc! { "created": -1 }),
        projection: Some(doc! { "status": 1 }),
        skip: 20,
        limit: 50,
    }
}

#[test]
fn command_wraps_the_find_with_verbosity_and_time_limit() {
    let cmd = explain_command(&ns(), &full_query(), Verbosity::ExecutionStats).unwrap();
    assert_eq!(cmd.get_str("verbosity").unwrap(), "executionStats");
    assert_eq!(cmd.get_i64("maxTimeMS").unwrap(), EXPLAIN_MAX_TIME_MS);
    assert_eq!(EXPLAIN_MAX_TIME_MS, 2000);
    assert!(cmd.get_document("explain").is_ok());
}

#[test]
fn command_puts_filter_sort_projection_skip_limit_in_the_inner_find() {
    let cmd = explain_command(&ns(), &full_query(), Verbosity::ExecutionStats).unwrap();
    let find = cmd.get_document("explain").unwrap();
    assert_eq!(find.get_str("find").unwrap(), "orders");
    assert_eq!(
        find.get_document("filter").unwrap(),
        &doc! { "status": "open" }
    );
    assert_eq!(find.get_document("sort").unwrap(), &doc! { "created": -1 });
    assert_eq!(
        find.get_document("projection").unwrap(),
        &doc! { "status": 1 }
    );
    assert_eq!(find.get_i64("skip").unwrap(), 20);
    // One more than the page, exactly like `find`, so the plan matches the real query.
    assert_eq!(find.get_i64("limit").unwrap(), 51);
}

#[test]
fn command_omits_absent_sort_projection_skip_and_unlimited_limit() {
    let q = Query {
        limit: 0,
        ..Query::default()
    };
    let cmd = explain_command(&ns(), &q, Verbosity::QueryPlanner).unwrap();
    assert_eq!(cmd.get_str("verbosity").unwrap(), "queryPlanner");
    let find = cmd.get_document("explain").unwrap();
    assert_eq!(find.get_document("filter").unwrap(), &doc! {});
    for absent in ["sort", "projection", "skip", "limit"] {
        assert!(!find.contains_key(absent), "{absent} should be absent");
    }
}

#[test]
fn command_rejects_a_skip_beyond_the_wire_range() {
    let q = Query {
        skip: u64::MAX,
        ..Query::default()
    };
    assert!(explain_command(&ns(), &q, Verbosity::ExecutionStats).is_err());
}

#[test]
fn time_limit_is_code_50_only() {
    assert!(is_time_limit(Some(50)));
    assert!(!is_time_limit(Some(13)));
    assert!(!is_time_limit(None));
}
