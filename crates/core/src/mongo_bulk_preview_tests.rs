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
fn data_errors_of_the_update_are_an_invalid_update() {
    // 9 FailedToParse (an unknown operator such as `$sett`), 52 DollarPrefixedFieldName,
    // 15 Overflow, plus the codes the server reports for data the update cannot be applied to.
    for code in [2, 9, 14, 15, 28, 40, 52, 66, 121, 11000] {
        assert_eq!(
            classify_failure(Some(code), false, false),
            Failure::InvalidUpdate,
            "code {code}"
        );
    }
}

#[test]
fn every_other_failure_degrades_instead_of_failing() {
    let unavailable = gap(PreviewGap::TransactionsUnavailable);
    // 24 LockTimeout, 251 NoSuchTransaction, an unknown code, no code at all.
    for code in [Some(24), Some(251), Some(9999), None] {
        assert_eq!(
            classify_failure(code, false, false),
            unavailable,
            "{code:?}"
        );
    }
}

#[test]
fn a_transient_label_wins_over_a_data_error_code() {
    assert_eq!(
        classify_failure(Some(14), true, false),
        gap(PreviewGap::Conflict)
    );
}

#[test]
fn a_reply_label_is_read() {
    let labelled = doc! { "ok": 1, "errorLabels": ["TransientTransactionError"] };
    assert!(reply_is_transient(&labelled));
    assert!(!reply_is_transient(&doc! { "ok": 1 }));
    assert!(!reply_is_transient(&doc! { "errorLabels": ["Other"] }));
}

#[test]
fn a_malformed_first_batch_is_none_not_empty() {
    let good = doc! { "cursor": { "firstBatch": [{ "_id": 1 }] } };
    assert_eq!(first_batch(&good), Some(vec![doc! { "_id": 1 }]));
    assert_eq!(first_batch(&doc! { "ok": 1 }), None);
    assert_eq!(first_batch(&doc! { "cursor": { "firstBatch": 5 } }), None);
    assert_eq!(
        first_batch(&doc! { "cursor": { "firstBatch": [1, 2] } }),
        None
    );
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

#[test]
fn an_upsert_with_no_match_runs_the_upsert_once_and_a_preview_of_matches_is_multi() {
    let req = BulkUpdateRequest {
        filter: doc! { "a": 1 },
        update: crate::bulk::UpdateSpec::Operators(doc! { "$set": { "x": 1 } }),
        upsert: true,
        scope: crate::bulk::MatchScope::Filtered,
    };
    let multi = |befores: &[Document]| {
        let cmd = preview_command("c", &req, befores);
        let entry = cmd.get_array("updates").unwrap()[0]
            .as_document()
            .unwrap()
            .clone();
        entry.get_bool("multi").unwrap()
    };
    assert!(!multi(&[]), "no before: the upsert runs once");
    assert!(multi(&[doc! { "_id": 1 }]));
}

#[test]
fn the_preview_command_narrows_the_filter_to_the_previewed_ids() {
    let req = BulkUpdateRequest {
        filter: doc! { "a": 1 },
        update: crate::bulk::UpdateSpec::Operators(doc! { "$set": { "x": 1 } }),
        upsert: false,
        scope: crate::bulk::MatchScope::Filtered,
    };
    let cmd = preview_command("c", &req, &[doc! { "_id": 7 }]);
    let entry = cmd.get_array("updates").unwrap()[0].as_document().unwrap();
    assert_eq!(
        entry.get_document("q").unwrap(),
        &restricted_filter(&req.filter, vec![Bson::Int32(7)])
    );
    assert_eq!(cmd.get_bool("ordered"), Ok(true));
}
