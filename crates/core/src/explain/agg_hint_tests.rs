//! The hints of an aggregate plan: when each is raised and, as much, when it is not.

use bson::{Document, doc};

use super::parse_aggregate_explain;
use crate::ejson::editor_json_to_doc;
use crate::explain::{AggHint, AggPlan, AggPlanOutcome};

const GROUP_STATS: &str =
    include_str!("../../tests/fixtures/explain_aggregate/group_over_ixscan_stats.json");
const EQ_LOOKUP: &str = include_str!("../../tests/fixtures/explain_aggregate/eq_lookup.json");
const TOPLEVEL_STATS: &str =
    include_str!("../../tests/fixtures/explain_aggregate/toplevel_stats.json");

fn plan_of(doc: &Document) -> AggPlan {
    match parse_aggregate_explain(doc) {
        AggPlanOutcome::Plan(p) => p,
        other => panic!("expected a plan, got {other:?}"),
    }
}

fn plan_from_text(text: &str) -> AggPlan {
    plan_of(&editor_json_to_doc(text).expect("fixture parses"))
}

// --- ExaminedMuchMoreThanReturned over a folded $group ---

#[test]
fn a_group_folded_into_the_plan_does_not_make_a_good_scan_look_wasteful() {
    let p = plan_from_text(GROUP_STATS);
    let input = p.input.as_ref().expect("input");
    assert_eq!(
        input.n_returned,
        Some(5000),
        "what the scan handed to the group"
    );
    assert_eq!(p.summary.n_returned, Some(20), "the pipeline's own output");
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

#[test]
fn a_genuinely_poor_scan_under_a_group_still_raises_the_hint() {
    let text = GROUP_STATS
        .replace("\"nReturned\": 5000", "\"nReturned\": 10")
        .replace("5000", "100000");
    let p = plan_from_text(&text);
    assert_eq!(p.input.as_ref().and_then(|i| i.n_returned), Some(10));
    assert_eq!(p.hints, [AggHint::ExaminedMuchMoreThanReturned]);
}

#[test]
fn a_group_whose_input_count_is_unknown_raises_no_hint() {
    let text = GROUP_STATS.replace("\"nReturned\": 5000,", "");
    let p = plan_from_text(&text);
    assert_eq!(p.input.as_ref().and_then(|i| i.n_returned), None);
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

// --- $lookup strategies ---

fn lookup_hints(from: &str, to: &str) -> Vec<AggHint> {
    plan_from_text(&EQ_LOOKUP.replace(from, to)).hints
}

#[test]
fn only_a_nested_loop_join_is_a_lookup_collection_scan() {
    assert_eq!(
        lookup_hints("IndexedLoopJoin", "NestedLoopJoin"),
        [AggHint::LookupCollectionScan]
    );
    for strategy in [
        "IndexedLoopJoin",
        "DynamicIndexedLoopJoin",
        "NonExistentForeignCollection",
        "HashJoin",
    ] {
        assert!(
            lookup_hints("IndexedLoopJoin", strategy).is_empty(),
            "{strategy} must raise no hint"
        );
    }
}

#[test]
fn a_lookup_that_unwinds_in_the_plan_is_read_like_a_lookup() {
    let nested = EQ_LOOKUP
        .replace("\"EQ_LOOKUP\"", "\"EQ_LOOKUP_UNWIND\"")
        .replace("IndexedLoopJoin", "NestedLoopJoin");
    let p = plan_from_text(&nested);
    assert_eq!(p.hints, [AggHint::LookupCollectionScan]);
    let unwound = EQ_LOOKUP.replace("\"EQ_LOOKUP\"", "\"EQ_LOOKUP_UNWIND\"");
    let p = plan_from_text(&unwound);
    assert!(p.hints.is_empty(), "{:?}", p.hints);
    assert_eq!(
        p.input.as_ref().map(|i| i.stage_kind.as_str()),
        Some("COLLSCAN"),
        "the local side is still the input; the joined index is ignored"
    );
}

// --- CollectionScan needs a filter ---

#[test]
fn a_scan_without_a_filter_is_no_finding_but_a_filtered_one_is() {
    let unfiltered = EQ_LOOKUP;
    assert!(plan_from_text(unfiltered).hints.is_empty());
    let filtered = EQ_LOOKUP.replacen("\"filter\": {}", "\"filter\": { \"a\": { \"$gt\": 1 } }", 1);
    assert_eq!(plan_from_text(&filtered).hints, [AggHint::CollectionScan]);
}

#[test]
fn an_unfiltered_scan_under_a_sort_is_still_a_finding() {
    let text = TOPLEVEL_STATS.replace("\"filter\": { \"total\": { \"$gt\": 100 } },", "");
    let p = plan_from_text(&text);
    assert!(p.hints.contains(&AggHint::CollectionScan), "{:?}", p.hints);
}

// --- InMemorySort only where an index could serve it ---

fn cursor_then(stages: Vec<Document>) -> Document {
    let mut all = vec![doc! { "$cursor": { "queryPlanner": { "winningPlan": {
        "stage": "COLLSCAN",
        "filter": { "status": { "$eq": "open" } },
        "direction": "forward",
    } } } }];
    all.extend(stages);
    doc! { "stages": all, "ok": 1 }
}

fn sort_stage() -> Document {
    doc! { "$sort": { "sortKey": { "total": -1 } } }
}

fn has_in_memory_sort(stages: Vec<Document>) -> bool {
    plan_of(&cursor_then(stages))
        .hints
        .contains(&AggHint::InMemorySort)
}

#[test]
fn a_sort_straight_after_the_input_is_a_finding() {
    assert!(has_in_memory_sort(vec![sort_stage()]));
}

#[test]
fn a_sort_after_a_stage_that_reshapes_documents_is_not() {
    let reshaping = [
        doc! { "$group": { "_id": "$customer", "total": { "$sum": "$amount" } } },
        doc! { "$unwind": "$items" },
        doc! { "$addFields": { "total": 1 } },
        doc! { "$lookup": { "from": "c", "as": "c", "localField": "a", "foreignField": "b" } },
        doc! { "$facet": { "a": [] } },
        doc! { "$bucket": { "groupBy": "$a", "boundaries": [0, 10] } },
        doc! { "$project": { "total": { "$sum": "$items" } } },
    ];
    for stage in reshaping {
        let name = stage.keys().next().cloned().unwrap_or_default();
        assert!(
            !has_in_memory_sort(vec![stage, sort_stage()]),
            "a sort after {name} cannot use an index"
        );
    }
}

#[test]
fn a_sort_after_a_plain_projection_or_a_limit_still_could() {
    assert!(has_in_memory_sort(vec![
        doc! { "$project": { "total": 1, "_id": 0 } },
        sort_stage()
    ]));
    assert!(has_in_memory_sort(vec![doc! { "$limit": 5 }, sort_stage()]));
}

#[test]
fn a_sort_after_a_group_folded_into_the_input_is_not_a_finding() {
    let cursor = doc! { "$cursor": { "queryPlanner": { "winningPlan": {
        "stage": "GROUP",
        "inputStage": { "stage": "COLLSCAN", "filter": { "a": 1 } },
    } } } };
    let explain = doc! { "stages": [cursor, sort_stage()], "ok": 1 };
    let p = plan_of(&explain);
    assert!(!p.hints.contains(&AggHint::InMemorySort), "{:?}", p.hints);
}

#[test]
fn a_blocking_sort_in_the_input_plan_is_still_a_finding() {
    let p = plan_from_text(TOPLEVEL_STATS);
    assert!(p.hints.contains(&AggHint::InMemorySort));
}
