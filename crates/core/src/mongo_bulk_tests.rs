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
