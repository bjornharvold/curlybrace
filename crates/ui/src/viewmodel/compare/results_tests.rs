use super::*;
use bson::Bson;
use curlybrace_core::compare::engine::Summary;

fn key(n: i32) -> KeyTuple {
    KeyTuple(vec![Bson::Int32(n)])
}

fn rows(from: i32, n: i32) -> Vec<RowView> {
    (from..from + n)
        .map(|i| RowView {
            key: key(i),
            has_source: true,
            has_target: true,
        })
        .collect()
}

fn summary(src: u64, tgt: u64, diff: u64) -> Summary {
    Summary {
        only_in_source: src,
        only_in_target: tgt,
        different: diff,
        ..Summary::default()
    }
}

fn loaded(s: Summary) -> ResultsView {
    let v = ResultsView::new(&s);
    let (cat, off) = v.requested.expect("a first page is requested");
    let total = v.counts.of(cat);
    v.accept_rows(cat, off, total, rows(0, total.min(PAGE_SIZE) as i32))
}

#[test]
fn counts_come_from_the_summary_and_the_first_non_empty_category_is_current() {
    let v = ResultsView::new(&summary(3, 0, 0));
    assert_eq!(v.counts.only_in_source, 3);
    assert_eq!(v.counts.only_in_target, 0);
    assert_eq!(v.counts.different, 0);
    assert_eq!(v.category, Category::OnlyInSource);
    assert_eq!(v.requested, Some((Category::OnlyInSource, 0)));
    let both = ResultsView::new(&summary(3, 0, 2));
    assert_eq!(both.category, Category::Different);
}

#[test]
fn an_empty_result_requests_nothing() {
    let v = ResultsView::new(&summary(0, 0, 0));
    assert_eq!(v.requested, None);
    assert!(v.request(1).is_none());
}

#[test]
fn page_steps_by_one_hundred_and_clamps_inside_the_total() {
    let v = loaded(summary(0, 0, 250));
    assert_eq!(PAGE_SIZE, 100);
    assert_eq!(v.paged(-1), None, "already on the first page");
    let v = v.paged(1).unwrap();
    assert_eq!(v.offset, 100);
    let v = v.paged(1).unwrap();
    assert_eq!(v.offset, 200);
    assert!(v.paged(1).is_none(), "no page after the last");
    assert_eq!(v.paged(-1).unwrap().offset, 100);
    assert_eq!(v.paged(-99).unwrap().offset, 0);
    assert_eq!(v.paged(99), None);
    assert_eq!(v.paged(i64::MAX), None);
    assert_eq!(v.paged(i64::MIN).unwrap().offset, 0);
}

