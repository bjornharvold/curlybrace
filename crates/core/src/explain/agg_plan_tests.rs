//! `parse_aggregate_explain` on hand-written explain documents shaped like MongoDB 5 to 8 output.

use bson::{Document, doc};

use super::parse_aggregate_explain;
use crate::ejson::editor_json_to_doc;
use crate::explain::{AggHint, AggPlan, AggPlanOutcome, ExplainVerbosity};

fn fixture(name: &str) -> Document {
    let text = match name {
        "stages_collscan" => {
            include_str!("../../tests/fixtures/explain_aggregate/stages_collscan.json")
        }
        "stages_ixscan" => {
            include_str!("../../tests/fixtures/explain_aggregate/stages_ixscan.json")
        }
        "stages_group_stats" => {
            include_str!("../../tests/fixtures/explain_aggregate/stages_group_stats.json")
        }
        "stages_sort_disk" => {
            include_str!("../../tests/fixtures/explain_aggregate/stages_sort_disk.json")
        }
        "stages_lookup" => {
            include_str!("../../tests/fixtures/explain_aggregate/stages_lookup.json")
        }
        "stages_facet" => include_str!("../../tests/fixtures/explain_aggregate/stages_facet.json"),
        "sharded_stages" => {
            include_str!("../../tests/fixtures/explain_aggregate/sharded_stages.json")
        }
        "toplevel_planner" => {
            include_str!("../../tests/fixtures/explain_aggregate/toplevel_planner.json")
        }
        "toplevel_stats" => {
            include_str!("../../tests/fixtures/explain_aggregate/toplevel_stats.json")
        }
        "sbe_group" => include_str!("../../tests/fixtures/explain_aggregate/sbe_group.json"),
        "sharded_winning_plan" => {
            include_str!("../../tests/fixtures/explain_aggregate/sharded_winning_plan.json")
        }
        "eq_lookup" => include_str!("../../tests/fixtures/explain_aggregate/eq_lookup.json"),
        "error_ok0" => include_str!("../../tests/fixtures/explain_aggregate/error_ok0.json"),
        "malformed" => include_str!("../../tests/fixtures/explain_aggregate/malformed.json"),
        other => panic!("unknown fixture {other}"),
    };
    editor_json_to_doc(text).expect("fixture parses")
}

fn plan(name: &str) -> AggPlan {
    match parse_aggregate_explain(&fixture(name)) {
        AggPlanOutcome::Plan(plan) => plan,
        AggPlanOutcome::NotUnderstood => panic!("{name} was not understood"),
    }
}

fn names(plan: &AggPlan) -> Vec<&str> {
    plan.stages.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn a_collection_scan_with_a_group_and_a_sort_in_the_old_stages_shape() {
    let p = plan("stages_collscan");
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.stage_kind, "COLLSCAN");
    assert_eq!(input.index, None);
    assert_eq!(input.docs_examined, None);
    assert_eq!(names(&p), ["$group", "$sort"]);
    assert_eq!(p.stages[0].detail.as_deref(), Some("total, orders"));
    assert_eq!(p.stages[1].detail.as_deref(), Some("total"));
    assert_eq!(p.hints, [AggHint::CollectionScan, AggHint::InMemorySort]);
    assert_eq!(p.summary.verbosity_seen, ExplainVerbosity::QueryPlanner);
    assert_eq!(p.summary.n_returned, None);
    assert!(p.shards.is_empty());
}

#[test]
fn an_index_scan_names_its_index_and_raises_no_hint() {
    let p = plan("stages_ixscan");
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.stage_kind, "IXSCAN");
    assert_eq!(input.index.as_deref(), Some("customer_1_created_-1"));
    assert_eq!(names(&p), ["$project"]);
    assert_eq!(p.stages[0].detail.as_deref(), Some("_id, customer, amount"));
    assert!(p.hints.is_empty());
}

