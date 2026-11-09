//! The snapshot directory as a whole: what is there, removing a run's files, and expiry.
//! Only headers are read, never documents.

use std::path::{Path, PathBuf};

use super::file::{SnapshotKind, SnapshotReader};
use crate::bulk::BulkRunId;
use crate::error::{CoreError, Result};
use crate::store::Namespace;

/// How long a snapshot is kept when nobody deletes it first.
pub const RETENTION_SECS: i64 = 14 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotOp {
    Update,
    Delete,
}

/// One saved run, as the management list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapshotInfo {
    pub run: BulkRunId,
    pub ns: Namespace,
    pub op: SnapshotOp,
    pub created_unix: i64,
    /// Size on disk of the snapshot and its after-state file together.
    pub bytes: u64,
}

pub fn snapshot_path(dir: &Path, run: BulkRunId) -> PathBuf {
    dir.join(format!("{}.jsonl", run.0))
}

fn companion(path: &Path) -> PathBuf {
    path.with_extension("after.jsonl")
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

/// Every readable snapshot in `dir`, newest first. A missing directory, a foreign file or a
/// damaged snapshot is skipped, not an error.
pub fn list(dir: &Path) -> Vec<SnapshotInfo> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<SnapshotInfo> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_snapshot_name(p))
        .filter_map(|p| {
            let reader = SnapshotReader::open(&p).ok()?;
            let header = reader.header();
            Some(SnapshotInfo {
                run: header.run,
                ns: header.ns.clone(),
                op: match header.kind {
                    SnapshotKind::Update { .. } => SnapshotOp::Update,
                    SnapshotKind::Delete { .. } => SnapshotOp::Delete,
                },
                created_unix: header.created_unix,
                bytes: size(&p) + size(&companion(&p)),
            })
        })
        .collect();
    found.sort_by_key(|s| std::cmp::Reverse(s.created_unix));
    found
}

/// `<uuid>.jsonl`, not the `<uuid>.after.jsonl` companion.
fn is_snapshot_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".jsonl"))
        .is_some_and(|stem| stem.parse::<uuid::Uuid>().is_ok())
}

/// Removes a run's snapshot and after-state files. A run with no files is already deleted.
pub fn delete(dir: &Path, run: BulkRunId) -> Result<()> {
    let path = snapshot_path(dir, run);
    for file in [path.clone(), companion(&path)] {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CoreError::SnapshotWriteFailed),
        }
    }
    Ok(())
}

/// Removes every snapshot created more than `max_age_secs` before `now_unix`; returns how many.
pub fn purge_older_than(dir: &Path, now_unix: i64, max_age_secs: i64) -> usize {
    list(dir)
        .into_iter()
        .filter(|s| now_unix.saturating_sub(s.created_unix) > max_age_secs)
        .filter(|s| delete(dir, s.run).is_ok())
        .count()
}
