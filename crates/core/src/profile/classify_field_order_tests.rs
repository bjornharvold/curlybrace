//! The field-order verdict of `classify_entry`: judged from the keys in the plan summary.

use bson::{Document, doc};

use super::entry_tests::entry;
use super::{ProfileEntry, classify_entry};
use crate::explain::VerdictKind;
use crate::shape::{Shape, from_profile_command};

/// A profiler entry of the verb `command` starts with, running `plan_summary`.
fn entry_for(plan_summary: &str, command: Document) -> ProfileEntry {
    let verb = command.keys().next().cloned().unwrap_or_default();
    let op = match verb.as_str() {
        "find" => "query",
        "update" => "update",
        _ => "command",
    };
    ProfileEntry {
        op: op.into(),
        plan_summary: plan_summary.into(),
        keys_examined: 3,
        docs_examined: 3,
        returned: 3,
        command,
        ..entry("ixscan")
    }
}

fn shape_of(entry: &ProfileEntry) -> Shape {
    from_profile_command(&entry.op, &entry.command).expect("the command has a shape")
}

const RANGE_FIRST: &str = "IXSCAN { createdAt: 1, status: 1 }";

fn find_filter() -> Document {
    doc! { "status": "a", "createdAt": { "$gt": 2 } }
}

fn find_verdict(plan: &str) -> VerdictKind {
    let e = entry_for(plan, doc! { "find": "orders", "filter": find_filter() });
    classify_entry(&e, &shape_of(&e))
}

#[test]
fn profile_entry_with_range_first_index_is_field_order() {
    assert_eq!(find_verdict(RANGE_FIRST), VerdictKind::FieldOrder);
}

#[test]
fn an_index_in_equality_then_range_order_is_index_used() {
    assert_eq!(
        find_verdict("IXSCAN { status: 1, createdAt: 1 }"),
        VerdictKind::IndexUsed
    );
}

#[test]
fn unparseable_summary_is_not_field_order() {
    for plan in [
        "IXSCAN { createdAt: 1, status: hashed }",
        "IXSCAN { createdAt: 1",
        "IXSCAN { createdAt: 1, status: 1 }, IXSCAN { status: 1 }",
        "IXSCAN { a:b: 1, status: 1 }",
        "IXSCAN",
        "MULTI_ITERATOR",
    ] {
        assert_eq!(find_verdict(plan), VerdictKind::IndexUsed, "{plan}");
    }
}

#[test]
fn a_collection_scan_still_wins_over_field_order() {
    assert_eq!(
        find_verdict("COLLSCAN, IXSCAN { createdAt: 1, status: 1 }"),
        VerdictKind::CollectionScan
    );
}

#[test]
fn field_order_beats_inefficient() {
    let mut e = entry_for(
        RANGE_FIRST,
        doc! { "find": "orders", "filter": find_filter() },
    );
    e.keys_examined = 5000;
    e.docs_examined = 5000;
    e.returned = 3;
    assert_eq!(classify_entry(&e, &shape_of(&e)), VerdictKind::FieldOrder);
}

#[test]
fn an_in_filter_with_a_sort_is_not_judged() {
    let command = doc! {
        "find": "orders",
        "filter": { "status": { "$in": ["a", "b"] }, "createdAt": { "$gt": 2 } },
        "sort": { "name": 1 },
    };
    let e = entry_for("IXSCAN { createdAt: 1, status: 1, name: 1 }", command);
    assert_eq!(classify_entry(&e, &shape_of(&e)), VerdictKind::IndexUsed);
}

#[test]
fn a_range_field_that_is_also_the_sort_is_not_judged() {
    let command = doc! {
        "find": "orders",
        "filter": find_filter(),
        "sort": { "createdAt": -1 },
    };
    let e = entry_for(RANGE_FIRST, command);
    assert_eq!(classify_entry(&e, &shape_of(&e)), VerdictKind::IndexUsed);
}

#[test]
fn a_sort_stage_stays_sort_not_covered() {
    let mut e = entry_for(
        RANGE_FIRST,
        doc! { "find": "orders", "filter": find_filter() },
    );
    e.has_sort_stage = true;
    assert_eq!(
        classify_entry(&e, &shape_of(&e)),
        VerdictKind::SortNotCovered
    );
}

#[test]
fn count_and_aggregate_entries_follow_the_same_rule() {
    let count = entry_for(
        RANGE_FIRST,
        doc! { "count": "orders", "query": find_filter() },
    );
    assert_eq!(
        classify_entry(&count, &shape_of(&count)),
        VerdictKind::FieldOrder
    );
    let aggregate = entry_for(
        RANGE_FIRST,
        doc! {
            "aggregate": "orders",
            "pipeline": [{ "$match": find_filter() }, { "$group": { "_id": "$region" } }],
            "cursor": {},
        },
    );
    assert_eq!(
        classify_entry(&aggregate, &shape_of(&aggregate)),
        VerdictKind::FieldOrder
    );
}

#[test]
fn an_update_entry_is_judged_by_its_first_statement() {
    let update = entry_for(
        RANGE_FIRST,
        doc! {
            "update": "orders",
            "updates": [{ "q": find_filter(), "u": { "$set": { "x": 1 } } }],
        },
    );
    assert_eq!(
        classify_entry(&update, &shape_of(&update)),
        VerdictKind::FieldOrder
    );
}
