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

/// Removes a run's snapshot and after-state files. A run with no files is already deleted. The
/// after-state goes first, so a failure leaves the readable snapshot in place: it stays listed,
/// and whoever asked for the deletion can tell from the listing that it did not happen.
pub fn delete(dir: &Path, run: BulkRunId) -> Result<()> {
    let path = snapshot_path(dir, run);
    for file in [companion(&path), path.clone()] {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CoreError::SnapshotDeleteFailed),
        }
    }
    Ok(())
}

/// Removes every snapshot created more than `max_age_secs` before `now_unix`; returns how many
/// files or snapshots went. A file of ours that is not a readable snapshot (torn by a crash, or
/// an after-state whose snapshot is gone) is never listed, so it expires by its modification
/// time instead: it holds the person's documents all the same.
pub fn purge_older_than(dir: &Path, now_unix: i64, max_age_secs: i64) -> usize {
    let (old, kept): (Vec<_>, Vec<_>) = list(dir)
        .into_iter()
        .partition(|s| now_unix.saturating_sub(s.created_unix) > max_age_secs);
    let snapshots = old.iter().filter(|s| delete(dir, s.run).is_ok()).count();
    // A readable snapshot lives by its header's date, never by its file time.
    let readable: std::collections::HashSet<uuid::Uuid> = kept.iter().map(|s| s.run.0).collect();
    snapshots + purge_stray_files(dir, now_unix, max_age_secs, &readable)
}

/// Our files (`<uuid>.jsonl`, `<uuid>.after.jsonl`) older than the limit by modification time.
fn purge_stray_files(
    dir: &Path,
    now_unix: i64,
    max_age_secs: i64,
    readable: &std::collections::HashSet<uuid::Uuid>,
) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_our_file(p))
        .filter(|p| !stem_uuid(p).is_some_and(|run| readable.contains(&run)))
        .filter(|p| age_secs(p, now_unix).is_some_and(|age| age > max_age_secs))
        .filter(|p| std::fs::remove_file(p).is_ok())
        .count()
}

/// The run id in `<uuid>.jsonl` or `<uuid>.after.jsonl`.
fn stem_uuid(path: &Path) -> Option<uuid::Uuid> {
    let name = path.file_name()?.to_str()?;
    name.strip_suffix(".after.jsonl")
        .or_else(|| name.strip_suffix(".jsonl"))?
        .parse()
        .ok()
}

fn is_our_file(path: &Path) -> bool {
    stem_uuid(path).is_some()
}

fn age_secs(path: &Path, now_unix: i64) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let modified = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(now_unix.saturating_sub(i64::try_from(modified.as_secs()).ok()?))
}
