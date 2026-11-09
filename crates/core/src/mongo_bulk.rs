//! Bulk count / update / delete for [`crate::mongo_store::MongoStore`].
//!
//! Scope and update checks run before any server call. Writes go out as one raw `update` /
//! `delete` command whose `comment` is the run id, so a cancel can find the operation. The write
//! itself carries no time limit. Server text (`errmsg`) is never copied into a result: it can
//! quote document values. Anything that makes the result untrustworthy (driver error,
//! unreadable counts, a lost connection) is `BulkOutcome::Unknown`, because the write may have
//! happened. A reply that carries `writeErrors` or a `writeConcernError` is
//! `BulkOutcome::Rejected` with a reason keyed by the error code: the server says why, but its
//! counts do not cover the documents written before the failure, so none are reported.

use std::time::Duration;

use bson::{Bson, Document, doc};
use mongodb::{Collection, Database};

use crate::bulk::{
    BulkDeleteRequest, BulkFailureReason, BulkOutcome, BulkRunId, BulkUpdateRequest, UpdateSpec,
    check_scope, validate_update,
};
use crate::driver_error::{is_time_limit_expiry, server_error_code};
use crate::error::{CoreError, Result};
use crate::mongo_bulk_preview::write_error_code;

/// The count, or `None` when the server gave up after `max_time`.
pub(crate) async fn count_matching(
    coll: &Collection<Document>,
    filter: &Document,
    max_time: Duration,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Option<u64>> {
    match coll
        .count_documents(filter.clone())
        .max_time(max_time)
        .await
    {
        Ok(n) => Ok(Some(n)),
        Err(e) if is_time_limit_expiry(&e) => Ok(None),
        Err(e) => Err(db_err(e)),
    }
}

pub(crate) async fn update_many(
    db: &Database,
    coll_name: &str,
    req: &BulkUpdateRequest,
    run: BulkRunId,
) -> Result<BulkOutcome> {
    check_scope(&req.filter, req.scope)?;
    validate_update(&req.update)?;
    Ok(send(db, update_command(coll_name, req, run), parse_update_reply).await)
}

pub(crate) async fn delete_many(
    db: &Database,
    coll_name: &str,
    req: &BulkDeleteRequest,
    run: BulkRunId,
) -> Result<BulkOutcome> {
    check_scope(&req.filter, req.scope)?;
    Ok(send(db, delete_command(coll_name, req, run), parse_delete_reply).await)
}

/// Runs `command`; a driver error or a reply `parse` rejects is an unknown outcome.
async fn send(
    db: &Database,
    command: Document,
    parse: fn(&Document) -> Option<BulkOutcome>,
) -> BulkOutcome {
    match db.run_command(command).await {
        Ok(reply) => parse(&reply)
            .or_else(|| rejection(&reply))
            .unwrap_or(UNKNOWN),
        Err(e) => outcome_for_error_code(server_error_code(&e)),
    }
}

/// Server error code `Interrupted`: what `killOp` makes the killed write fail with (observed on
/// mongod 9.0.4 for both `update` and `delete`). A kill we did not issue (an admin's) also
/// reports `cancelled: true`; that is acceptable, the write was cancelled either way.
const CODE_INTERRUPTED: i32 = 11601;

/// A failed write is unknown; it is `cancelled` only when the server says it was killed.
pub(crate) fn outcome_for_error_code(code: Option<i32>) -> BulkOutcome {
    BulkOutcome::Unknown {
        cancelled: code == Some(CODE_INTERRUPTED),
    }
}

/// `Rejected` when an `ok: 1` reply reports `writeErrors` (the reason is its first error's code)
/// or a `writeConcernError`; `None` otherwise.
pub(crate) fn rejection(reply: &Document) -> Option<BulkOutcome> {
    reply_ok(reply).then_some(())?;
    let reason = match write_error_code(reply) {
        Some(code) => BulkFailureReason::from_code(code),
        None if reply.contains_key("writeConcernError") => BulkFailureReason::WriteConcern,
        None => return None,
    };
    Some(BulkOutcome::Rejected { reason })
}

const UNKNOWN: BulkOutcome = BulkOutcome::Unknown { cancelled: false };

fn update_command(coll_name: &str, req: &BulkUpdateRequest, run: BulkRunId) -> Document {
    doc! {
        "update": coll_name,
        "updates": [update_entry(req, req.filter.clone(), true)],
        "ordered": true,
        "comment": run.0.to_string(),
    }
}

/// The one `updates` entry for `req`, matching `filter` instead of `req.filter` (the preview
/// narrows it to a few ids). `multi` is false only for the preview of an upsert that matches
/// nothing, which runs the upsert once.
pub(crate) fn update_entry(req: &BulkUpdateRequest, filter: Document, multi: bool) -> Document {
    let update = match &req.update {
        UpdateSpec::Operators(ops) => Bson::Document(ops.clone()),
        UpdateSpec::Pipeline(stages) => {
            Bson::Array(stages.iter().cloned().map(Bson::Document).collect())
        }
    };
    doc! { "q": filter, "u": update, "multi": multi, "upsert": req.upsert }
}

fn delete_command(coll_name: &str, req: &BulkDeleteRequest, run: BulkRunId) -> Document {
    doc! {
        "delete": coll_name,
        "deletes": [{ "q": req.filter.clone(), "limit": 0 }],
        "ordered": true,
        "comment": run.0.to_string(),
    }
}

/// Counts from an `update` reply; `None` when it reports any error or cannot be read.
fn parse_update_reply(reply: &Document) -> Option<BulkOutcome> {
    reply_is_clean(reply)?;
    let upserted = upserted_ids(reply)?;
    // The server's `n` counts upserted documents too; `matched` is the pre-existing ones.
    let matched = count_field(reply, "n")?.saturating_sub(upserted.len() as u64);
    Some(BulkOutcome::Applied {
        matched,
        modified: count_field(reply, "nModified")?,
        upserted,
        deleted: 0,
    })
}

/// Counts from a `delete` reply. `matched` and `deleted` are both `n`; nothing is modified or
/// upserted.
fn parse_delete_reply(reply: &Document) -> Option<BulkOutcome> {
    reply_is_clean(reply)?;
    let n = count_field(reply, "n")?;
    Some(BulkOutcome::Applied {
        matched: n,
        modified: 0,
        upserted: Vec::new(),
        deleted: n,
    })
}

/// `Some(())` when the command succeeded (`ok: 1`) with no write or write concern errors.
pub(crate) fn reply_is_clean(reply: &Document) -> Option<()> {
    let clean = reply_ok(reply)
        && !reply.contains_key("writeErrors")
        && !reply.contains_key("writeConcernError");
    clean.then_some(())
}

fn reply_ok(reply: &Document) -> bool {
    match reply.get("ok") {
        Some(Bson::Double(v)) => *v == 1.0,
        Some(Bson::Int32(v)) => *v == 1,
        Some(Bson::Int64(v)) => *v == 1,
        _ => false,
    }
}

fn count_field(reply: &Document, key: &str) -> Option<u64> {
    match reply.get(key)? {
        Bson::Int32(n) => u64::try_from(*n).ok(),
        Bson::Int64(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// The `_id` of every upserted document; an absent `upserted` is empty, a malformed one is `None`.
pub(crate) fn upserted_ids(reply: &Document) -> Option<Vec<Bson>> {
    let Some(entries) = reply.get("upserted") else {
        return Some(Vec::new());
    };
    entries
        .as_array()?
        .iter()
        .map(|entry| entry.as_document()?.get("_id").cloned())
        .collect()
}

#[cfg(test)]
#[path = "mongo_bulk_tests.rs"]
mod tests;
