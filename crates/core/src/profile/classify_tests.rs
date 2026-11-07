use super::classify_entry;
use super::entry_tests::entry;
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
fn a_getmore_collscan_is_judged_by_its_plan() {
    assert_eq!(
        classify_entry(&entry("getmore"), true),
        VerdictKind::CollectionScan
    );
}
