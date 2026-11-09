//! Snapshot streaming and undo for [`crate::mongo_store::MongoStore`].
//!
//! Every undo item is ONE entry of a server write command that carries its own precondition, so
//! an edit made by someone else between the check and the write cannot be overwritten: a replace
//! or delete only matches while the document still equals the state the run produced. The
//! expected document is wrapped in `$literal` so a string value such as `"$x"` is never read as a
//! field path. Entries go out in size-bounded, unordered batches; the server reports failures by
//! index, and what the precondition decided is read back afterwards. All reads go to the primary:
//! a secondary may not have the write yet. Server text is never copied into a result or a log
//! line. A lost connection aborts the undo with `ConnectionInterrupted` (some documents may
//! already be restored) instead of failing item after item.

use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::{Collection, Database};

use crate::bulk::BulkRunId;
use crate::driver_error::classify_driver_error;
use crate::error::{CoreError, Result};
use crate::mongo_bulk_cancel::primary;
use crate::snapshot::{ItemResult, SnapshotWriter, UndoItem, id_key};

const DUPLICATE_KEY: i32 = 11000;
/// Code recorded for an entry whose whole batch failed; never a server code.
const BATCH_FAILED: i32 = -1;
/// Most bytes one write command carries (the server limit is 48 MB per message).
const MAX_COMMAND_BYTES: usize = 8 * 1024 * 1024;

/// Streams `filter`'s matches into `out`; a snapshot failure (cap, disk) aborts the stream. The
/// `find` is tagged with the run id so a cancel can kill it like it kills the write.
pub(crate) async fn snapshot_matching(
    coll: &Collection<Document>,
    filter: &Document,
    run: BulkRunId,
    out: &mut SnapshotWriter,
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<u64> {
    let mut cursor = coll
        .find(filter.clone())
        .selection_criteria(primary())
        .comment(Bson::String(run.0.to_string()))
        .await
        .map_err(db_err)?;
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
        .selection_criteria(primary())
        .await
        .map_err(db_err)?;
    cursor.try_collect().await.map_err(db_err)
}

pub(crate) async fn apply_undo(
    db: &Database,
    name: &str,
    items: &[UndoItem],
    db_err: fn(mongodb::error::Error) -> CoreError,
) -> Result<Vec<ItemResult>> {
    let coll = db.collection::<Document>(name);
    let mut results = vec![ItemResult::Failed; items.len()];
    let of_kind = |pick: fn(&UndoItem) -> bool| -> Vec<usize> {
        (0..items.len()).filter(|&i| pick(&items[i])).collect()
    };
    let reinserts = of_kind(|i| matches!(i, UndoItem::Reinsert { .. }));
    let restores = of_kind(|i| matches!(i, UndoItem::Restore { .. }));
    let removes = of_kind(|i| matches!(i, UndoItem::Remove { .. }));
    let ctx = Ctx {
        db,
        name,
        coll: &coll,
        db_err,
    };
    reinsert(&ctx, items, &reinserts, &mut results).await?;
    restore(&ctx, items, &restores, &mut results).await?;
    remove(&ctx, items, &removes, &mut results).await?;
    Ok(results)
}

struct Ctx<'a> {
    db: &'a Database,
    name: &'a str,
    coll: &'a Collection<Document>,
    db_err: fn(mongodb::error::Error) -> CoreError,
}

fn id_of(doc: &Document) -> Bson {
    doc.get("_id").cloned().unwrap_or(Bson::Null)
}

/// The filter that matches `expected` only while the stored document still equals it.
fn still_equals(expected: &Document) -> Document {
    doc! {
        "_id": id_of(expected),
        "$expr": { "$eq": ["$$ROOT", { "$literal": expected.clone() }] },
    }
}

