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
pub mod sanitize;

use std::io;

use thiserror::Error;

use crate::compare::apply::{ApplyKind, Direction, IdPolicy};
use crate::compare::fingerprint::Fingerprint;
use crate::compare::key::KeyTuple;

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

/// Name of a skip reason as written in plans and comments.
pub(crate) fn skip_name(reason: crate::compare::apply::SkipReason) -> &'static str {
    use crate::compare::apply::SkipReason;
    match reason {
        SkipReason::Stale => "stale",
        SkipReason::NotPlanned => "notPlanned",
    }
}
