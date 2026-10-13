//! The messages of the compare feature, wrapped by `runtime::Command::Compare` /
//! `runtime::Event::Compare`. Pure data: no behaviour.

use std::path::PathBuf;

use bson::Document;

use super::apply::{ApplyKind, ApplyOp, ApplyReport, Direction, IdPolicy};
use super::engine::Summary;
use super::key::{KeySpec, KeyTuple};
use super::results::Category;
use super::rules::CompareRules;
use crate::connection::ConnectionConfig;
use crate::error::CoreError;
use crate::store::{CollectionKind, DuplicateGroup, IndexStatus, Namespace};

pub type SessionId = u64;
pub type PairId = u32;
pub type RunId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Source,
    Target,
}

#[derive(Clone, Debug)]
pub struct PairSpec {
    pub id: PairId,
    pub source: Namespace,
    pub target: Namespace,
    pub source_filter: Document,
    pub target_filter: Document,
    pub key: KeySpec,
    pub rules: CompareRules,
}

/// Most groups shown per side; the runtime asks the store for one more to learn whether there
/// are more.
pub const DUPLICATE_KEYS_LIMIT: u32 = 20;

/// The most frequent duplicate key groups of one side; `more` when there are further groups.
#[derive(Clone, Debug, PartialEq)]
pub struct DuplicateReport {
    pub groups: Vec<DuplicateGroup>,
    pub more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preflight {
    pub source_count: u64,
    pub target_count: u64,
    pub source_index: IndexStatus,
    pub target_index: IndexStatus,
    /// A side has no usable index and is above `max_unindexed_documents`.
    pub unindexed_too_large: bool,
}

#[derive(Clone, Debug)]
pub enum CompareCommand {
    Open {
        session: SessionId,
        source: ConnectionConfig,
        target: ConnectionConfig,
    },
    Databases {
        session: SessionId,
        side: Side,
    },
    Collections {
        session: SessionId,
        side: Side,
        db: String,
    },
    Check {
        session: SessionId,
        pair: PairSpec,
    },
    Start {
        session: SessionId,
        run: RunId,
        pairs: Vec<PairSpec>,
    },
    Cancel {
        session: SessionId,
        run: RunId,
    },
    Rows {
        session: SessionId,
        pair: PairId,
        category: Category,
        offset: usize,
        limit: usize,
    },
    Detail {
        session: SessionId,
        pair: PairId,
        key: KeyTuple,
    },
    Discard {
        session: SessionId,
        pair: PairId,
    },
    /// Which key tuples occur more than once on each side of the pair.
    DuplicateKeys {
        session: SessionId,
        pair: PairSpec,
    },
    /// Writes the chosen operations into the destination of their pairs. `bulk` items are
    /// expanded by the runtime into one operation per stored row of their category, appended
    /// after `ops`. Answers with `ApplyProgress`, one `ApplyPairDone` per pair and `ApplyDone`
    /// (or `Failed` when refused).
    Apply {
        session: SessionId,
        run: RunId,
        ops: Vec<ApplyOp>,
        bulk: Vec<BulkApply>,
        id_policy: IdPolicy,
    },
    /// Writes the pending changes, planned like an `Apply` would but never applied, to the file
    /// at `path` (absolute, existing parent directory, not a directory) as a mongosh script or a
    /// JSON plan. `ops` and `bulk` have the shapes of `Apply`. Answers with `ExportProgress`*
    /// and one `ExportDone`, or `Failed { context: "compare_export" }` when refused or when the
    /// file could not be written (the partial file is removed in every case).
    Export {
        session: SessionId,
        run: RunId,
        request: ExportRequest,
        path: PathBuf,
    },
    /// Writes a CSV report of the stored differences of `request.pairs` to a file (same file
    /// rules and answers as `Export`) or, for the clipboard, returns it as `ExportText` before
    /// the one `ExportDone` (rows in `written`). Refused as `Failed { context: "compare_export" }`
    /// when a pair has no results, no pair is named, the path is unusable or the clipboard
    /// report would hold more than `CLIPBOARD_MAX_ROWS` (`export::report`) rows.
    ExportReport {
        session: SessionId,
        run: RunId,
        request: ReportRequest,
        target: ReportTarget,
    },
    Close {
        session: SessionId,
    },
}

/// What an `ExportReport` command asks for. `label` is free text that ends up in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportRequest {
    pub pairs: Vec<PairId>,
    /// Fill `changed_paths` of the different rows (re-reads both documents of each; paths only).
    pub include_paths: bool,
    pub label: String,
}