async fn reinsert(
    ctx: &Ctx<'_>,
    items: &[UndoItem],
    at: &[usize],
    results: &mut [ItemResult],
) -> Result<()> {
    let entries: Vec<Document> = at
        .iter()
        .filter_map(|&i| match &items[i] {
            UndoItem::Reinsert { original } => Some(original.clone()),
            _ => None,
        })
        .collect();
    let codes = run_batches(ctx, "insert", "documents", entries).await?;
    let mut clashes: Vec<(usize, Bson)> = Vec::new();
    for (&i, code) in at.iter().zip(&codes) {
        match code {
            None => results[i] = ItemResult::Done,
            Some(DUPLICATE_KEY) => clashes.push((i, id_of(original_of(&items[i])))),
            Some(_) => results[i] = ItemResult::Failed,
        }
    }
    if clashes.is_empty() {
        return Ok(());
    }
    // A duplicate key may be a unique secondary index, not the `_id`: only an existing `_id`
    // means the document is present.
    let ids: Vec<Bson> = clashes.iter().map(|(_, id)| id.clone()).collect();
    let present: std::collections::HashSet<String> = find_by_ids(ctx.coll, &ids, ctx.db_err)
        .await?
        .iter()
        .map(|d| id_key(&id_of(d)))
        .collect();
    for (i, id) in clashes {
        results[i] = if present.contains(&id_key(&id)) {
            ItemResult::SkippedPresent
        } else {
            ItemResult::Failed
        };
    }
    Ok(())
}

fn original_of(item: &UndoItem) -> &Document {
    match item {
        UndoItem::Reinsert { original } | UndoItem::Restore { original, .. } => original,
        UndoItem::Remove { expected } => expected,
    }
}

async fn restore(
    ctx: &Ctx<'_>,
    items: &[UndoItem],
    at: &[usize],
    results: &mut [ItemResult],
) -> Result<()> {
    let entries: Vec<Document> = at
        .iter()
        .filter_map(|&i| match &items[i] {
            UndoItem::Restore { original, expected } => Some(doc! {
                "q": still_equals(expected),
                "u": original.clone(),
                "multi": false,
                "upsert": false,
            }),
            _ => None,
        })
        .collect();
    let codes = run_batches(ctx, "update", "updates", entries).await?;
    let sent: Vec<usize> = at
        .iter()
        .zip(&codes)
        .filter_map(|(&i, code)| code.is_none().then_some(i))
        .collect();
    // The precondition decides per entry whether it matched; the reply only totals them, so read
    // back: a document that is now the original was restored (or already was).
    let current = current_by_id(ctx, items, &sent).await?;
    for &i in &sent {
        let original = original_of(&items[i]);
        results[i] = match current.get(&id_key(&id_of(original))) {
            Some(now) if now == original => ItemResult::Done,
            _ => ItemResult::SkippedChanged,
        };
    }
    Ok(())
}

async fn remove(
    ctx: &Ctx<'_>,
    items: &[UndoItem],
    at: &[usize],
    results: &mut [ItemResult],
) -> Result<()> {
    let entries: Vec<Document> = at
        .iter()
        .filter_map(|&i| match &items[i] {
            UndoItem::Remove { expected } => Some(doc! { "q": still_equals(expected), "limit": 1 }),
            _ => None,
        })
        .collect();
    let codes = run_batches(ctx, "delete", "deletes", entries).await?;
    let sent: Vec<usize> = at
        .iter()
        .zip(&codes)
        .filter_map(|(&i, code)| code.is_none().then_some(i))
        .collect();
    let current = current_by_id(ctx, items, &sent).await?;
    for &i in &sent {
        results[i] = if current.contains_key(&id_key(&id_of(original_of(&items[i])))) {
            ItemResult::SkippedChanged
        } else {
            ItemResult::Done
        };
    }
    Ok(())
}

async fn current_by_id(
    ctx: &Ctx<'_>,
    items: &[UndoItem],
    at: &[usize],
) -> Result<std::collections::HashMap<String, Document>> {
    if at.is_empty() {
        return Ok(Default::default());
    }
    let ids: Vec<Bson> = at.iter().map(|&i| id_of(original_of(&items[i]))).collect();
    Ok(find_by_ids(ctx.coll, &ids, ctx.db_err)
        .await?
        .into_iter()
        .map(|d| (id_key(&id_of(&d)), d))
        .collect())
}

