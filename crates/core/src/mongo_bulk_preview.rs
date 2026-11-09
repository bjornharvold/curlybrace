//! The update preview for [`crate::mongo_store::MongoStore`]: the user's real update runs inside
//! a client-session transaction on a few documents and the transaction is ALWAYS aborted. No code
//! path in this file commits.
//!
//! Anything the transaction cannot do (standalone server, capped collection, old server) degrades
//! to the "before" documents with a gap, never to a guessed "after". Server text and document
//! values are never copied into an error or a log line.

use bson::{Bson, Document, doc};
use mongodb::error::{ErrorKind, TRANSIENT_TRANSACTION_ERROR};
use mongodb::{Client, ClientSession, Collection, Database};

use crate::bulk::{
    BULK_PREVIEW_DOCS, BulkUpdateRequest, PreviewDoc, PreviewGap, UpdatePreview, check_scope,
    validate_update,
};
use crate::driver_error::server_error_code;
use crate::error::{CoreError, Result};
use crate::mongo_bulk::{reply_is_clean, update_entry, upserted_ids};
use crate::mongo_write::write_error_code;
use crate::store::Namespace;

const CODE_WRITE_CONFLICT: i32 = 112;
/// `IllegalOperation`: a standalone server ("Transaction numbers are only allowed on a replica
/// set member or mongos") and a write to a capped collection in a transaction.
const CODE_ILLEGAL_OPERATION: i32 = 20;
const CODE_COMMAND_NOT_SUPPORTED: i32 = 115;
const CODE_OPERATION_NOT_SUPPORTED_IN_TRANSACTION: i32 = 263;
const CODE_TRANSACTIONS_NOT_SUPPORTED: i32 = 303;
/// Codes meaning "this deployment or collection cannot run the update in a transaction".
const UNAVAILABLE_CODES: [i32; 4] = [
    CODE_ILLEGAL_OPERATION,
    CODE_COMMAND_NOT_SUPPORTED,
    CODE_OPERATION_NOT_SUPPORTED_IN_TRANSACTION,
    CODE_TRANSACTIONS_NOT_SUPPORTED,
];

/// Server codes for an update that is wrong for the data it meets or is itself malformed (not a
/// problem of the deployment): 2 BadValue, 9 FailedToParse (an unknown operator such as `$sett`),
/// 14 TypeMismatch, 15 Overflow, 28 PathNotViable, 40 ConflictingUpdateOperators,
/// 52 DollarPrefixedFieldName, 66 ImmutableField, 121 DocumentValidationFailure, 11000
/// DuplicateKey. Only these are reported as an invalid update; 9, 14, 28 and 66 are exercised
/// against a real server in `local_bulk.rs`. (16 InvalidLength is not a data error of an update.)
const DATA_ERROR_CODES: [i32; 10] = [2, 9, 14, 15, 28, 40, 52, 66, 121, 11000];

/// How a failed step inside the preview transaction is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Show the gap and keep the "before" documents.
    Gap(PreviewGap),
    /// The user's update fails on these documents: the real run would fail too.
    InvalidUpdate,
}

/// Pure: maps the facts of a failure (server code, `TransientTransactionError` label, driver
/// "transactions are not supported" kind) to its handling. Everything that is not a data error
/// of the user's update degrades to a gap, because a preview failure is "preview unavailable",
/// never a failed update.
pub(crate) fn classify_failure(
    code: Option<i32>,
    transient: bool,
    no_transactions: bool,
) -> Failure {
    match code {
        _ if no_transactions => Failure::Gap(PreviewGap::TransactionsUnavailable),
        Some(CODE_WRITE_CONFLICT) => Failure::Gap(PreviewGap::Conflict),
        Some(c) if UNAVAILABLE_CODES.contains(&c) => {
            Failure::Gap(PreviewGap::TransactionsUnavailable)
        }
        _ if transient => Failure::Gap(PreviewGap::Conflict),
        Some(c) if DATA_ERROR_CODES.contains(&c) => Failure::InvalidUpdate,
        _ => Failure::Gap(PreviewGap::TransactionsUnavailable),
    }
}

