//! The messages of the compare feature, wrapped by `runtime::Command::Compare` /
//! `runtime::Event::Compare`. Pure data: no behaviour.

use std::path::{Path, PathBuf};

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

// Commands are short-lived, moved once into a channel; boxing would ripple through every call site.
#[allow(clippy::large_enum_variant)]
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
        path: ExportPath,
    },
    /// Writes a CSV report of the stored differences of `request.pairs` to a file (same file
    /// rules and answers as `Export`) or, for the clipboard, returns it as `ExportText` before
    /// the one `ExportDone` (rows in `written`). Refused as `Failed { context: "compare_export" }`
    /// when a pair has no results, no pair is named, the path is unusable or the clipboard
    /// report would hold more than `CLIPBOARD_MAX_ROWS` (`export::report`) rows.
    /// The cap counts rows; with `include_paths` one row can carry many paths, so the
    /// `changed_paths` cell is cut at 200 paths plus a `"..."` element. The outcome counts are
    /// those of a report (see `ExportOutcome`: `skipped` and `failed` are inside `written`).
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

/// The file an export writes. A path can hold a user name: `Debug` never prints it, so a `{:?}` of a
/// command can not leak it into a log.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportPath(PathBuf);

impl ExportPath {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

impl From<PathBuf> for ExportPath {
    fn from(path: PathBuf) -> Self {
        Self(path)
    }
}

impl std::fmt::Debug for ExportPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExportPath(..)")
    }
}

/// The clipboard report. It lists keys, which can be personal data: `Debug` prints only its size.
#[derive(Clone, PartialEq, Eq)]
pub struct ReportText(String);

impl ReportText {
    pub fn new(text: String) -> Self {
        Self(text)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<&str> for ReportText {
    fn from(text: &str) -> Self {
        Self(text.to_string())
    }
}

impl From<String> for ReportText {
    fn from(text: String) -> Self {
        Self(text)
    }
}

impl std::fmt::Debug for ReportText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ReportText(<{} bytes>)", self.0.len())
    }
}

#[cfg(test)]
mod redaction_tests {
    use super::*;

    #[test]
    fn debug_output_never_carries_the_path_or_the_report() {
        let path = ExportPath::new("/Users/secret-name/plan.js");
        let text = ReportText::new(
            "pair,key
1,alice@example.com"
                .into(),
        );
        for shown in [
            format!("{path:?}"),
            format!("{text:?}"),
            format!(
                "{:?}",
                CompareCommand::ExportReport {
                    session: 1,
                    run: 2,
                    request: ReportRequest {
                        pairs: vec![1],
                        include_paths: false,
                        label: String::new()
                    },
                    target: ReportTarget::File(path.clone()),
                }
            ),
            format!("{:?}", ReportTarget::File(path.clone())),
            format!(
                "{:?}",
                CompareEvent::ExportText {
                    session: 1,
                    run: 2,
                    text: text.clone()
                }
            ),
        ] {
            assert!(
                !shown.contains("secret-name") && !shown.contains("alice"),
                "{shown}"
            );
        }
        assert_eq!(path.as_path(), Path::new("/Users/secret-name/plan.js"));
        assert_eq!(
            text.as_str(),
            "pair,key
1,alice@example.com"
        );
    }
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
    File(ExportPath),
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
///
/// For a REPORT the counts mean something else: `written` is the rows written; `skipped` is the
/// rows whose documents changed since the compare (stale, or a fresh diff with no paths;
/// `changed_paths` left empty) and `failed` the rows whose documents could not be read (empty
/// cell). Both are counted INSIDE `written` (those rows are still listed), whereas for a plan
/// `written`, `skipped` and `failed` are separate groups of operations.
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
        text: ReportText,
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