#[test]
fn execution_stats_fill_the_counters_of_the_input_and_the_stages() {
    let p = plan("stages_group_stats");
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.index.as_deref(), Some("status_1"));
    assert_eq!(
        (input.keys_examined, input.docs_examined, input.n_returned),
        (Some(1200), Some(1200), Some(1200))
    );
    assert_eq!(p.stages[0].n_returned, Some(300));
    assert_eq!(p.stages[0].millis, Some(22));
    assert_eq!(p.summary.verbosity_seen, ExplainVerbosity::ExecutionStats);
    assert_eq!(p.summary.n_returned, Some(300));
    assert_eq!(p.summary.keys_examined, Some(1200));
    assert_eq!(p.summary.docs_examined, Some(1200));
    assert_eq!(p.summary.total_millis, Some(22));
    assert!(p.hints.is_empty(), "{:?}", p.hints);
}

#[test]
fn a_sort_that_used_the_disk_is_a_disk_sort_not_an_in_memory_one() {
    let p = plan("stages_sort_disk");
    assert_eq!(p.hints, [AggHint::CollectionScan, AggHint::DiskSort]);
    assert_eq!(p.summary.total_millis, Some(4100));
}

#[test]
fn a_lookup_reports_its_joined_collection_and_a_scan_of_it() {
    let p = plan("stages_lookup");
    assert_eq!(names(&p), ["$lookup", "$lookup", "$unwind", "$limit"]);
    assert_eq!(p.stages[0].detail.as_deref(), Some("customers"));
    assert_eq!(p.stages[1].detail.as_deref(), Some("shipments"));
    assert_eq!(p.stages[1].docs_examined, Some(4000));
    assert_eq!(p.stages[1].keys_examined, Some(0));
    assert_eq!(p.stages[2].detail.as_deref(), Some("$shipments"));
    assert_eq!(p.stages[3].detail.as_deref(), Some("10"));
    assert_eq!(
        p.hints,
        [AggHint::CollectionScan, AggHint::LookupCollectionScan]
    );
}

#[test]
fn a_facet_names_its_branches() {
    let p = plan("stages_facet");
    assert_eq!(names(&p), ["$facet"]);
    assert_eq!(p.stages[0].detail.as_deref(), Some("byStatus, recent"));
    assert_eq!(p.hints, [AggHint::CollectionScan]);
}

#[test]
fn a_sharded_explain_has_one_plan_per_shard_and_the_merger_stages() {
    let p = plan("sharded_stages");
    let shard_names: Vec<&str> = p.shards.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(shard_names, ["shardA", "shardB"]);
    assert_eq!(names(&p), ["$mergeCursors", "$group"]);
    let a = &p.shards[0].1;
    assert_eq!(
        a.input.as_ref().map(|i| i.stage_kind.as_str()),
        Some("IXSCAN")
    );
    assert!(a.hints.is_empty());
    let b = &p.shards[1].1;
    assert_eq!(
        b.hints,
        [
            AggHint::CollectionScan,
            AggHint::ExaminedMuchMoreThanReturned
        ]
    );
    assert_eq!(
        p.hints,
        [
            AggHint::CollectionScan,
            AggHint::ExaminedMuchMoreThanReturned
        ]
    );
    assert_eq!(p.summary.verbosity_seen, ExplainVerbosity::ExecutionStats);
    assert_eq!(p.summary.keys_examined, Some(700));
    assert_eq!(p.summary.docs_examined, Some(90_700));
    assert_eq!(p.summary.total_millis, Some(12));
    assert_eq!(p.summary.n_returned, None);
}

#[test]
fn the_newer_top_level_query_planner_has_an_input_and_no_stages() {
    let p = plan("toplevel_planner");
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.stage_kind, "IXSCAN");
    assert_eq!(input.index.as_deref(), Some("customer_1"));
    assert!(p.stages.is_empty());
    assert!(p.hints.is_empty());
    assert_eq!(p.summary.verbosity_seen, ExplainVerbosity::QueryPlanner);
}

