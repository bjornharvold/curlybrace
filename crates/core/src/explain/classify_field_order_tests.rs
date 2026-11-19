//! The field-order verdict of `classify`: a range key before an exact-match or sort key of the
//! index the plan uses.

use bson::{Document, doc};

use super::{VerdictKind, classify};
use crate::ejson::editor_json_to_doc;
use crate::store::Query;

fn real_field_order() -> Document {
    editor_json_to_doc(include_str!("fixtures/real_field_order.json")).expect("fixture parses")
}

fn real_ixscan() -> Document {
    editor_json_to_doc(include_str!("fixtures/real_ixscan.json")).expect("fixture parses")
}

/// The query the real fixture was captured for.
fn status_and_created() -> Query {
    Query {
        filter: doc! { "status": "a", "createdAt": { "$gt": 2 } },
        ..Query::default()
    }
}

fn ixscan(index: &str, keys: Document) -> Document {
    doc! { "stage": "IXSCAN", "indexName": index, "keyPattern": keys }
}

/// A plan-only explain whose winning plan is `plan`, reading the documents it finds.
fn explain_of(plan: Document) -> Document {
    let fetched = if plan.get_str("stage") == Ok("IXSCAN") {
        doc! { "stage": "FETCH", "inputStage": plan }
    } else {
        plan
    };
    doc! { "queryPlanner": { "namespace": "t.c", "winningPlan": fetched } }
}

fn with_stats(mut explain: Document, keys: i32, docs: i32, returned: i32) -> Document {
    explain.insert(
        "executionStats",
        doc! { "totalKeysExamined": keys, "totalDocsExamined": docs, "nReturned": returned },
    );
    explain
}

fn reason_keys(explain: &Document, query: &Query) -> Vec<&'static str> {
    classify(explain, query)
        .unwrap()
        .reasons
        .iter()
        .map(|r| r.key)
        .collect()
}

#[test]
fn range_before_equality_index_is_field_order_on_small_data() {
    let v = classify(&real_field_order(), &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::FieldOrder, "{v:?}");
    assert_eq!(v.index_name.as_deref(), Some("createdAt_1_status_1"));
    assert!(!v.plan_only);
    // 7 keys to return 3: far too small for the ratio check.
    assert!(v.keys_examined < 100);
    assert_eq!(v.reasons.len(), 1);
    let reason = &v.reasons[0];
    assert_eq!(reason.key, "field_order");
    let param = |name: &str| {
        reason
            .params
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(param("index"), Some("createdAt_1_status_1"));
    assert_eq!(param("range"), Some("createdAt"));
    assert_eq!(param("field"), Some("status"));
}

#[test]
fn the_same_query_through_an_equality_first_index_is_index_used() {
    let esr = editor_json_to_doc(include_str!("fixtures/real_field_order_esr.json")).unwrap();
    let v = classify(&esr, &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed, "{v:?}");
    assert_eq!(v.index_name.as_deref(), Some("status_1_createdAt_1"));
}

#[test]
fn field_order_and_inefficient_show_both_reasons() {
    let wide = with_stats(
        explain_of(ixscan(
            "createdAt_1_status_1",
            doc! { "createdAt": 1, "status": 1 },
        )),
        5000,
        5000,
        3,
    );
    let v = classify(&wide, &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::FieldOrder);
    assert_eq!(
        reason_keys(&wide, &status_and_created()),
        ["field_order", "inefficient"]
    );
}

#[test]
fn sort_after_range_stays_sort_not_covered_with_field_order_reason() {
    let plan = doc! {
        "stage": "SORT",
        "inputStage": {
            "stage": "FETCH",
            "inputStage": ixscan("createdAt_1_name_1", doc! { "createdAt": 1, "name": 1 }),
        },
    };
    let query = Query {
        filter: doc! { "createdAt": { "$gt": 5 } },
        sort: Some(doc! { "name": 1 }),
        ..Query::default()
    };
    let explain = explain_of(plan);
    let v = classify(&explain, &query).unwrap();
    assert_eq!(v.kind, VerdictKind::SortNotCovered);
    assert_eq!(
        reason_keys(&explain, &query),
        ["sort_blocking", "field_order_sort"]
    );
    let params = &v.reasons[1].params;
    assert!(params.contains(&("range", "createdAt".to_string())));
    assert!(params.contains(&("field", "name".to_string())));
    assert!(params.contains(&("index", "createdAt_1_name_1".to_string())));
    assert!(v.suggestion.is_some());
}

#[test]
fn a_regex_literal_filter_is_not_a_false_field_order() {
    let plan = explain_of(ixscan("ts_1_name_1", doc! { "ts": 1, "name": 1 }));
    let regex = bson::Bson::RegularExpression(bson::Regex {
        pattern: "foo".into(),
        options: String::new(),
    });
    let query = Query {
        filter: doc! { "ts": { "$gt": 1 }, "name": regex },
        ..Query::default()
    };
    let v = classify(&plan, &query).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed, "{v:?}");
    assert!(v.suggestion.is_none());
}

