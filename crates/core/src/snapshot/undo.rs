//! Undo of a bulk write, from its snapshot.
//!
//! The snapshot holds the originals. Right after the write, [`capture_after`] saves what the
//! touched documents look like (one line per original, in the same order, then the upserted
//! documents). Undo restores an original only when the document still equals that saved
//! after-state, and the server checks that in the same operation that replaces it, so an edit
//! made in between is never overwritten.

use std::path::{Path, PathBuf};

use bson::{Bson, Document, doc};

use super::file::{SnapshotHeader, SnapshotKind, SnapshotReader, SnapshotWriter};
use crate::error::{CoreError, Result};
use crate::store::DocumentStore;

/// Documents handled per round trip.
const BATCH: usize = 500;

/// One document's undo, applied by the server as a single compare-and-swap.
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

fn after_header(header: &SnapshotHeader) -> SnapshotHeader {
    header.clone()
}

/// Saves the current state of every document in the snapshot, plus the `upserted` documents.
/// Call it right after the write finished, whatever its outcome.
pub async fn capture_after(
    store: &dyn DocumentStore,
    snapshot: &Path,
    upserted: &[Bson],
    max_bytes: u64,
) -> Result<()> {
    let reader = SnapshotReader::open(snapshot)?;
    let header = reader.header().clone();
    let dir = snapshot.parent().ok_or(CoreError::SnapshotWriteFailed)?;
    let name = after_path_for(snapshot);
    let name = name
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(CoreError::SnapshotWriteFailed)?;
    let mut out = SnapshotWriter::create_as(dir, name, after_header(&header), max_bytes)?;
    let ns = header.ns.clone();

    let mut batch: Vec<Bson> = Vec::with_capacity(BATCH);
    for original in reader {
        batch.push(id_of(&original?)?);
        if batch.len() == BATCH {
            write_after(store, &ns, &batch, false, &mut out, true).await?;
            batch.clear();
        }
    }
    write_after(store, &ns, &batch, false, &mut out, true).await?;
    for chunk in upserted.chunks(BATCH) {
        write_after(store, &ns, chunk, true, &mut out, false).await?;
    }
    out.finish().map(|_| ())
}

/// Writes the after-state of `ids`. With `all`, every id gets a line (`after: null` when the
/// document is gone), keeping the file aligned with the snapshot; otherwise only found ones.
async fn write_after(
    store: &dyn DocumentStore,
    ns: &crate::store::Namespace,
    ids: &[Bson],
    upserted: bool,
    out: &mut SnapshotWriter,
    all: bool,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let found: std::collections::HashMap<String, Document> = store
        .find_by_ids(ns, ids)
        .await?
        .into_iter()
        .filter_map(|d| Some((id_key(d.get("_id")?), d)))
        .collect();
    for id in ids {
        let after = found.get(&id_key(id));
        if after.is_none() && !all {
            continue;
        }
        let mut line = doc! { "_id": id.clone(), "after": after.map_or(Bson::Null, |d| Bson::Document(d.clone())) };
        if upserted {
            line.insert("upserted", true);
        }
        out.push(&line)?;
    }
    Ok(())
}

/// Restores the snapshot's documents. An update needs `after` (see [`capture_after`]); a delete
/// does not.
pub async fn run_undo(
    store: &dyn DocumentStore,
    snapshot: &Path,
    after: Option<&Path>,
) -> Result<UndoReport> {
    let reader = SnapshotReader::open(snapshot)?;
    let header = reader.header().clone();
    let mut report = UndoReport::default();
    match header.kind {
        SnapshotKind::Delete { .. } => {
            let mut items = Vec::with_capacity(BATCH);
            for original in reader {
                items.push(UndoItem::Reinsert {
                    original: original?,
                });
                if items.len() == BATCH {
                    apply(store, &header, &mut items, &mut report).await?;
                }
            }
            apply(store, &header, &mut items, &mut report).await?;
        }
        SnapshotKind::Update { .. } => {
            let after = after.ok_or(CoreError::SnapshotUnreadable)?;
            undo_update(store, &header, reader, after, &mut report).await?;
        }
    }
    Ok(report)
}

async fn undo_update(
    store: &dyn DocumentStore,
    header: &SnapshotHeader,
    mut originals: SnapshotReader,
    after: &Path,
    report: &mut UndoReport,
) -> Result<()> {
    let mut afters = SnapshotReader::open(after)?;
    let mut items = Vec::with_capacity(BATCH);
    for original in originals.by_ref() {
        let original = original?;
        let line = afters.next().ok_or(CoreError::SnapshotUnreadable)??;
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
        if items.len() == BATCH {
            apply(store, header, &mut items, report).await?;
        }
    }
    apply(store, header, &mut items, report).await?;
    // What is left in the after-state file are the upserted documents.
    for line in afters {
        let line = line?;
        if let Some(Bson::Document(expected)) = line.get("after") {
            items.push(UndoItem::Remove {
                expected: expected.clone(),
            });
        }
        if items.len() == BATCH {
            apply(store, header, &mut items, report).await?;
        }
    }
    apply(store, header, &mut items, report).await
}

async fn apply(
    store: &dyn DocumentStore,
    header: &SnapshotHeader,
    items: &mut Vec<UndoItem>,
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
    items.clear();
    Ok(())
}
