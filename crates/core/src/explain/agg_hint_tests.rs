//! The hints of an aggregate plan: when each is raised and, as much, when it is not.

use bson::{Document, doc};

use super::parse_aggregate_explain;
use crate::ejson::editor_json_to_doc;
use crate::explain::{AggHint, AggPlan, AggPlanOutcome};

// Captured from a MongoDB 9.0 server (20000 documents, an index on `status`).
const GOOD_GROUP: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_group_sbe_stats.json");
const POOR_GROUP: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_poor_scan_group_sbe_stats.json");
const LOOKUP_GROUP: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_lookup_group_sbe_stats.json");
const SORT_GROUP: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_sort_group_sbe_stats.json");
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

// --- ExaminedMuchMoreThanReturned over a folded $group (real SBE output) ---

#[test]
fn a_well_indexed_match_and_group_does_not_look_wasteful() {
    let p = plan_from_text(GOOD_GROUP);
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.n_returned, Some(5000), "what the scan handed on");
    assert_eq!(p.summary.n_returned, Some(5), "the pipeline's own output");
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

#[test]
fn a_poor_scan_under_a_group_raises_the_hint() {
    let p = plan_from_text(POOR_GROUP);
    assert_eq!(p.input.as_ref().and_then(|i| i.docs_examined), Some(20000));
    assert_eq!(p.input.as_ref().and_then(|i| i.n_returned), Some(10));
    assert_eq!(
        p.hints,
        [
            AggHint::CollectionScan,
            AggHint::ExaminedMuchMoreThanReturned
        ]
    );
}

