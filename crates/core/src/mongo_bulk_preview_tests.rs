use super::*;

fn gap(g: PreviewGap) -> Failure {
    Failure::Gap(g)
}

#[test]
fn a_standalone_server_and_capped_collections_degrade() {
    // 20 IllegalOperation: "Transaction numbers are only allowed on a replica set member".
    assert_eq!(
        classify_failure(Some(20), false, false),
        gap(PreviewGap::TransactionsUnavailable)
    );
    for code in [115, 263, 303] {
        assert_eq!(
            classify_failure(Some(code), false, false),
            gap(PreviewGap::TransactionsUnavailable),
            "code {code}"
        );
    }
}

#[test]
fn the_driver_refusing_transactions_degrades() {
    assert_eq!(
        classify_failure(None, false, true),
        gap(PreviewGap::TransactionsUnavailable)
    );
}

#[test]
fn a_write_conflict_or_transient_label_is_a_conflict() {
    assert_eq!(
        classify_failure(Some(112), false, false),
        gap(PreviewGap::Conflict)
    );
    assert_eq!(
        classify_failure(None, true, false),
        gap(PreviewGap::Conflict)
    );
    assert_eq!(
        classify_failure(Some(251), true, false),
        gap(PreviewGap::Conflict)
    );
}

#[test]
fn any_other_failure_is_not_a_gap() {
    // 14 TypeMismatch: `$inc` on a string.
    assert_eq!(classify_failure(Some(14), false, false), Failure::Other);
    assert_eq!(classify_failure(None, false, false), Failure::Other);
}

#[test]
fn write_errors_yield_the_first_code() {
    let reply = doc! { "ok": 1, "writeErrors": [{ "index": 0, "code": 112, "errmsg": "x" }] };
    assert_eq!(write_error_code(&reply), Some(Some(112)));
    assert_eq!(write_error_code(&doc! { "ok": 1, "n": 1 }), None);
    let odd = doc! { "writeErrors": [{ "index": 0 }] };
    assert_eq!(write_error_code(&odd), Some(None));
}

#[test]
fn the_filter_is_narrowed_to_the_ids_with_and() {
    let got = restricted_filter(&doc! { "g": "a" }, vec![Bson::Int32(1), Bson::Int32(2)]);
    assert_eq!(
        got,
        doc! { "$and": [{ "_id": { "$in": [1, 2] } }, { "g": "a" }] }
    );
}

#[test]
fn pairs_follow_the_before_order_and_upserts_have_an_empty_before() {
    let befores = vec![doc! { "_id": 2, "v": 1 }, doc! { "_id": 1, "v": 1 }];
    let afters = vec![doc! { "_id": 1, "v": 2 }, doc! { "_id": 2, "v": 2 }];
    let got = pair_up(befores, afters);
    assert_eq!(got.gap, None);
    assert_eq!(got.docs[0].after, Some(doc! { "_id": 2, "v": 2 }));
    assert_eq!(got.docs[1].after, Some(doc! { "_id": 1, "v": 2 }));

    let upsert = pair_up(vec![], vec![doc! { "_id": 9 }]);
    assert!(upsert.docs[0].before.is_empty());
    assert_eq!(upsert.docs[0].after, Some(doc! { "_id": 9 }));
}

#[test]
fn before_only_keeps_every_before_and_no_after() {
    let got = before_only(vec![doc! { "_id": 1 }], PreviewGap::Conflict);
    assert_eq!(got.gap, Some(PreviewGap::Conflict));
    assert_eq!(got.docs[0].after, None);
}
