use super::*;
use bson::Bson;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::store::DuplicateGroup;

fn group(values: Vec<Bson>, count: u64) -> DuplicateGroup {
    DuplicateGroup {
        key: KeyTuple(values),
        count,
    }
}

fn report(groups: Vec<DuplicateGroup>) -> DuplicateReport {
    DuplicateReport {
        groups,
        more: false,
    }
}

#[test]
fn find_is_offered_only_after_a_duplicate_key_failure_and_never_while_loading() {
    assert!(DuplicatesView::Idle.can_find(true));
    assert!(!DuplicatesView::Idle.can_find(false));
    assert!(DuplicatesView::Failed.can_find(true));
    let loaded = DuplicatesView::Loaded {
        source: report(vec![]),
        target: report(vec![]),
    };
    assert!(loaded.can_find(true));
    for stale in [false, true] {
        assert!(!DuplicatesView::Loading { stale }.can_find(true));
    }
}

#[test]
fn rows_use_the_key_table_formatter_and_keep_the_report_order() {
    let rows = DuplicatesView::rows(&report(vec![
        group(vec![Bson::Int32(1)], 5),
        group(vec![Bson::String("a-1".into())], 3),
    ]));
    assert_eq!(
        rows,
        vec![
            DuplicateRow {
                key_text: key_label(&KeyTuple(vec![Bson::Int32(1)])),
                count: 5
            },
            DuplicateRow {
                key_text: key_label(&KeyTuple(vec![Bson::String("a-1".into())])),
                count: 3
            },
        ]
    );
    assert_eq!(rows[0].key_text, "1");
    assert_eq!(rows[1].key_text, "\"a-1\"");
}

#[test]
fn a_compound_key_shows_every_value_in_field_order() {
    let rows = DuplicatesView::rows(&report(vec![group(
        vec![Bson::Int32(7), Bson::String("x".into())],
        2,
    )]));
    assert_eq!(rows[0].key_text, "7, \"x\"");
}

#[test]
fn only_a_loaded_view_with_no_groups_on_either_side_is_an_empty_result() {
    let loaded = |source: DuplicateReport, target: DuplicateReport| DuplicatesView::Loaded {
        source,
        target,
    };
    let one = || report(vec![group(vec![Bson::Int32(1)], 2)]);
    assert!(loaded(report(vec![]), report(vec![])).is_empty_result());
    assert!(!loaded(one(), report(vec![])).is_empty_result());
    assert!(!loaded(report(vec![]), one()).is_empty_result());
    assert!(!DuplicatesView::Idle.is_empty_result());
    assert!(!DuplicatesView::Failed.is_empty_result());
    assert!(!DuplicatesView::Loading { stale: false }.is_empty_result());
}

#[test]
fn empty_reports_are_a_loaded_view() {
    let view = DuplicatesView::Loaded {
        source: report(vec![]),
        target: report(vec![]),
    };
    let DuplicatesView::Loaded { source, target } = &view else {
        unreachable!()
    };
    assert!(DuplicatesView::rows(source).is_empty());
    assert!(DuplicatesView::rows(target).is_empty());
}

#[test]
fn superseding_stales_a_request_in_flight_and_clears_everything_else() {
    assert_eq!(
        DuplicatesView::Loading { stale: false }.superseded(),
        DuplicatesView::Loading { stale: true }
    );
    assert_eq!(DuplicatesView::Failed.superseded(), DuplicatesView::Idle);
}