#[test]
fn a_page_change_requests_rows_at_the_new_offset() {
    let v = loaded(summary(0, 0, 250)).paged(1).unwrap();
    match v.request(7).unwrap() {
        CompareCommand::Rows {
            session,
            pair,
            category,
            offset,
            limit,
        } => assert_eq!(
            (session, pair, category, offset, limit),
            (7, PAIR_ID, Category::Different, 100, 100)
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
fn switching_category_resets_the_offset_and_requests_its_first_page() {
    let v = loaded(summary(5, 0, 250)).paged(1).unwrap();
    assert!(v.in_category(Category::Different).is_none());
    let v = v.in_category(Category::OnlyInSource).unwrap();
    assert_eq!((v.category, v.offset), (Category::OnlyInSource, 0));
    assert_eq!(v.requested, Some((Category::OnlyInSource, 0)));
    assert!(
        v.rows.is_empty(),
        "old rows are not shown under the new tab"
    );
}

#[test]
fn select_returns_a_detail_command_for_a_listed_key_only() {
    let v = loaded(summary(0, 0, 3));
    match v.select(2, &key(1)).unwrap() {
        CompareCommand::Detail {
            session,
            pair,
            key: k,
        } => {
            assert_eq!((session, pair, k), (2, PAIR_ID, key(1)));
        }
        other => panic!("{other:?}"),
    }
    assert!(v.select(2, &key(99)).is_none());
}

#[test]
fn rows_beyond_the_total_are_ignored() {
    let v = ResultsView::new(&summary(0, 0, 2));
    let v = v.accept_rows(Category::Different, 0, 2, rows(0, 5));
    assert_eq!(v.rows.len(), 2);
}

#[test]
fn stale_rows_are_dropped() {
    let fresh = ResultsView::new(&summary(0, 0, 250));
    let wrong_offset = fresh
        .clone()
        .accept_rows(Category::Different, 100, 250, rows(0, 3));
    assert_eq!(wrong_offset, fresh);
    let wrong_category = fresh
        .clone()
        .accept_rows(Category::OnlyInTarget, 0, 250, rows(0, 3));
    assert_eq!(wrong_category, fresh);
    let accepted = fresh.accept_rows(Category::Different, 0, 250, rows(0, 3));
    assert_eq!(accepted.rows.len(), 3);
    assert_eq!(accepted.requested, None);
    let again = accepted
        .clone()
        .accept_rows(Category::Different, 0, 250, rows(0, 1));
    assert_eq!(
        again, accepted,
        "a duplicate answer is not requested any more"
    );
}

#[test]
fn key_labels_are_compact_and_join_fields() {
    let k = KeyTuple(vec![Bson::String("a".into()), Bson::Int32(2)]);
    assert_eq!(key_label(&k), "\"a\", 2");
}

#[test]
fn pager_state_follows_the_offset_and_the_total() {
    let v = loaded(summary(0, 0, 250));
    assert!(!v.has_prev() && v.has_next());
    assert_eq!(v.shown_range(), (1, 100));
    let v = v
        .paged(1)
        .unwrap()
        .accept_rows(Category::Different, 100, 250, rows(100, 100));
    assert!(v.has_prev() && v.has_next());
    assert_eq!(v.shown_range(), (101, 200));
    let v = v
        .paged(1)
        .unwrap()
        .accept_rows(Category::Different, 200, 250, rows(200, 50));
    assert!(v.has_prev() && !v.has_next());
    assert_eq!(v.shown_range(), (201, 250));
}

#[test]
fn an_empty_category_has_no_pager_and_an_empty_range() {
    let v = ResultsView::new(&summary(0, 0, 0));
    assert!(!v.has_prev() && !v.has_next());
    assert_eq!(v.shown_range(), (0, 0));
    assert!(v.is_clean());
    assert!(!loaded(summary(1, 0, 0)).is_clean());
}

#[test]
fn identical_documents_are_counted_but_do_not_make_a_difference() {
    let s = Summary {
        equal: 42,
        ..summary(0, 0, 0)
    };
    let v = ResultsView::new(&s);
    assert_eq!(v.counts.identical, 42);
    assert!(v.is_clean(), "identical documents are not differences");
    assert_eq!(v.requested, None);
    let mixed = ResultsView::new(&Summary {
        equal: 5,
        ..summary(0, 0, 2)
    });
    assert_eq!((mixed.counts.identical, mixed.total()), (5, 2));
    assert!(!mixed.is_clean());
}

#[test]
fn a_failed_page_keeps_its_request_until_retried_or_answered() {
    let v = ResultsView::new(&summary(0, 0, 3)).rows_failed();
    assert!(v.load_failed && v.requested == Some((Category::Different, 0)));
    assert!(!v.is_loading(), "a failed page is not loading any more");
    let retried = v.retried().expect("a failed page can be retried");
    assert!(!retried.load_failed && retried.is_loading());
    assert_eq!(retried.requested, v.requested);
    assert!(retried.request(1).is_some());
    assert!(
        retried.retried().is_none(),
        "nothing failed: nothing to retry"
    );
    let answered = v.clone().accept_rows(Category::Different, 0, 3, rows(0, 3));
    assert!(!answered.load_failed);
    let moved = v.in_category(Category::OnlyInSource).unwrap();
    assert!(!moved.load_failed);
    // Without an outstanding request nothing is marked failed.
    let idle = loaded(summary(0, 0, 3)).rows_failed();
    assert!(!idle.load_failed);
}