fn classify_driver(e: &mongodb::error::Error) -> Failure {
    let code = server_error_code(e);
    let no_txn = matches!(e.kind.as_ref(), ErrorKind::Transaction { .. });
    classify_failure(code, e.contains_label(TRANSIENT_TRANSACTION_ERROR), no_txn)
}

/// Whether a command reply carries the `TransientTransactionError` label.
pub(crate) fn reply_is_transient(reply: &Document) -> bool {
    reply.get_array("errorLabels").is_ok_and(|labels| {
        labels
            .iter()
            .any(|l| l.as_str() == Some(TRANSIENT_TRANSACTION_ERROR))
    })
}

/// `{ $and: [ { _id: { $in: ids } }, filter ] }`: the filter narrowed to the previewed ids.
pub(crate) fn restricted_filter(filter: &Document, ids: Vec<Bson>) -> Document {
    doc! { "$and": [{ "_id": { "$in": ids } }, filter.clone()] }
}

fn ids_of(docs: &[Document]) -> Vec<Bson> {
    docs.iter().filter_map(|d| d.get("_id").cloned()).collect()
}

/// What the transaction produced.
enum Attempt {
    /// The documents re-read after the update, in no particular order.
    Afters(Vec<Document>),
    Gap(PreviewGap),
    Invalid,
}

