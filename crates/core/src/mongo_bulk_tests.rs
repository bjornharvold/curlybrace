use super::*;
use crate::bulk::MatchScope;

fn run() -> BulkRunId {
    BulkRunId(uuid::Uuid::new_v4())
}

fn update_req(update: UpdateSpec, upsert: bool) -> BulkUpdateRequest {
    BulkUpdateRequest {
        filter: doc! { "a": 1 },
        update,
        upsert,
        scope: MatchScope::Filtered,
    }
}

#[test]
fn an_update_command_is_multi_with_the_run_id_as_comment_and_no_max_time() {
    let id = run();
    let req = update_req(UpdateSpec::Operators(doc! { "$set": { "x": 1 } }), true);
    let cmd = update_command("c", &req, id);
    let op = cmd.get_array("updates").unwrap()[0].as_document().unwrap();
    assert_eq!(op.get_bool("multi"), Ok(true));
    assert_eq!(op.get_bool("upsert"), Ok(true));
    assert_eq!(op.get_document("u"), Ok(&doc! { "$set": { "x": 1 } }));
    assert_eq!(cmd.get_str("comment").unwrap(), id.0.to_string());
    assert!(!cmd.contains_key("maxTimeMS"));
}

#[test]
fn a_pipeline_update_is_sent_as_an_array() {
    let req = update_req(
        UpdateSpec::Pipeline(vec![doc! { "$set": { "x": 1 } }]),
        false,
    );
    let cmd = update_command("c", &req, run());
    let op = cmd.get_array("updates").unwrap()[0].as_document().unwrap();
    assert_eq!(op.get_array("u").unwrap().len(), 1);
    assert_eq!(op.get_bool("upsert"), Ok(false));
}

#[test]
fn a_delete_command_has_no_limit_and_the_run_id() {
    let id = run();
    let req = BulkDeleteRequest {
        filter: doc! { "a": 1 },
        scope: MatchScope::Filtered,
    };
    let cmd = delete_command("c", &req, id);
    let op = cmd.get_array("deletes").unwrap()[0].as_document().unwrap();
    assert_eq!(op.get_i32("limit"), Ok(0));
    assert_eq!(cmd.get_str("comment").unwrap(), id.0.to_string());
}

#[test]
fn an_update_reply_gives_counts_and_upserted_ids() {
    let reply = doc! { "ok": 1.0, "n": 3, "nModified": 2,
    "upserted": [{ "index": 0, "_id": 7 }] };
    assert_eq!(
        parse_update_reply(&reply),
        Some(BulkOutcome::Applied {
            matched: 2,
            modified: 2,
            upserted: vec![Bson::Int32(7)],
            deleted: 0
        })
    );
}

#[test]
fn a_delete_reply_counts_deleted_and_matched_alike() {
    let reply = doc! { "ok": 1.0, "n": 4_i64 };
    assert_eq!(
        parse_delete_reply(&reply),
        Some(BulkOutcome::Applied {
            matched: 4,
            modified: 0,
            upserted: vec![],
            deleted: 4
        })
    );
}

#[test]
fn replies_that_cannot_be_trusted_are_rejected_without_server_text() {
    let write_errors = doc! { "ok": 1.0, "n": 1, "nModified": 1,
    "writeErrors": [{ "index": 0, "code": 11000, "errmsg": "SECRET" }] };
    let concern = doc! { "ok": 1.0, "n": 1, "nModified": 1,
    "writeConcernError": { "code": 64, "errmsg": "SECRET" } };
    let not_ok = doc! { "ok": 0.0, "n": 1, "nModified": 1 };
    let no_counts = doc! { "ok": 1.0 };
    let bad_upsert = doc! { "ok": 1.0, "n": 1, "nModified": 0, "upserted": ["junk"] };
    for reply in [&write_errors, &concern, &not_ok, &no_counts, &bad_upsert] {
        assert_eq!(parse_update_reply(reply), None);
    }
    // A delete reply has no `upserted`, so only the others apply.
    for reply in [&write_errors, &concern, &not_ok, &no_counts] {
        assert_eq!(parse_delete_reply(reply), None);
    }
    assert_eq!(UNKNOWN, BulkOutcome::Unknown { cancelled: false });
}

