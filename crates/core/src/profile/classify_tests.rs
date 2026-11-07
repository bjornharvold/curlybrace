use super::entry_tests::entry;
use super::{ProfileEntry, classify_entry};
use crate::explain::VerdictKind;

#[test]
fn collscan_is_collection_scan() {
    assert_eq!(
        classify_entry(&entry("collscan"), true),
        VerdictKind::CollectionScan
    );
}

#[test]
fn empty_filter_collscan_is_no_filter() {
    assert_eq!(
        classify_entry(&entry("collscan"), false),
        VerdictKind::NoFilter
    );
}

#[test]
fn a_collscan_that_read_nothing_is_an_empty_collection() {
    let mut e = entry("collscan");
    e.docs_examined = 0;
    e.returned = 0;
    assert_eq!(classify_entry(&e, true), VerdictKind::EmptyCollection);
    assert_eq!(classify_entry(&e, false), VerdictKind::EmptyCollection);
}

#[test]
fn collscan_with_sort_stage_is_sort_not_covered() {
    let mut e = entry("collscan");
    e.has_sort_stage = true;
    assert_eq!(classify_entry(&e, true), VerdictKind::SortNotCovered);
    // Release A: an unfiltered scan that also sorts is the sort's problem, not "no filter".
    assert_eq!(classify_entry(&e, false), VerdictKind::SortNotCovered);
}

#[test]
fn sort_stage_is_sort_not_covered() {
    assert_eq!(
        classify_entry(&entry("sort"), true),
        VerdictKind::SortNotCovered
    );
}

#[test]
fn ixscan_with_many_keys_is_inefficient() {
    assert_eq!(
        classify_entry(&entry("inefficient"), true),
        VerdictKind::InefficientIndex
    );
}

#[test]
fn inefficiency_thresholds_match_release_a() {
    let mut e = entry("inefficient");
    e.keys_examined = 99;
    e.docs_examined = 99;
    e.returned = 0;
    assert_eq!(classify_entry(&e, true), VerdictKind::IndexUsed);
    e.keys_examined = 100;
    e.docs_examined = 100;
    assert_eq!(classify_entry(&e, true), VerdictKind::InefficientIndex);
    e.returned = 10;
    assert_eq!(classify_entry(&e, true), VerdictKind::InefficientIndex);
    e.returned = 11;
    assert_eq!(classify_entry(&e, true), VerdictKind::IndexUsed);
}

#[test]
fn good_ixscan_is_index_used() {
    assert_eq!(
        classify_entry(&entry("ixscan"), true),
        VerdictKind::IndexUsed
    );
}

#[test]
fn idhack_is_judged_like_any_index_plan() {
    let mut e = entry("ixscan");
    e.plan_summary = "IDHACK".into();
    assert_eq!(classify_entry(&e, true), VerdictKind::IndexUsed);
}

#[test]
fn ops_without_a_plan_summary_are_neutral() {
    assert_eq!(
        classify_entry(&entry("insert"), false),
        VerdictKind::NoFilter
    );
    assert_eq!(
        classify_entry(&entry("index_stats_aggregation"), false),
        VerdictKind::NoFilter
    );
}

#[test]
fn classify_entry_is_neutral_for_a_continuation() {
    // The poller skips continuations; if one slips through it must never read as a miss
    // (nor as an empty collection), whatever its own plan says.
    for has_filter in [true, false] {
        assert_eq!(
            classify_entry(&entry("getmore"), has_filter),
            VerdictKind::NoFilter
        );
    }
    let mut e = entry("getmore");
    e.docs_examined = 0;
    assert_eq!(classify_entry(&e, true), VerdictKind::NoFilter);
}

fn with_plan(plan: &str, keys: u64, docs: u64, returned: u64) -> ProfileEntry {
    let mut e = entry("ixscan");
    e.plan_summary = plan.into();
    e.keys_examined = keys;
    e.docs_examined = docs;
    e.returned = returned;
    e
}

#[test]
fn collscan_anywhere_in_the_summary_is_a_collection_scan() {
    for plan in [
        "COLLSCAN { direction: 1 }",
        "COLLSCAN",
        "IXSCAN { a: 1 }, COLLSCAN",
        "COLLSCAN, IXSCAN { a: 1 }",
    ] {
        let e = with_plan(plan, 0, 500, 5);
        assert_eq!(
            classify_entry(&e, true),
            VerdictKind::CollectionScan,
            "{plan}"
        );
        assert_eq!(classify_entry(&e, false), VerdictKind::NoFilter, "{plan}");
        let none = with_plan(plan, 0, 0, 0);
        assert_eq!(
            classify_entry(&none, true),
            VerdictKind::EmptyCollection,
            "{plan}"
        );
        let mut sorted = with_plan(plan, 0, 500, 5);
        sorted.has_sort_stage = true;
        assert_eq!(
            classify_entry(&sorted, false),
            VerdictKind::SortNotCovered,
            "{plan}"
        );
    }
}

#[test]
fn a_stage_that_only_contains_the_word_is_not_a_collscan() {
    let e = with_plan("NOT_A_COLLSCAN_AT_ALL", 1, 1, 1);
    assert_eq!(classify_entry(&e, true), VerdictKind::IndexUsed);
}

#[test]
fn other_plan_summaries() {
    // EOF: the collection does not exist or is empty, as in release A.
    assert_eq!(
        classify_entry(&with_plan("EOF", 0, 0, 0), true),
        VerdictKind::EmptyCollection
    );
    // Index-ish plans are IndexUsed unless they examined far more than they returned.
    for plan in [
        "IDHACK",
        "COUNT_SCAN { a: 1 }",
        "DISTINCT_SCAN { a: 1 }",
        "EXPRESS_IXSCAN { a: 1 }",
    ] {
        assert_eq!(
            classify_entry(&with_plan(plan, 2, 2, 2), true),
            VerdictKind::IndexUsed,
            "{plan}"
        );
        assert_eq!(
            classify_entry(&with_plan(plan, 900, 900, 3), true),
            VerdictKind::InefficientIndex,
            "{plan}"
        );
    }
    // Unknown or non-index summaries show no evidence of a miss: IndexUsed (still size-checked).
    assert_eq!(
        classify_entry(&with_plan("MULTI_ITERATOR", 0, 3, 3), true),
        VerdictKind::IndexUsed
    );
}