#[test]
fn top_level_execution_stats_judge_a_blocking_sort_over_a_scan() {
    let p = plan("toplevel_stats");
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.stage_kind, "COLLSCAN");
    assert_eq!(input.docs_examined, Some(250_000));
    assert_eq!(input.n_returned, Some(12));
    assert_eq!(p.summary.total_millis, Some(380));
    assert_eq!(p.summary.n_returned, Some(12));
    assert_eq!(
        p.hints,
        [
            AggHint::CollectionScan,
            AggHint::InMemorySort,
            AggHint::ExaminedMuchMoreThanReturned
        ]
    );
}

#[test]
fn a_pushed_down_group_in_the_slot_based_engine_still_shows_its_scan() {
    let p = plan("sbe_group");
    assert_eq!(
        p.input.as_ref().map(|i| i.stage_kind.as_str()),
        Some("COLLSCAN")
    );
    assert_eq!(p.hints, [AggHint::CollectionScan]);
}

#[test]
fn a_pushed_down_lookup_keeps_the_local_side_and_ignores_the_joined_index() {
    let p = plan("eq_lookup");
    let input = p.input.as_ref().expect("input");
    assert_eq!(input.stage_kind, "COLLSCAN");
    assert_eq!(
        input.index, None,
        "the _id_ index belongs to the joined collection"
    );
    assert_eq!(input.plan_stages, ["EQ_LOOKUP", "COLLSCAN"]);
    assert!(p.stages.is_empty());
    assert_eq!(p.hints, [AggHint::CollectionScan]);
}

#[test]
fn a_pushed_down_lookup_without_an_index_scans_the_joined_collection() {
    let mut explain = fixture("eq_lookup");
    let text = explain.to_string().replace("IndexedLoopJoin", "HashJoin");
    explain = editor_json_to_doc(&text).expect("still parses");
    let AggPlanOutcome::Plan(p) = parse_aggregate_explain(&explain) else {
        panic!("expected a plan");
    };
    assert_eq!(
        p.hints,
        [AggHint::CollectionScan, AggHint::LookupCollectionScan]
    );
}

#[test]
fn the_plan_stages_list_what_the_server_pushed_down() {
    let p = plan("sbe_group");
    let input = p.input.as_ref().expect("input");
    assert_eq!(
        input.plan_stages,
        ["GROUP", "PROJECTION_SIMPLE", "COLLSCAN"]
    );
}

#[test]
fn a_sharded_query_planner_form_lists_each_shard_with_its_own_figures() {
    let p = plan("sharded_winning_plan");
    assert!(p.input.is_none());
    let shard_names: Vec<&str> = p.shards.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(shard_names, ["shardA", "shardB"]);
    let a = &p.shards[0].1;
    let a_input = a.input.as_ref().expect("input");
    assert_eq!(a_input.index.as_deref(), Some("status_1"));
    assert_eq!(
        (
            a_input.keys_examined,
            a_input.docs_examined,
            a_input.n_returned
        ),
        (Some(700), Some(700), Some(400))
    );
    assert!(a.hints.is_empty());
    let b = &p.shards[1].1;
    assert_eq!(
        b.hints,
        [
            AggHint::CollectionScan,
            AggHint::ExaminedMuchMoreThanReturned
        ]
    );
    assert_eq!(p.summary.n_returned, Some(900));
    assert_eq!(p.summary.total_millis, Some(25));
    assert_eq!(p.summary.docs_examined, Some(40_700));
    assert_eq!(
        p.hints,
        [
            AggHint::CollectionScan,
            AggHint::ExaminedMuchMoreThanReturned
        ]
    );
}

#[test]
fn an_error_document_is_not_understood() {
    assert_eq!(
        parse_aggregate_explain(&fixture("error_ok0")),
        AggPlanOutcome::NotUnderstood
    );
}

