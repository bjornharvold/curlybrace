//! Undo snapshots of a bulk write: the original version of every document it will touch,
//! saved to a local file before the write runs.
//!
//! The file holds the user's documents in clear text, so it is created with owner-only
//! permissions, its contents are never logged, and errors carry no path or document value.

mod file;
mod library;
mod undo;

#[cfg(test)]
mod file_tests;
#[cfg(test)]
mod library_tests;
#[cfg(test)]
mod undo_tests;

pub use file::{SnapshotHeader, SnapshotKind, SnapshotReader, SnapshotWriter};
pub use library::{
    RETENTION_SECS, SnapshotInfo, SnapshotOp, delete, list, purge_older_than, snapshot_path,
};
pub use undo::{ItemResult, UndoItem, UndoReport, after_path_for, capture_after, id_key, run_undo};
