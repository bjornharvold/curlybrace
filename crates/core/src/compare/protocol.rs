//! The messages of the compare feature, wrapped by `runtime::Command::Compare` /
//! `runtime::Event::Compare`. Pure data: no behaviour.

use bson::Document;

use super::apply::{ApplyOp, ApplyReport, IdPolicy};
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
    /// Writes the chosen operations into the destination of their pairs. Answers with
    /// `ApplyProgress`, one `ApplyPairDone` per pair and `ApplyDone` (or `Failed` when refused).
    Apply {
        session: SessionId,
        run: RunId,
        ops: Vec<ApplyOp>,
        id_policy: IdPolicy,
    },
    Close {
        session: SessionId,
    },
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
