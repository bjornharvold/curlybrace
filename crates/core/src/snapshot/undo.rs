//! Undo of a bulk write, from its snapshot.
//!
//! The snapshot holds the originals. Right after the write, [`capture_after`] saves what the
//! touched documents look like (one line per original, in the same order, then the upserted
//! documents). Undo restores an original only when the document still equals that saved
//! after-state, and the server checks that in the same operation that replaces it, so an edit
//! made in between is never overwritten.
//!
//! The files can be large, so they are read and written in batches on the blocking pool, never on
//! the async worker threads.

use std::path::{Path, PathBuf};

use bson::{Bson, Document, doc};

use super::file::{SnapshotHeader, SnapshotKind, SnapshotReader, SnapshotWriter};
use crate::error::{CoreError, Result};
use crate::store::{DocumentStore, Namespace};

/// Documents handled per round trip and per file batch.
const BATCH: usize = 500;

/// One document's undo, applied by the server with its own precondition.
#[derive(Debug, Clone, PartialEq)]
pub enum UndoItem {
    /// A deleted document: insert it unless its `_id` exists now.
    Reinsert { original: Document },
    /// An updated document: replace it with `original` only while it still equals `expected`.
    Restore {
        original: Document,
        expected: Document,
    },
    /// An upserted document: delete it only while it still equals `expected`.
    Remove { expected: Document },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemResult {
    /// The document is now as it was before the run.
    Done,
    /// A reinsert found the `_id` already in use.
    SkippedPresent,
    /// The document was changed (or removed) by someone else since the run.
    SkippedChanged,
    /// The server refused this item.
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UndoReport {
    pub restored: u64,
    pub skipped_changed: u64,
    pub skipped_present: u64,
    /// Documents the run never changed, so there was nothing to restore.
    pub unchanged: u64,
    pub errors: u64,
}

impl UndoReport {
    fn add(&mut self, result: ItemResult) {
        match result {
            ItemResult::Done => self.restored += 1,
            ItemResult::SkippedPresent => self.skipped_present += 1,
            ItemResult::SkippedChanged => self.skipped_changed += 1,
            ItemResult::Failed => self.errors += 1,
        }
    }
}

/// `<run>.after.jsonl` next to the snapshot.
pub fn after_path_for(snapshot: &Path) -> PathBuf {
    snapshot.with_extension("after.jsonl")
}

/// A stable text key for an `_id` (canonical extended JSON), for matching ids across files.
pub fn id_key(id: &Bson) -> String {
    id.clone().into_canonical_extjson().to_string()
}

fn id_of(doc: &Document) -> Result<Bson> {
    doc.get("_id").cloned().ok_or(CoreError::SnapshotUnreadable)
}

/// Runs blocking file work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| CoreError::SnapshotUnreadable)?
}

/// The next `n` documents of `reader` (fewer at the end), with the reader handed back.
async fn read_batch(
    mut reader: SnapshotReader,
    n: usize,
) -> Result<(SnapshotReader, Vec<Document>)> {
    blocking(move || {
        let docs = reader.by_ref().take(n).collect::<Result<Vec<_>>>()?;
        Ok((reader, docs))
    })
    .await
}

async fn push_all(mut out: SnapshotWriter, lines: Vec<Document>) -> Result<SnapshotWriter> {
    blocking(move || {
        for line in &lines {
            out.push(line)?;
        }
        Ok(out)
    })
    .await
}

/// Saves the current state of every document in the snapshot, plus the `upserted` documents.
/// Call it right after the write finished, whatever its outcome.
pub async fn capture_after(
    store: &dyn DocumentStore,
    snapshot: &Path,
    upserted: &[Bson],
    max_bytes: u64,
) -> Result<()> {
    let mut reader = SnapshotReader::open(snapshot)?;
    let header = reader.header().clone();
    let dir = snapshot.parent().ok_or(CoreError::SnapshotWriteFailed)?;
    let after = after_path_for(snapshot);
    let name = after
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(CoreError::SnapshotWriteFailed)?;
    let mut out = SnapshotWriter::create_as(dir, name, header.clone(), max_bytes)?;
    loop {
        let (next, originals) = read_batch(reader, BATCH).await?;
        reader = next;
        if originals.is_empty() {
            break;
        }
        let ids = originals.iter().map(id_of).collect::<Result<Vec<_>>>()?;
        let lines = after_lines(store, &header.ns, &ids, false).await?;
        out = push_all(out, lines).await?;
    }
    for chunk in upserted.chunks(BATCH) {
        let lines = after_lines(store, &header.ns, chunk, true).await?;
        out = push_all(out, lines).await?;
    }
    blocking(move || out.finish().map(|_| ())).await
}

/// The after-state lines of `ids`. For the snapshot's own documents (`upserted` false) every id
/// gets a line (`after: null` when the document is gone), keeping the file aligned with the
/// snapshot; for upserted ones only the found documents do.
async fn after_lines(
    store: &dyn DocumentStore,
    ns: &Namespace,
    ids: &[Bson],
    upserted: bool,
) -> Result<Vec<Document>> {
    let found: std::collections::HashMap<String, Document> = store
        .find_by_ids(ns, ids)
        .await?
        .into_iter()
        .filter_map(|d| Some((id_key(d.get("_id")?), d)))
        .collect();
    let mut lines = Vec::with_capacity(ids.len());
    for id in ids {
        let after = found.get(&id_key(id));
        if after.is_none() && upserted {
            continue;
        }
        let mut line = doc! {
            "_id": id.clone(),
            "after": after.map_or(Bson::Null, |d| Bson::Document(d.clone())),
        };
        if upserted {
            line.insert("upserted", true);
        }
        lines.push(line);
    }
    Ok(lines)
}

/// Restores the snapshot's documents. An update needs `after` (see [`capture_after`]); a delete
/// does not. A lost connection aborts with `ConnectionInterrupted`: some documents may already
/// be restored.
pub async fn run_undo(
    store: &dyn DocumentStore,
    snapshot: &Path,
    after: Option<&Path>,
) -> Result<UndoReport> {
    let reader = SnapshotReader::open(snapshot)?;
    let header = reader.header().clone();
    let mut report = UndoReport::default();
    match header.kind {
        SnapshotKind::Delete { .. } => undo_delete(store, &header, reader, &mut report).await?,
        SnapshotKind::Update { .. } => {
            let after = after.ok_or(CoreError::SnapshotUnreadable)?;
            undo_update(store, &header, reader, after, &mut report).await?;
        }
    }
    Ok(report)
}

async fn undo_delete(
    store: &dyn DocumentStore,
    header: &SnapshotHeader,
    mut reader: SnapshotReader,
    report: &mut UndoReport,
) -> Result<()> {
    loop {
        let (next, originals) = read_batch(reader, BATCH).await?;
        reader = next;
        if originals.is_empty() {
            return Ok(());
        }
        let items: Vec<UndoItem> = originals
            .into_iter()
            .map(|original| UndoItem::Reinsert { original })
            .collect();
        apply(store, header, &items, report).await?;
    }
}

async fn undo_update(
    store: &dyn DocumentStore,
    header: &SnapshotHeader,
    mut originals: SnapshotReader,
    after: &Path,
    report: &mut UndoReport,
) -> Result<()> {
    let mut afters = SnapshotReader::open(after)?;
    loop {
        let (next, batch) = read_batch(originals, BATCH).await?;
        originals = next;
        if batch.is_empty() {
            break;
        }
        let (next, lines) = read_batch(afters, batch.len()).await?;
        afters = next;
        if lines.len() != batch.len() {
            return Err(CoreError::SnapshotUnreadable);
        }
        let mut items = Vec::with_capacity(batch.len());
        for (original, line) in batch.into_iter().zip(lines) {
            if line.get("_id") != original.get("_id") {
                return Err(CoreError::SnapshotUnreadable);
            }
            match line.get("after") {
                Some(Bson::Document(expected)) if *expected == original => report.unchanged += 1,
                Some(Bson::Document(expected)) => items.push(UndoItem::Restore {
                    original,
                    expected: expected.clone(),
                }),
                Some(Bson::Null) => report.skipped_changed += 1,
                _ => return Err(CoreError::SnapshotUnreadable),
            }
        }
        apply(store, header, &items, report).await?;
    }
    // What is left in the after-state file are the upserted documents.
    loop {
        let (next, lines) = read_batch(afters, BATCH).await?;
        afters = next;
        if lines.is_empty() {
            return Ok(());
        }
        let items: Vec<UndoItem> = lines
            .iter()
            .filter_map(|line| match line.get("after") {
                Some(Bson::Document(expected)) => Some(UndoItem::Remove {
                    expected: expected.clone(),
                }),
                _ => None,
            })
            .collect();
        apply(store, header, &items, report).await?;
    }
}

async fn apply(
    store: &dyn DocumentStore,
    header: &SnapshotHeader,
    items: &[UndoItem],
    report: &mut UndoReport,
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let results = store.apply_undo(&header.ns, items).await?;
    if results.len() != items.len() {
        return Err(CoreError::SnapshotUnreadable);
    }
    results.into_iter().for_each(|r| report.add(r));
    Ok(())
}
