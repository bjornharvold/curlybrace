//! Snapshot streaming and undo for [`crate::mongo_store::MongoStore`].
//!
//! Every undo item is ONE server operation that carries its own precondition, so an edit made
//! by someone else between the check and the write cannot be overwritten: a replace or delete
//! only matches while the document still equals the state the run produced. The expected
//! document is wrapped in `$literal` so a string value such as `"$x"` is never read as a field
//! path. Server text is never copied into a result or a log line.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::Collection;

use crate::driver_error::server_error_code;
use crate::error::{CoreError, Result};
use crate::snapshot::{ItemResult, SnapshotWriter, UndoItem};

const DUPLICATE_KEY: i32 = 11000;

/// Streams `filter`'s matches into `out`; a snapshot failure (cap, disk) aborts the stream.
pub(crate) async fn snapshot_matching(
    coll: &Collection<Document>,
    filter: &Document,
    out: &mut SnapshotWriter,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<u64> {
    let mut cursor = coll.find(filter.clone()).await.map_err(db_err)?;
    let mut count = 0_u64;
    while let Some(doc) = cursor.try_next().await.map_err(db_err)? {
        out.push(&doc)?;
        count += 1;
    }
    Ok(count)
}

pub(crate) async fn find_by_ids(
    coll: &Collection<Document>,
    ids: &[Bson],
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Vec<Document>> {
    let cursor = coll
        .find(doc! { "_id": { "$in": ids.to_vec() } })
        .await
        .map_err(db_err)?;
    cursor.try_collect().await.map_err(db_err)
}

pub(crate) async fn apply_undo(coll: &Collection<Document>, items: &[UndoItem]) -> Vec<ItemResult> {
    let mut results = Vec::with_capacity(items.len());
    for item in items {
        results.push(apply_item(coll, item).await);
    }
    results
}

async fn apply_item(coll: &Collection<Document>, item: &UndoItem) -> ItemResult {
    let outcome = match item {
        UndoItem::Reinsert { original } => reinsert(coll, original).await,
        UndoItem::Restore { original, expected } => restore(coll, original, expected).await,
        UndoItem::Remove { expected } => remove(coll, expected).await,
    };
    outcome.unwrap_or_else(|_| {
        log::warn!("undo: the server refused one document");
        ItemResult::Failed
    })
}

type Outcome = std::result::Result<ItemResult, mongodb::error::Error>;

async fn reinsert(coll: &Collection<Document>, original: &Document) -> Outcome {
    match coll.insert_one(original).await {
        Ok(_) => Ok(ItemResult::Done),
        Err(e) if server_error_code(&e) == Some(DUPLICATE_KEY) => Ok(ItemResult::SkippedPresent),
        Err(e) => Err(e),
    }
}

/// The filter that matches `expected` only while the stored document still equals it.
fn still_equals(expected: &Document) -> Document {
    doc! {
        "_id": expected.get("_id").cloned().unwrap_or(Bson::Null),
        "$expr": { "$eq": ["$$ROOT", { "$literal": expected.clone() }] },
    }
}

async fn restore(coll: &Collection<Document>, original: &Document, expected: &Document) -> Outcome {
    let replaced = coll.replace_one(still_equals(expected), original).await?;
    if replaced.matched_count == 1 {
        return Ok(ItemResult::Done);
    }
    // Not matched: either already back to the original (fine) or changed by someone else.
    let id = original.get("_id").cloned().unwrap_or(Bson::Null);
    let current = coll.find_one(doc! { "_id": id }).await?;
    Ok(match current {
        Some(doc) if doc == *original => ItemResult::Done,
        _ => ItemResult::SkippedChanged,
    })
}

async fn remove(coll: &Collection<Document>, expected: &Document) -> Outcome {
    let deleted = coll.delete_one(still_equals(expected)).await?;
    Ok(if deleted.deleted_count == 1 {
        ItemResult::Done
    } else {
        ItemResult::SkippedChanged
    })
}