/// Runs `entries` as unordered write commands bounded in size and returns, per entry, the server
/// code of its write error (`None` = no error). A batch the server refused as a whole marks its
/// entries failed; a lost connection aborts.
async fn run_batches(
    ctx: &Ctx<'_>,
    command: &str,
    key: &str,
    entries: Vec<Document>,
) -> Result<Vec<Option<i32>>> {
    let mut codes = vec![None; entries.len()];
    let mut start = 0;
    while start < entries.len() {
        let end = batch_end(&entries, start);
        let chunk = Bson::Array(
            entries[start..end]
                .iter()
                .cloned()
                .map(Bson::Document)
                .collect(),
        );
        let cmd = doc! { command: ctx.name, key: chunk, "ordered": false };
        match ctx.db.run_command(cmd).await {
            Ok(reply) => mark_errors(&reply, start, end, &mut codes),
            Err(e) if connection_lost(&e) => {
                return Err(CoreError::ConnectionInterrupted("undo".into()));
            }
            Err(_) => {
                log::warn!("undo: the server refused a batch of documents");
                codes[start..end].fill(Some(BATCH_FAILED));
            }
        }
        start = end;
    }
    Ok(codes)
}

/// The end of the batch starting at `start`: at least one entry, at most `MAX_COMMAND_BYTES`.
fn batch_end(entries: &[Document], start: usize) -> usize {
    let mut bytes = 0;
    let mut end = start;
    while end < entries.len() {
        bytes += bson::to_vec(&entries[end]).map_or(MAX_COMMAND_BYTES, |v| v.len());
        if end > start && bytes > MAX_COMMAND_BYTES {
            break;
        }
        end += 1;
    }
    end
}

/// Records the per-entry errors of a reply; a reply that is not `ok` fails the whole batch.
fn mark_errors(reply: &Document, start: usize, end: usize, codes: &mut [Option<i32>]) {
    let ok = reply
        .get("ok")
        .is_some_and(|v| matches!(v, Bson::Double(d) if *d == 1.0) || matches!(v, Bson::Int32(1)));
    if !ok {
        codes[start..end].fill(Some(BATCH_FAILED));
        return;
    }
    for error in reply.get_array("writeErrors").into_iter().flatten() {
        let Some(error) = error.as_document() else {
            continue;
        };
        let index = error
            .get("index")
            .and_then(|i| i.as_i32().map(i64::from).or_else(|| i.as_i64()));
        let code = error.get_i32("code").unwrap_or(BATCH_FAILED);
        if let Some(slot) = index
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| codes.get_mut(start + i))
            .filter(|_| start < end)
        {
            *slot = Some(code);
        }
    }
}

/// Whether the driver lost the server (so continuing would only fail every remaining item).
fn connection_lost(e: &mongodb::error::Error) -> bool {
    matches!(
        classify_driver_error(e),
        CoreError::Unreachable
            | CoreError::UnreachableSsh
            | CoreError::ConnectionInterrupted(_)
            | CoreError::TlsHandshake
            | CoreError::TlsHandshakeSsh
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_errors_are_recorded_by_index_within_their_batch() {
        let reply = doc! {
            "ok": 1.0,
            "writeErrors": [{ "index": 1, "code": 11000 }, { "index": 2, "code": 121 }],
        };
        let mut codes = vec![None; 6];
        mark_errors(&reply, 2, 5, &mut codes);
        assert_eq!(codes, [None, None, None, Some(11000), Some(121), None]);
    }

    #[test]
    fn a_reply_that_is_not_ok_fails_the_whole_batch() {
        let mut codes = vec![None; 4];
        mark_errors(&doc! { "ok": 0.0 }, 1, 3, &mut codes);
        assert_eq!(codes, [None, Some(BATCH_FAILED), Some(BATCH_FAILED), None]);
    }

    #[test]
    fn batches_are_bounded_in_size_but_never_empty() {
        let big = doc! { "x": "y".repeat(MAX_COMMAND_BYTES) };
        let small = doc! { "x": 1 };
        let entries = vec![big.clone(), small.clone(), small];
        assert_eq!(batch_end(&entries, 0), 1, "a huge entry goes alone");
        assert_eq!(batch_end(&entries, 1), 3);
    }

    #[test]
    fn the_precondition_wraps_the_expected_document_in_literal() {
        let f = still_equals(&doc! { "_id": 1, "s": "$notAPath" });
        let expr = f.get_document("$expr").unwrap();
        assert_eq!(
            expr.get_array("$eq").unwrap()[1],
            Bson::Document(doc! { "$literal": { "_id": 1, "s": "$notAPath" } })
        );
    }
}
