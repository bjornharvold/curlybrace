//! Bulk count / update / delete for [`crate::mongo_store::MongoStore`].
//!
//! Scope and update checks run before any server call. Writes go out as one raw `update` /
//! `delete` command whose `comment` is the run id, so a cancel can find the operation. The write
//! itself carries no time limit. Server text (`errmsg`) is never copied into a result: it can
//! quote document values. Anything that makes the result untrustworthy (driver error,
//! `writeErrors`, write concern error, unreadable counts) is `BulkOutcome::Unknown`, because the
//! write may have happened.

use std::time::Duration;

use bson::{Bson, Document, doc};
use mongodb::{Collection, Database};

use crate::bulk::{
    BulkDeleteRequest, BulkOutcome, BulkRunId, BulkUpdateRequest, UpdateSpec, check_scope,
    validate_update,
};
use crate::driver_error::is_time_limit_expiry;
use crate::error::{CoreError, Result};

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
        Ok(reply) => parse(&reply).unwrap_or(UNKNOWN),
        Err(_) => UNKNOWN,
    }
}

const UNKNOWN: BulkOutcome = BulkOutcome::Unknown { cancelled: false };

fn update_command(coll_name: &str, req: &BulkUpdateRequest, run: BulkRunId) -> Document {
    let update = match &req.update {
        UpdateSpec::Operators(ops) => Bson::Document(ops.clone()),
        UpdateSpec::Pipeline(stages) => {
            Bson::Array(stages.iter().cloned().map(Bson::Document).collect())
        }
    };
    doc! {
        "update": coll_name,
        "updates": [{ "q": req.filter.clone(), "u": update, "multi": true, "upsert": req.upsert }],
        "ordered": true,
        "comment": run.0.to_string(),
    }
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
fn reply_is_clean(reply: &Document) -> Option<()> {
    let ok = match reply.get("ok") {
        Some(Bson::Double(v)) => *v == 1.0,
        Some(Bson::Int32(v)) => *v == 1,
        Some(Bson::Int64(v)) => *v == 1,
        _ => false,
    };
    let clean =
        ok && !reply.contains_key("writeErrors") && !reply.contains_key("writeConcernError");
    clean.then_some(())
}

fn count_field(reply: &Document, key: &str) -> Option<u64> {
    match reply.get(key)? {
        Bson::Int32(n) => u64::try_from(*n).ok(),
        Bson::Int64(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// The `_id` of every upserted document; an absent `upserted` is empty, a malformed one is `None`.
fn upserted_ids(reply: &Document) -> Option<Vec<Bson>> {
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
