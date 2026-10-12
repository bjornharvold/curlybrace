use super::*;
use curlybrace_core::store::IndexStatus;

fn pre(sc: u64, tc: u64, si: IndexStatus, ti: IndexStatus, too_large: bool) -> Preflight {
    Preflight {
        source_count: sc,
        target_count: tc,
        source_index: si,
        target_index: ti,
        unindexed_too_large: too_large,
    }
}

#[test]
fn indexed_sides_are_ok_and_allow_start() {
    let v = PreflightView::from(&pre(
        5,
        7,
        IndexStatus::Indexed,
        IndexStatus::Indexed,
        false,
    ));
    assert_eq!(v, PreflightView::Ok);
    assert!(v.allows_start());
    assert_eq!(v.message(), None);
}

#[test]
fn a_large_unindexed_side_needs_an_index_and_blocks_start() {
    let v = PreflightView::from(&pre(
        5,
        9_000_000,
        IndexStatus::Indexed,
        IndexStatus::NotIndexed,
        true,
    ));
    assert_eq!(v, PreflightView::NeedsIndex { side: Side::Target });
    assert!(!v.allows_start());
    let m = v.message().unwrap();
    assert_eq!(m.key, "compare.preflight.needs_index");
    assert_eq!(m.side_key, Some("compare.side.target"));
}

#[test]
fn both_unindexed_names_the_larger_side() {
    let v = PreflightView::from(&pre(
        10,
        3,
        IndexStatus::NotIndexed,
        IndexStatus::NotIndexed,
        true,
    ));
    assert_eq!(v, PreflightView::NeedsIndex { side: Side::Source });
}

#[test]
fn two_empty_sides_are_reported_but_may_still_run() {
    let v = PreflightView::from(&pre(
        0,
        0,
        IndexStatus::Indexed,
        IndexStatus::Indexed,
        false,
    ));
    assert_eq!(v, PreflightView::EmptyBoth);
    assert!(v.allows_start());
    assert_eq!(v.message().unwrap().key, "compare.preflight.empty_both");
}

#[test]
fn unchecked_and_checking_do_not_allow_start() {
    assert!(!PreflightView::Unchecked.allows_start());
    assert!(!PreflightView::Checking.allows_start());
}