pub(crate) async fn preview_update(
    client: &Client,
    ns: &Namespace,
    req: &BulkUpdateRequest,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<UpdatePreview> {
    check_scope(&req.filter, req.scope)?;
    validate_update(&req.update)?;
    let db = client.database(&ns.db);
    let coll: Collection<Document> = db.collection(&ns.collection);
    let befores = first_matches(&coll, &req.filter).await.map_err(db_err)?;
    if befores.is_empty() && !req.upsert {
        return Ok(UpdatePreview {
            docs: Vec::new(),
            gap: Some(PreviewGap::NoMatch),
        });
    }
    let Ok(mut session) = client.start_session().await else {
        return Ok(before_only(befores, PreviewGap::TransactionsUnavailable));
    };
    if let Err(e) = session.start_transaction().await {
        let gap = match classify_driver(&e) {
            Failure::Gap(gap) => gap,
            Failure::InvalidUpdate => PreviewGap::TransactionsUnavailable,
        };
        return Ok(before_only(befores, gap));
    }
    let attempt = in_transaction(&db, &ns.collection, &mut session, req, &befores).await;
    abort(&mut session).await?;
    match attempt {
        Attempt::Afters(afters) => Ok(pair_up(befores, afters)),
        Attempt::Gap(gap) => Ok(before_only(befores, gap)),
        Attempt::Invalid => Err(CoreError::BulkInvalidUpdate),
    }
}

/// Aborts the transaction. The driver (mongodb 3.6) swallows a server-side `abortTransaction`
/// failure and returns `Ok`, so this errors only on a driver state error (no transaction in
/// progress, already aborted/committed): loud, not silent.
async fn abort(session: &mut ClientSession) -> Result<()> {
    session.abort_transaction().await.map_err(|_| {
        log::error!("bulk update preview: aborting the preview transaction failed");
        CoreError::BulkPreviewUnavailable
    })
}

async fn first_matches(
    coll: &Collection<Document>,
    filter: &Document,
) -> mongodb::error::Result<Vec<Document>> {
    use futures::TryStreamExt;
    let cursor = coll
        .find(filter.clone())
        .limit(BULK_PREVIEW_DOCS as i64)
        .await?;
    cursor.try_collect().await
}

fn before_only(befores: Vec<Document>, gap: PreviewGap) -> UpdatePreview {
    let docs = befores
        .into_iter()
        .map(|before| PreviewDoc {
            before,
            after: None,
        })
        .collect();
    UpdatePreview {
        docs,
        gap: Some(gap),
    }
}

/// Pairs each before with the re-read document of the same `_id`. An upsert has no before: its
/// pair carries an empty `before` (a stored document always has an `_id`, so it cannot be
/// mistaken for one).
fn pair_up(befores: Vec<Document>, afters: Vec<Document>) -> UpdatePreview {
    let docs = if befores.is_empty() {
        afters
            .into_iter()
            .map(|after| PreviewDoc {
                before: Document::new(),
                after: Some(after),
            })
            .collect()
    } else {
        befores
            .into_iter()
            .map(|before| {
                let after = afters.iter().find(|a| a.get("_id") == before.get("_id"));
                PreviewDoc {
                    after: after.cloned(),
                    before,
                }
            })
            .collect()
    };
    UpdatePreview { docs, gap: None }
}

/// The `update` command the preview runs: the filter narrowed to the previewed ids and applied
/// to all of them, or, when nothing matches (an upsert), the user's filter run once.
pub(crate) fn preview_command(
    coll: &str,
    req: &BulkUpdateRequest,
    befores: &[Document],
) -> Document {
    let (filter, multi) = if befores.is_empty() {
        (req.filter.clone(), false)
    } else {
        (restricted_filter(&req.filter, ids_of(befores)), true)
    };
    doc! {
        "update": coll,
        "updates": [update_entry(req, filter, multi)],
        "ordered": true,
    }
}

async fn in_transaction(
    db: &Database,
    coll: &str,
    session: &mut ClientSession,
    req: &BulkUpdateRequest,
    befores: &[Document],
) -> Attempt {
    let command = preview_command(coll, req, befores);
    let reply = match db.run_command(command).session(&mut *session).await {
        Ok(reply) => reply,
        Err(e) => return failed(&e),
    };
    if let Some(code) = write_error_code(&reply) {
        return from_failure(classify_failure(code, reply_is_transient(&reply), false));
    }
    if reply_is_clean(&reply).is_none() {
        return Attempt::Gap(PreviewGap::TransactionsUnavailable);
    }
    let Some(ids) = read_back_ids(&reply, befores) else {
        return Attempt::Gap(PreviewGap::Conflict);
    };
    match read_in(db, coll, session, ids).await {
        Ok(Some(afters)) if !afters.is_empty() => Attempt::Afters(afters),
        // An empty or unreadable re-read is never shown as a clean preview.
        Ok(_) => Attempt::Gap(PreviewGap::TransactionsUnavailable),
        Err(e) => failed(&e),
    }
}

fn failed(e: &mongodb::error::Error) -> Attempt {
    from_failure(classify_driver(e))
}

fn from_failure(failure: Failure) -> Attempt {
    match failure {
        Failure::Gap(gap) => Attempt::Gap(gap),
        Failure::InvalidUpdate => Attempt::Invalid,
    }
}

/// The ids to re-read: the previewed ones, or the one an upsert inserted. `None` when an upsert
/// preview inserted nothing (a concurrent writer made the filter match meanwhile).
fn read_back_ids(reply: &Document, befores: &[Document]) -> Option<Vec<Bson>> {
    if !befores.is_empty() {
        return Some(ids_of(befores));
    }
    let ids = upserted_ids(reply)?;
    (!ids.is_empty()).then_some(ids)
}

/// A `find` on `_id` inside the session's transaction. `Ok(None)` when the reply has no readable
/// `firstBatch` of documents (at most [`BULK_PREVIEW_DOCS`] are asked for).
async fn read_in(
    db: &Database,
    coll: &str,
    session: &mut ClientSession,
    ids: Vec<Bson>,
) -> mongodb::error::Result<Option<Vec<Document>>> {
    let command = doc! {
        "find": coll,
        "filter": { "_id": { "$in": ids } },
        "limit": BULK_PREVIEW_DOCS as i64,
    };
    let reply = db.run_command(command).session(session).await?;
    Ok(first_batch(&reply))
}

/// The documents of a `find` reply's `firstBatch`; `None` when it is absent or has a non-document.
pub(crate) fn first_batch(reply: &Document) -> Option<Vec<Document>> {
    let batch = reply
        .get_document("cursor")
        .ok()?
        .get_array("firstBatch")
        .ok()?;
    batch.iter().map(|d| d.as_document().cloned()).collect()
}

#[cfg(test)]
#[path = "mongo_bulk_preview_tests.rs"]
mod tests;