#[test]
fn collscan_still_wins_over_field_order() {
    let plan = doc! {
        "stage": "COLLSCAN",
        "direction": "forward",
    };
    let v = classify(&explain_of(plan), &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::CollectionScan);
    // A scan next to an index scan (an `$or` with an unindexed branch) is a scan too.
    let plan = doc! {
        "stage": "OR",
        "inputStages": [
            ixscan("createdAt_1_status_1", doc! { "createdAt": 1, "status": 1 }),
            { "stage": "COLLSCAN" },
        ],
    };
    let v = classify(&explain_of(plan), &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::CollectionScan);
}

#[test]
fn esr_index_is_index_used() {
    let query = Query {
        filter: doc! { "c": 5 },
        ..Query::default()
    };
    let v = classify(&real_ixscan(), &query).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
    assert!(v.reasons.is_empty());
    let plan = explain_of(ixscan(
        "status_1_createdAt_1",
        doc! { "status": 1, "createdAt": 1 },
    ));
    let v = classify(&plan, &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
}

#[test]
fn two_index_scans_are_not_judged() {
    let or = doc! {
        "stage": "OR",
        "inputStages": [
            ixscan("createdAt_1_status_1", doc! { "createdAt": 1, "status": 1 }),
            ixscan("createdAt_1_status_1", doc! { "createdAt": 1, "status": 1 }),
        ],
    };
    let plan = doc! { "stage": "FETCH", "inputStage": or };
    let v = classify(&explain_of(plan), &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
}

#[test]
fn an_index_without_a_plain_key_pattern_is_not_judged() {
    for keys in [
        doc! { "createdAt": "hashed", "status": 1 },
        doc! { "createdAt": 1, "status": "text" },
    ] {
        let v = classify(&explain_of(ixscan("x", keys)), &status_and_created()).unwrap();
        assert_eq!(v.kind, VerdictKind::IndexUsed);
    }
    // An IXSCAN with no key pattern at all.
    let plan = doc! { "stage": "IXSCAN", "indexName": "x" };
    let v = classify(&explain_of(plan), &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::IndexUsed);
}

#[test]
fn plan_only_explain_is_judged_by_keys() {
    let mut explain = real_field_order();
    explain.remove("executionStats");
    let v = classify(&explain, &status_and_created()).unwrap();
    assert!(v.plan_only);
    assert_eq!(v.kind, VerdictKind::FieldOrder);
    assert_eq!(v.reasons.len(), 1);
}

#[test]
fn field_order_gets_the_esr_suggestion() {
    let v = classify(&real_field_order(), &status_and_created()).unwrap();
    let keys = v.suggestion.expect("a suggestion").keys;
    assert_eq!(
        keys,
        vec![("status".to_string(), 1), ("createdAt".to_string(), 1)]
    );
}

#[test]
fn a_covered_plan_with_the_wrong_order_is_still_field_order() {
    let plan = doc! {
        "stage": "PROJECTION_COVERED",
        "inputStage": ixscan("createdAt_1_status_1", doc! { "createdAt": 1, "status": 1 }),
    };
    let v = classify(&explain_of(plan), &status_and_created()).unwrap();
    assert_eq!(v.kind, VerdictKind::FieldOrder);
}

#[test]
fn queries_the_check_cannot_judge_stay_index_used() {
    let plan = explain_of(ixscan(
        "createdAt_1_status_1",
        doc! { "createdAt": 1, "status": 1 },
    ));
    for filter in [
        // a logical operator at the top
        doc! { "$or": [{ "status": "a" }, { "createdAt": { "$gt": 2 } }] },
        // `$in` with a sort
        doc! { "status": { "$in": ["a", "b"] }, "createdAt": { "$gt": 2 } },
    ] {
        let query = Query {
            filter,
            sort: Some(doc! { "name": 1 }),
            ..Query::default()
        };
        let v = classify(&plan, &query).unwrap();
        assert_eq!(v.kind, VerdictKind::IndexUsed, "{:?}", query.filter);
    }
}
