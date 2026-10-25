//! The commands and events of a copy run between two open connections.
//!
//! Unlike a compare run, a copy works on the stores of the sessions that are already open (named
//! by their `SessionId`, the connection id), so a read-only connection stays read-only. The runtime
//! module `runtime::copy` documents the order and the guarantees of the events below.
//!
//! No event carries a document value. `CopyFailure::key` inside `ItemDone` is the `_id` of a
//! refused document and is for the UI to display; it is never logged.

use bson::Document;

use super::engine::{CopyProgress, CopyReport};
use super::policy::ConflictPolicy;
use super::preview::PreviewCounts;
use crate::compare::protocol::RunId;
use crate::error::CoreError;
use crate::runtime::SessionId;
use crate::store::Namespace;

/// One collection to copy: where it is read, where it is written and which documents.
#[derive(Clone, Debug, PartialEq)]
pub struct CopyItem {
    pub source: Namespace,
    pub target: Namespace,
    /// Which source documents to copy; empty means the whole collection.
    pub filter: Document,
}

/// What a run copies.
#[derive(Clone, Debug, PartialEq)]
pub enum CopyScope {
    /// Exactly these collections, in this order.
    Collections(Vec<CopyItem>),
    /// Every plain collection of `source_db`, listed when the run starts (server-owned
    /// collections and views are left out), into the same names in `target_db`.
    Database {
        source_db: String,
        target_db: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyRequest {
    pub source: SessionId,
    pub target: SessionId,
    pub scope: CopyScope,
    pub policy: ConflictPolicy,
    /// Whether collection options (first) and indexes (last) are copied too. The result is in
    /// `CopyReport::structure` and `CopyReport::indexes` of each `ItemDone`.
    pub with_indexes_and_options: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CopyCommand {
    /// Counts what the copy would do, per collection. Writes nothing.
    Preview { run: RunId, request: CopyRequest },
    /// Copies. Only one run (preview or copy) is active per runtime.
    Start { run: RunId, request: CopyRequest },
    /// Ends the run `run` after the item in progress reported its counts; unknown runs are ignored.
    Cancel { run: RunId },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CopyEvent {
    /// The first event of every accepted run: the collections it will go through, by `index`.
    Planned {
        run: RunId,
        items: Vec<CopyItem>,
    },
    PreviewProgress {
        run: RunId,
        index: usize,
        scanned: u64,
    },
    /// `counts` is `None` when they are unavailable because the compare result cap was exceeded;
    /// that never fails the preview.
    PreviewItemDone {
        run: RunId,
        index: usize,
        counts: Option<PreviewCounts>,
    },
    PreviewDone {
        run: RunId,
    },
    Progress {
        run: RunId,
        index: usize,
        progress: CopyProgress,
        /// The collection's estimated document count, when the whole collection is copied.
        estimated_total: Option<u64>,
    },
    /// The exact counts of one collection, whatever its outcome.
    ItemDone {
        run: RunId,
        index: usize,
        report: CopyReport,
    },
    /// The run ended without a failure: every item finished, or `cancelled` ended it early.
    Done {
        run: RunId,
        cancelled: bool,
    },
    /// The run was refused before any read or write, or ended with an error.
    Failed {
        run: RunId,
        error: CoreError,
    },
}