/// Where a report goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportTarget {
    File(PathBuf),
    Clipboard,
}

/// The file format of an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Mongosh,
    JsonPlan,
}

/// What an `Export` command asks for. `label` is free text that ends up in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct ExportRequest {
    pub format: ExportFormat,
    pub label: String,
    pub id_policy: IdPolicy,
    pub ops: Vec<ApplyOp>,
    pub bulk: Vec<BulkApply>,
}

/// How an export ended. `skipped` is stale + not planned + unsupported (listed in the file, never
/// written as operations); `failed` are operations that could not be verified. When `cancelled`
/// (or the file could not be kept) the counts say nothing was exported and `bytes` is 0.
/// `partial_removed` is true when no export file remains (it was removed, or none was ever
/// created); `file_left` is true only when one remains because its removal failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOutcome {
    pub written: u64,
    pub skipped: u64,
    pub failed: u64,
    pub not_run: u64,
    pub bytes: u64,
    pub cancelled: bool,
    pub partial_removed: bool,
    pub file_left: bool,
}

/// "Every row of this category of this pair" (resolved by the runtime from the stored results).
#[derive(Clone, Debug, PartialEq)]
pub struct BulkApply {
    pub pair: PairId,
    pub category: Category,
    pub kind: ApplyKind,
    pub direction: Direction,
}

/// How one pair ended. A run reports exactly one of these per pair.
#[derive(Clone, Debug, PartialEq)]
pub enum PairOutcome {
    Completed(Summary),
    Cancelled(Summary),
    Failed(CoreError),
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
    pub key: KeyTuple,
    pub has_source: bool,
    pub has_target: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CompareEvent {
    Opened {
        session: SessionId,
    },
    OpenFailed {
        session: SessionId,
        error: CoreError,
    },
    Databases {
        session: SessionId,
        side: Side,
        names: Vec<String>,
    },
    Collections {
        session: SessionId,
        side: Side,
        db: String,
        entries: Vec<(String, CollectionKind)>,
    },
    Checked {
        session: SessionId,
        pair: PairId,
        preflight: Preflight,
    },
    Progress {
        session: SessionId,
        run: RunId,
        pair: PairId,
        scanned_source: u64,
        scanned_target: u64,
        differences: u64,
    },
    PairDone {
        session: SessionId,
        run: RunId,
        pair: PairId,
        outcome: PairOutcome,
    },
    RunDone {
        session: SessionId,
        run: RunId,
    },
    ApplyProgress {
        session: SessionId,
        run: RunId,
        pair: PairId,
        done: u64,
        total: u64,
    },
    /// The outcome of one pair's operations. Failures carry keys: show them, never log them.
    ApplyPairDone {
        session: SessionId,
        run: RunId,
        pair: PairId,
        report: ApplyReport,
    },
    ApplyDone {
        session: SessionId,
        run: RunId,
    },
    ExportProgress {
        session: SessionId,
        run: RunId,
        done: u64,
        total: u64,
    },
    ExportDone {
        session: SessionId,
        run: RunId,
        outcome: ExportOutcome,
    },
    /// The clipboard report: carries keys (personal data possibly); sent once, before
    /// `ExportDone`. Show or copy it, never log it.
    ExportText {
        session: SessionId,
        run: RunId,
        text: String,
    },
    Rows {
        session: SessionId,
        pair: PairId,
        category: Category,
        offset: usize,
        total: usize,
        rows: Vec<RowView>,
    },
    Detail {
        session: SessionId,
        pair: PairId,
        key: KeyTuple,
        source: Option<Document>,
        target: Option<Document>,
        changes: Vec<super::diff::FieldChange>,
    },
    /// Duplicate key groups of both sides. Carries key values: show them, never log them.
    DuplicateKeys {
        session: SessionId,
        pair: PairId,
        source: DuplicateReport,
        target: DuplicateReport,
    },
    /// A command failed as a whole. `context` is an i18n suffix such as `"compare_check"`.
    Failed {
        session: SessionId,
        context: &'static str,
        error: CoreError,
    },
}