#[test]
fn unknown_and_degenerate_shapes_are_not_understood_and_never_panic() {
    let shapes = [
        doc! {},
        doc! { "foo": 1 },
        doc! { "stages": [] },
        doc! { "stages": "x" },
        doc! { "stages": [1, "a", { "b": 2 }] },
        doc! { "stages": [ { "$cursor": 5 } ] },
        doc! { "stages": [ { "$cursor": { "queryPlanner": 5 } } ] },
        doc! { "queryPlanner": 5 },
        doc! { "queryPlanner": {} },
        doc! { "queryPlanner": { "winningPlan": {} } },
        doc! { "queryPlanner": { "winningPlan": { "stage": 5 } } },
        doc! { "queryPlanner": { "winningPlan": { "shards": [] } } },
        doc! { "queryPlanner": { "winningPlan": { "shards": [1, "x"] } } },
        doc! { "shards": {} },
        doc! { "shards": { "a": 5, "b": { "x": 1 } } },
        doc! { "shards": [ { "stages": [] } ] },
        doc! { "ok": 0, "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } } },
        doc! { "ok": 0.0, "stages": [ { "$match": {} } ] },
    ];
    for shape in shapes {
        assert_eq!(
            parse_aggregate_explain(&shape),
            AggPlanOutcome::NotUnderstood,
            "{shape}"
        );
    }
}

#[test]
fn a_document_with_odd_values_degrades_field_by_field() {
    let p = plan("malformed");
    assert!(p.input.is_none(), "a string winning plan is no plan");
    assert_eq!(names(&p), ["$limit"]);
    let stage = &p.stages[0];
    assert_eq!(stage.detail, None);
    assert_eq!(stage.n_returned, None);
    assert_eq!(stage.millis, None);
    assert_eq!(p.summary.total_millis, None);
}

#[test]
fn counters_read_in_any_numeric_width_and_negative_ones_read_as_missing() {
    let explain = doc! {
        "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } },
        "executionStats": {
            "nReturned": 3_i32,
            "totalDocsExamined": 12.0_f64,
            "totalKeysExamined": -1_i64,
            "executionTimeMillis": 5_i64,
        },
    };
    let AggPlanOutcome::Plan(p) = parse_aggregate_explain(&explain) else {
        panic!("expected a plan");
    };
    let input = p.input.expect("input");
    assert_eq!(input.n_returned, Some(3));
    assert_eq!(input.docs_examined, Some(12));
    assert_eq!(input.keys_examined, None);
    assert_eq!(p.summary.total_millis, Some(5));
}

#[test]
fn a_stage_detail_holds_field_names_never_values() {
    let explain = doc! {
        "stages": [
            { "$cursor": { "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } } } },
            { "$match": { "email": "secret@example.com", "$or": [ { "a": 1 } ] } },
            { "$group": { "_id": "$x", "sum": { "$sum": "secret-total" } } },
            { "$unionWith": { "coll": "archive", "pipeline": [] } },
            { "$count": "n" },
        ],
    };
    let AggPlanOutcome::Plan(p) = parse_aggregate_explain(&explain) else {
        panic!("expected a plan");
    };
    let details: Vec<Option<&str>> = p.stages.iter().map(|s| s.detail.as_deref()).collect();
    assert_eq!(
        details,
        [Some("email, $or"), Some("sum"), Some("archive"), Some("n")]
    );
}

#[test]
fn hint_keys_are_stable_snake_case_codes() {
    let keys: Vec<&str> = [
        AggHint::CollectionScan,
        AggHint::InMemorySort,
        AggHint::DiskSort,
        AggHint::ExaminedMuchMoreThanReturned,
        AggHint::LookupCollectionScan,
    ]
    .iter()
    .map(|h| h.key())
    .collect();
    assert_eq!(
        keys,
        [
            "collection_scan",
            "in_memory_sort",
            "disk_sort",
            "examined_much_more_than_returned",
            "lookup_collection_scan"
        ]
    );
}
