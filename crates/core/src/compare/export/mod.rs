//! Pure renderers for the data-compare export: mongosh scripts, JSON plans and CSV reports,
//! plus the sanitising helpers they share. No I/O except through `std::io::Write`.
//!
//! Safety rules (see the slice 3 plan, rulings 5-8):
//! * names, labels and values that reach code are written as string literals
//!   ([`sanitize::js_string`]); the ones that reach a comment pass [`sanitize::comment_text`];
//! * errors never carry a value, a name or a key;
//! * every writer is deterministic: the same input gives the same bytes.

pub mod csv;
pub mod json_plan;
pub mod mongosh;
pub mod plan;
pub mod report;
pub mod sanitize;

use std::io;

use thiserror::Error;

use crate::compare::apply::{ApplyKind, Direction, IdPolicy, SkipReason};
use crate::compare::fingerprint::Fingerprint;
use crate::compare::key::KeyTuple;
use crate::compare::protocol::Side;

/// Why something could not be rendered. Value-free by design: no name, key or document text.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum RenderError {
    /// A value or path the target format cannot represent faithfully.
    #[error("a value or path cannot be rendered")]
    Unrenderable,
    /// A database or collection name that cannot be addressed.
    #[error("a database or collection name cannot be rendered")]
    BadName,
    /// A writer method called out of order (a programming error).
    #[error("export writer used out of order")]
    Sequence,
    /// The sink failed; only the error kind is kept (messages can contain paths).
    #[error("writing the export failed")]
    Io(io::ErrorKind),
}

impl From<io::Error> for RenderError {
    fn from(e: io::Error) -> Self {
        Self::Io(e.kind())
    }
}

/// How many operations of each kind an export holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportCounts {
    pub insert: u64,
    pub replace: u64,
    pub delete: u64,
    pub update_fields: u64,
    /// Operations that are listed but not exported as writes.
    pub skipped: u64,
}

/// The top of a script or plan. `label` is free text from the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportHeader {
    pub label: String,
    pub counts: ExportCounts,
    pub id_policy: IdPolicy,
    /// Counts are space-padded to this width, so a header with other counts (and the other
    /// `complete` value) has the SAME length and can overwrite it in place.
    pub count_width: usize,
    /// False while the export is being written: the script then starts with `quit(1);` and the
    /// plan says `"complete":false`, so a file cut short can neither run nor pass as whole.
    pub complete: bool,
    /// The side all writes of a script go to (scripts only; one side per script).
    pub destination: Option<Side>,
}

/// Digits needed to print `n` (at least 1): the count width for an export of `n` operations.
pub fn count_width(n: u64) -> usize {
    n.to_string().len()
}

/// What every exported operation says about where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct OpMeta {
    pub direction: Direction,
    pub key: KeyTuple,
    pub source: Option<Fingerprint>,
    pub target: Option<Fingerprint>,
}

/// Stable lowercase text for a fingerprint (hex), `None` when absent.
pub(crate) fn fingerprint_hex(fp: &Fingerprint) -> String {
    fp.0.iter().map(|b| format!("{b:02x}")).collect()
}

/// Name of an apply kind as written in plans and comments.
pub(crate) fn kind_name(kind: ApplyKind) -> &'static str {
    match kind {
        ApplyKind::Insert => "insert",
        ApplyKind::Replace => "replace",
        ApplyKind::Delete => "delete",
        ApplyKind::Fields => "updateFields",
    }
}

/// Name of a direction as written in plans and comments.
pub(crate) fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::SourceToTarget => "sourceToTarget",
        Direction::TargetToSource => "targetToSource",
    }
}

/// Name of an id policy as written in plans and comments.
pub(crate) fn id_policy_name(policy: IdPolicy) -> &'static str {
    match policy {
        IdPolicy::Keep => "keep",
        IdPolicy::Generate => "generate",
    }
}

/// Why an operation is listed but not exported as a write. `Stale` and `NotPlanned` are the
/// apply planner's own reasons; `Unsupported` is export-only (a value or path the shell cannot
/// represent), so the apply engine's [`SkipReason`] keeps its meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportSkip {
    Stale,
    NotPlanned,
    Unsupported,
    /// A document could not be read while verifying the operation.
    Unreadable,
}

impl From<SkipReason> for ExportSkip {
    fn from(reason: SkipReason) -> Self {
        match reason {
            SkipReason::Stale => Self::Stale,
            SkipReason::NotPlanned => Self::NotPlanned,
        }
    }
}

/// Name of a skip reason as written in plans and comments.
pub(crate) fn skip_name(reason: ExportSkip) -> &'static str {
    match reason {
        ExportSkip::Stale => "stale",
        ExportSkip::NotPlanned => "notPlanned",
        ExportSkip::Unsupported => "unsupported",
        ExportSkip::Unreadable => "unreadable",
    }
}

/// Which side of the comparison an operation writes to, as written in plans.
pub(crate) fn destination_name(direction: Direction) -> &'static str {
    match direction {
        Direction::SourceToTarget => "target",
        Direction::TargetToSource => "source",
    }
}

/// The destination comment of a script.
pub(crate) fn destination_comment(side: Side) -> &'static str {
    match side {
        Side::Target => "// destination: target side of the comparison",
        Side::Source => "// destination: source side of the comparison",
    }
}
