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

fn loading(stale: bool) -> DuplicatesView {
    DuplicatesView::Loading {
        entry: 3,
        pair: 9,
        stale,
    }
}

#[test]
fn the_panel_remembers_which_entry_it_belongs_to() {
    assert_eq!(DuplicatesView::Idle.entry(), None);
    assert_eq!(loading(false).entry(), Some(3));
    assert_eq!(DuplicatesView::Failed { entry: 4 }.entry(), Some(4));
    let loaded = DuplicatesView::Loaded {
        entry: 5,
        source: report(vec![]),
        target: report(vec![]),
    };
    assert_eq!(loaded.entry(), Some(5));
    assert_eq!(loading(false).superseded().entry(), Some(3));
}

#[test]
fn find_is_offered_only_after_a_duplicate_key_failure_and_never_while_loading() {
    assert!(DuplicatesView::Idle.can_find(true));
    assert!(!DuplicatesView::Idle.can_find(false));
    assert!(DuplicatesView::Failed { entry: 3 }.can_find(true));
    let loaded = DuplicatesView::Loaded {
        entry: 3,
        source: report(vec![]),
        target: report(vec![]),
    };
    assert!(loaded.can_find(true));
    for stale in [false, true] {
        assert!(!loading(stale).can_find(true));
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
        entry: 3,
        source,
        target,
    };
    let one = || report(vec![group(vec![Bson::Int32(1)], 2)]);
    assert!(loaded(report(vec![]), report(vec![])).is_empty_result());
    assert!(!loaded(one(), report(vec![])).is_empty_result());
    assert!(!loaded(report(vec![]), one()).is_empty_result());
    assert!(!DuplicatesView::Idle.is_empty_result());
    assert!(!DuplicatesView::Failed { entry: 3 }.is_empty_result());
    assert!(!loading(false).is_empty_result());
}

#[test]
fn empty_reports_are_a_loaded_view() {
    let view = DuplicatesView::Loaded {
        entry: 3,
        source: report(vec![]),
        target: report(vec![]),
    };
    let DuplicatesView::Loaded { source, target, .. } = &view else {
        unreachable!()
    };
    assert!(DuplicatesView::rows(source).is_empty());
    assert!(DuplicatesView::rows(target).is_empty());
}

#[test]
fn superseding_stales_a_request_in_flight_and_clears_everything_else() {
    assert_eq!(loading(false).superseded(), loading(true));
    assert_eq!(
        DuplicatesView::Failed { entry: 3 }.superseded(),
        DuplicatesView::Idle
    );
}
