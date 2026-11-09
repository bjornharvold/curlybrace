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
use crate::error::{CoreError, Result};
use crate::mongo_bulk::{reply_is_clean, update_entry, upserted_ids};
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

/// How a failed step inside the preview transaction is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    /// Show the gap and keep the "before" documents.
    Gap(PreviewGap),
    /// Any other failure: the update itself is wrong for these documents, or the driver failed.
    Other,
}

/// Pure: maps the facts of a failure (server code, `TransientTransactionError` label, driver
/// "transactions are not supported" kind) to its handling.
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
        _ => Failure::Other,
    }
}

fn classify_driver(e: &mongodb::error::Error) -> Failure {
    let code = match e.kind.as_ref() {
        ErrorKind::Command(c) => Some(c.code),
        _ => None,
    };
    let no_txn = matches!(e.kind.as_ref(), ErrorKind::Transaction { .. });
    classify_failure(code, e.contains_label(TRANSIENT_TRANSACTION_ERROR), no_txn)
}

/// The code of the first entry of a reply's `writeErrors`, if there is one.
pub(crate) fn write_error_code(reply: &Document) -> Option<Option<i32>> {
    let first = reply.get_array("writeErrors").ok()?.first()?;
    Some(first.as_document().and_then(|d| d.get_i32("code").ok()))
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
    Failed(CoreError),
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
    let mut session = client.start_session().await.map_err(db_err)?;
    if let Err(e) = session.start_transaction().await {
        return match classify_driver(&e) {
            Failure::Gap(gap) => Ok(before_only(befores, gap)),
            Failure::Other => Err(db_err(e)),
        };
    }
    let attempt = in_transaction(&db, &ns.collection, &mut session, req, &befores, db_err).await;
    abort(&mut session).await?;
    match attempt {
        Attempt::Afters(afters) => Ok(pair_up(befores, afters)),
        Attempt::Gap(gap) => Ok(before_only(befores, gap)),
        Attempt::Failed(e) => Err(e),
    }
}

/// Aborts the transaction. A failed abort leaves it to time out on the server: loud, not silent.
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

async fn in_transaction(
    db: &Database,
    coll: &str,
    session: &mut ClientSession,
    req: &BulkUpdateRequest,
    befores: &[Document],
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Attempt {
    let filter = if befores.is_empty() {
        req.filter.clone()
    } else {
        restricted_filter(&req.filter, ids_of(befores))
    };
    let command = doc! {
        "update": coll,
        "updates": [update_entry(req, filter)],
        "ordered": true,
    };
    let reply = match db.run_command(command).session(&mut *session).await {
        Ok(reply) => reply,
        Err(e) => return failed(&e, db_err),
    };
    if let Some(code) = write_error_code(&reply) {
        return match classify_failure(code, false, false) {
            Failure::Gap(gap) => Attempt::Gap(gap),
            Failure::Other => Attempt::Failed(CoreError::BulkInvalidUpdate),
        };
    }
    if reply_is_clean(&reply).is_none() {
        return Attempt::Failed(CoreError::BulkPreviewUnavailable);
    }
    let ids = match read_back_ids(&reply, befores) {
        Some(ids) => ids,
        None => return Attempt::Gap(PreviewGap::Conflict),
    };
    match read_in(db, coll, session, ids).await {
        Ok(afters) => Attempt::Afters(afters),
        Err(e) => failed(&e, db_err),
    }
}

fn failed(e: &mongodb::error::Error, db_err: fn(mongodb::error::Error) -> CoreError) -> Attempt {
    match classify_driver(e) {
        Failure::Gap(gap) => Attempt::Gap(gap),
        Failure::Other => Attempt::Failed(db_err(e.clone())),
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

/// A `find` on `_id` inside the session's transaction; the reply's first batch (at most
/// [`BULK_PREVIEW_DOCS`] documents) is the result.
async fn read_in(
    db: &Database,
    coll: &str,
    session: &mut ClientSession,
    ids: Vec<Bson>,
) -> mongodb::error::Result<Vec<Document>> {
    let command = doc! {
        "find": coll,
        "filter": { "_id": { "$in": ids } },
        "limit": BULK_PREVIEW_DOCS as i64,
    };
    let reply = db.run_command(command).session(session).await?;
    let batch = reply
        .get_document("cursor")
        .and_then(|c| c.get_array("firstBatch"))
        .map(|b| b.iter().filter_map(|d| d.as_document().cloned()).collect());
    Ok(batch.unwrap_or_default())
}

#[cfg(test)]
#[path = "mongo_bulk_preview_tests.rs"]
mod tests;