#[test]
fn a_group_over_a_joined_input_reads_the_outer_reducer_not_the_joined_subplan() {
    let p = plan_from_text(LOOKUP_GROUP);
    assert_eq!(p.input.as_ref().and_then(|i| i.n_returned), Some(5000));
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

#[test]
fn a_group_in_a_shape_that_is_not_recognised_raises_no_hint() {
    let text = POOR_GROUP.replace("\"stage\": \"group\"", "\"stage\": \"mystery\"");
    let p = plan_from_text(&text);
    assert_eq!(p.input.as_ref().and_then(|i| i.n_returned), None);
    assert!(
        !p.hints.contains(&AggHint::ExaminedMuchMoreThanReturned),
        "{:?}",
        p.hints
    );
}

#[test]
fn an_uppercase_classic_style_group_stage_is_read_too() {
    let text = POOR_GROUP.replace("\"stage\": \"group\"", "\"stage\": \"GROUP\"");
    let p = plan_from_text(&text);
    assert_eq!(p.input.as_ref().and_then(|i| i.n_returned), Some(10));
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
fn a_sort_after_a_plain_inclusion_projection_still_could() {
    assert!(has_in_memory_sort(vec![
        doc! { "$project": { "total": 1, "_id": 0 } },
        sort_stage()
    ]));
}

fn hints_of(stages: Vec<Document>) -> Vec<AggHint> {
    plan_of(&cursor_then(stages)).hints
}

#[test]
fn a_sort_after_a_limit_is_not_a_finding() {
    assert!(!has_in_memory_sort(vec![
        doc! { "$limit": 5 },
        sort_stage()
    ]));
}

#[test]
fn a_sort_after_a_skip_is_still_a_finding_without_the_index_promise() {
    let hints = hints_of(vec![doc! { "$skip": 5 }, sort_stage()]);
    assert_eq!(
        hints,
        [AggHint::CollectionScan, AggHint::InMemorySortAfterSkip],
        "no index can serve it, but the sort is real"
    );
}

#[test]
fn a_reshaping_or_limiting_stage_after_the_skip_ends_the_finding() {
    for stage in [
        doc! { "$limit": 5 },
        doc! { "$group": { "_id": "$a" } },
        doc! { "$unwind": "$a" },
    ] {
        let hints = hints_of(vec![doc! { "$skip": 5 }, stage, sort_stage()]);
        assert!(
            !hints.contains(&AggHint::InMemorySortAfterSkip)
                && !hints.contains(&AggHint::InMemorySort),
            "{hints:?}"
        );
    }
}

#[test]
fn a_skip_after_a_reshaping_stage_does_not_bring_the_finding_back() {
    let hints = hints_of(vec![
        doc! { "$group": { "_id": "$a" } },
        doc! { "$skip": 5 },
        sort_stage(),
    ]);
    assert!(
        !hints.contains(&AggHint::InMemorySortAfterSkip),
        "{hints:?}"
    );
}

#[test]
fn a_nested_inclusion_projection_does_not_reshape_but_a_computed_one_does() {
    // `{"a.b": 1}` is printed by the server as `{"a": {"b": true}}`.
    assert!(has_in_memory_sort(vec![
        doc! { "$project": { "a": { "b": true }, "_id": false } },
        sort_stage()
    ]));
    assert!(!has_in_memory_sort(vec![
        doc! { "$project": { "a": { "$literal": 1 } } },
        sort_stage()
    ]));
}

#[test]
fn desugared_reshaping_stages_hide_the_sort_from_the_index() {
    for name in [
        "$_internalSetWindowFields",
        "$_internalDensify",
        "$_internalSearchMongotRemote",
        "$_internalSearchIdLookup",
        "$vectorSearch",
        "$sample",
    ] {
        let mut stage = Document::new();
        stage.insert(name, doc! {});
        assert!(
            !has_in_memory_sort(vec![stage, sort_stage()]),
            "a sort after {name} cannot use an index"
        );
    }
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

// --- a SORT inside the pushed-down plan counts only on the input side ---

fn planned(winning: Document) -> AggPlan {
    plan_of(&doc! { "queryPlanner": { "winningPlan": winning }, "ok": 1 })
}

#[test]
fn a_sort_feeding_a_group_is_a_finding_and_so_is_its_unfiltered_scan() {
    let p = plan_from_text(SORT_GROUP);
    assert!(p.hints.contains(&AggHint::InMemorySort), "{:?}", p.hints);
    assert!(p.hints.contains(&AggHint::CollectionScan), "{:?}", p.hints);
}

#[test]
fn a_sort_above_a_group_runs_on_reshaped_data_and_is_no_finding() {
    // The server keeps a `$sort` after a `$group` as its own stage; this is the plan shape of a
    // version that pushes it down (a SORT over the GROUP over the scan).
    let p = planned(doc! { "stage": "SORT", "inputStage": {
    "stage": "GROUP", "inputStage": { "stage": "COLLSCAN", "filter": {} } } });
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

#[test]
fn a_sort_above_a_lookup_or_unwind_is_no_finding_but_one_below_it_is() {
    for reshaper in ["UNWIND", "EQ_LOOKUP", "EQ_LOOKUP_UNWIND"] {
        let above = planned(doc! { "stage": "SORT", "inputStage": {
        "stage": reshaper, "inputStage": { "stage": "COLLSCAN", "filter": {} } } });
        assert!(above.hints.is_empty(), "{reshaper}: {:?}", above.hints);
        let below = planned(doc! { "stage": reshaper, "inputStage": {
        "stage": "SORT", "inputStage": { "stage": "COLLSCAN", "filter": {} } } });
        assert!(
            below.hints.contains(&AggHint::InMemorySort),
            "{reshaper}: {:?}",
            below.hints
        );
    }
}

// --- a leading $limit / $skip the server folds into the $cursor (real output, MongoDB 9.0) ---

const MATCH_LIMIT_SORT: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_limit_sort_match_limit_sort.json");
const SKIP_SORT: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_limit_sort_skip_sort.json");
const LIMIT_SORT: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_limit_sort_limit_sort.json");
const MATCH_SORT: &str =
    include_str!("../../tests/fixtures/explain_aggregate/real_limit_sort_match_sort.json");

#[test]
fn a_sort_after_a_folded_limit_raises_nothing() {
    for (name, text) in [
        ("match+limit+sort", MATCH_LIMIT_SORT),
        ("limit+sort", LIMIT_SORT),
    ] {
        let p = plan_from_text(text);
        assert!(p.hints.is_empty(), "{name}: {:?}", p.hints);
    }
}

#[test]
fn a_sort_after_a_folded_skip_sorts_what_is_left_in_memory() {
    let p = plan_from_text(SKIP_SORT);
    assert_eq!(p.hints, [AggHint::InMemorySortAfterSkip]);
}

#[test]
fn a_sort_over_a_skip_in_the_plan_tree_is_the_skip_variant() {
    let p = planned(doc! { "stage": "SORT", "inputStage": {
    "stage": "SKIP", "inputStage": { "stage": "COLLSCAN", "filter": {} } } });
    assert_eq!(p.hints, [AggHint::InMemorySortAfterSkip]);
}

// --- a $lookup's joined side is not the local plan ---

#[test]
fn a_nested_loop_on_the_joined_side_of_a_lookup_is_not_a_finding() {
    let p = planned(doc! {
        "stage": "EQ_LOOKUP",
        "strategy": "IndexedLoopJoin",
        "inputStages": [
            { "stage": "COLLSCAN", "filter": {} },
            { "stage": "EQ_LOOKUP", "strategy": "NestedLoopJoin", "inputStages": [
                { "stage": "COLLSCAN", "filter": {} },
                { "stage": "COLLSCAN", "filter": {} },
            ] },
        ],
    });
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

#[test]
fn the_joined_collections_reads_do_not_make_a_healthy_join_look_wasteful() {
    // The counters at the top add the joined collection's reads to the local side's.
    let text = LOOKUP_GROUP
        .replacen(
            "\"totalKeysExamined\": 10000",
            "\"totalKeysExamined\": 5000000",
            1,
        )
        .replacen(
            "\"totalDocsExamined\": 10000",
            "\"totalDocsExamined\": 5000000",
            1,
        );
    let p = plan_from_text(&text);
    assert_eq!(p.summary.keys_examined, Some(5_000_000), "still reported");
    assert!(
        !p.hints.contains(&AggHint::ExaminedMuchMoreThanReturned),
        "{:?}",
        p.hints
    );
}

#[test]
fn a_plain_match_and_sort_on_an_unindexed_field_still_raises_the_in_memory_sort() {
    let p = plan_from_text(MATCH_SORT);
    assert!(p.hints.contains(&AggHint::InMemorySort), "{:?}", p.hints);
}

#[test]
fn a_sort_over_a_limit_in_the_plan_tree_is_not_input_side() {
    let p = planned(doc! { "stage": "SORT", "inputStage": {
    "stage": "LIMIT", "inputStage": { "stage": "COLLSCAN", "filter": {} } } });
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}