#[test]
fn only_the_kill_code_makes_a_failed_write_cancelled() {
    assert_eq!(
        outcome_for_error_code(Some(11601)),
        BulkOutcome::Unknown { cancelled: true }
    );
    for code in [None, Some(0), Some(11600), Some(13), Some(50), Some(2)] {
        assert_eq!(
            outcome_for_error_code(code),
            BulkOutcome::Unknown { cancelled: false },
            "{code:?}"
        );
    }
}

#[test]
fn a_cancel_that_is_not_permitted_or_supported_is_unsupported() {
    use crate::mongo_bulk_cancel::cancel_failure;
    for code in [13, 59, 115, 40324, 8000] {
        assert!(matches!(
            cancel_failure(Some(code)),
            CoreError::Unsupported(_)
        ));
    }
    assert!(matches!(cancel_failure(Some(2)), CoreError::Database(_)));
    assert!(matches!(cancel_failure(None), CoreError::Database(_)));
}

#[test]
fn the_current_op_lookup_is_pinned_to_the_primary() {
    use mongodb::options::SelectionCriteria;
    assert!(matches!(
        crate::mongo_bulk_cancel::primary(),
        SelectionCriteria::ReadPreference(mongodb::options::ReadPreference::Primary)
    ));
}

#[test]
fn a_reply_with_write_errors_is_rejected_with_a_code_keyed_reason_and_no_counts() {
    // Observed on mongod 9.0.4: n and nModified read 0 although earlier documents were written.
    let reply = |code: i32| {
        doc! { "ok": 1.0, "n": 0, "nModified": 0,
        "writeErrors": [{ "index": 0, "code": code, "errmsg": "SECRET" }] }
    };
    for (code, reason) in [
        (11000, BulkFailureReason::DuplicateKey),
        (121, BulkFailureReason::ValidationFailed),
        (66, BulkFailureReason::ImmutableField),
        (14, BulkFailureReason::TypeMismatch),
        (28, BulkFailureReason::Other),
    ] {
        assert_eq!(
            rejection(&reply(code)),
            Some(BulkOutcome::Rejected { reason })
        );
    }
}

#[test]
fn a_write_concern_error_is_rejected_as_write_concern() {
    let reply =
        doc! { "ok": 1.0, "n": 5, "writeConcernError": { "code": 100, "errmsg": "SECRET" } };
    assert_eq!(
        rejection(&reply),
        Some(BulkOutcome::Rejected {
            reason: BulkFailureReason::WriteConcern
        })
    );
}

#[test]
fn write_errors_win_over_a_write_concern_error() {
    let reply = doc! { "ok": 1.0, "writeErrors": [{ "index": 0, "code": 11000 }],
    "writeConcernError": { "code": 100 } };
    assert_eq!(
        rejection(&reply),
        Some(BulkOutcome::Rejected {
            reason: BulkFailureReason::DuplicateKey
        })
    );
}

#[test]
fn a_clean_or_unreadable_reply_is_not_a_rejection() {
    assert_eq!(rejection(&doc! { "ok": 1.0, "n": 1, "nModified": 1 }), None);
    assert_eq!(
        rejection(&doc! { "ok": 0.0, "writeErrors": [{ "code": 11000 }] }),
        None
    );
}

#[test]
fn an_unreadable_write_error_is_rejected_as_other() {
    let reply = doc! { "ok": 1.0, "writeErrors": ["junk"] };
    assert_eq!(
        rejection(&reply),
        Some(BulkOutcome::Rejected {
            reason: BulkFailureReason::Other
        })
    );
}
