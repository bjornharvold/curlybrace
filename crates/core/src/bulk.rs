//! Request types, validation and outcomes for bulk update and delete.
//!
//! Pure data and checks: nothing here touches a store. Errors carry no document
//! values; they expose fixed i18n keys only.

use bson::{Bson, Document};
use serde_json::Value;

use crate::ejson::editor_json_to_doc;
use crate::error::{CoreError, Result};

/// How many documents the update preview shows.
pub const BULK_PREVIEW_DOCS: usize = 3;

/// Pipeline stages the server accepts in an update pipeline.
const PIPELINE_STAGES: [&str; 6] = [
    "$set",
    "$addFields",
    "$unset",
    "$project",
    "$replaceRoot",
    "$replaceWith",
];

/// Whether a request is meant to hit the documents a filter selects, or the whole collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchScope {
    Filtered,
    AllDocuments,
}

/// The update to apply: an operator document or an aggregation-style pipeline.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateSpec {
    Operators(Document),
    Pipeline(Vec<Document>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BulkUpdateRequest {
    pub filter: Document,
    pub update: UpdateSpec,
    pub upsert: bool,
    pub scope: MatchScope,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BulkDeleteRequest {
    pub filter: Document,
    pub scope: MatchScope,
}

/// What a cancel request did on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelEffect {
    /// An operation of the run was found and a kill was sent for it. The write's own finish
    /// still comes (as `Unknown { cancelled: true }` when the kill landed).
    Stopped,
    /// No operation of the run is registered on the server: it already finished, or it has not
    /// started yet (it may still be waiting for a connection or in flight). Nothing was killed.
    NothingToStop,
}

/// Whether a bulk write saves the originals of its documents first, so it can be undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoPolicy {
    /// Save a snapshot before the write; if that fails or is too large, the write does not run.
    Snapshot,
    /// Run without a snapshot. The person chose this knowing it cannot be undone.
    None,
}

/// What a finished run can be undone with. Carried by the finish event of a run that left a
/// snapshot behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoOffer {
    /// Documents the snapshot holds (the run's matches), not counting upserted ones.
    pub documents: u64,
    /// Documents the write changed that the snapshot does not hold: they matched the filter
    /// only after the snapshot was taken, so undo cannot bring them back. Usually 0.
    pub uncovered: u64,
}

/// What a finished run can be undone with, and why not when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoStatus {
    /// The run asked for no snapshot (or never got as far as one).
    NotRequested,
    /// A snapshot was asked for but the run touched nothing (or never wrote): nothing to undo.
    NothingToUndo,
    /// The write ran but the snapshot could not be completed (for example the saved state of an
    /// update did not fit the size cap): it can NOT be undone, although undo was asked for.
    Lost,
    Offered(UndoOffer),
}

/// A document the undo would restore: what it was, and what it is now.
#[derive(Debug, Clone, PartialEq)]
pub struct UndoPreviewDoc {
    pub original: Document,
    /// `None` when the document does not exist now.
    pub current: Option<Document>,
}

/// What an undo would touch: the size of the snapshot and its first documents.
#[derive(Debug, Clone, PartialEq)]
pub struct UndoPreview {
    pub documents: u64,
    pub docs: Vec<UndoPreviewDoc>,
}

/// Identifies one bulk run, so a cancel or a late result can be matched to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkRunId(pub uuid::Uuid);

#[derive(Debug, Clone, PartialEq)]
pub enum BulkOutcome {
    Applied {
        matched: u64,
        modified: u64,
        upserted: Vec<Bson>,
        deleted: u64,
    },
    /// The result is not known: the run was cut short or the connection was lost.
    Unknown { cancelled: bool },
    /// The write was applied and its counts are known, but the server reported a write concern
    /// error: it could not confirm the write reached the requested durability (for example a
    /// majority acknowledgement timed out). Carries the same counts as `Applied`.
    Unconfirmed {
        matched: u64,
        modified: u64,
        upserted: Vec<Bson>,
        deleted: u64,
    },
    /// The server answered with an error for the write. The reply's counts do not cover the
    /// documents changed before the failure (observed: `n` and `nModified` read 0 after several
    /// documents were written), so none are reported: some documents MAY already be changed.
    Rejected { reason: BulkFailureReason },
}

/// Why the server rejected a bulk write, from its error code only (never its text, which can
/// quote document values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BulkFailureReason {
    DuplicateKey,
    ValidationFailed,
    ImmutableField,
    TypeMismatch,
    Other,
}

impl BulkFailureReason {
    /// Maps a server write error code: 11000 DuplicateKey, 121 DocumentValidationFailure,
    /// 66 ImmutableField, 14 TypeMismatch.
    pub fn from_code(code: Option<i32>) -> Self {
        match code {
            Some(11000) => Self::DuplicateKey,
            Some(121) => Self::ValidationFailed,
            Some(66) => Self::ImmutableField,
            Some(14) => Self::TypeMismatch,
            _ => Self::Other,
        }
    }

    /// Key into the UI's `en.json`.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::DuplicateKey => "bulk.failure.duplicate_key",
            Self::ValidationFailed => "bulk.failure.validation_failed",
            Self::ImmutableField => "bulk.failure.immutable_field",
            Self::TypeMismatch => "bulk.failure.type_mismatch",
            Self::Other => "bulk.failure.other",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PreviewDoc {
    pub before: Document,
    pub after: Option<Document>,
}

/// Why a preview shows fewer documents than expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewGap {
    TransactionsUnavailable,
    Conflict,
    NoMatch,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdatePreview {
    pub docs: Vec<PreviewDoc>,
    pub gap: Option<PreviewGap>,
}

/// An empty filter needs `AllDocuments`, and `AllDocuments` needs an empty filter, so the
/// flag always means what it says. The parsed document is checked, never the text; any key
/// at all (even `$expr: true`) is an explicit filter.
pub fn check_scope(filter: &Document, scope: MatchScope) -> Result<()> {
    match (filter.is_empty(), scope) {
        (true, MatchScope::Filtered) => Err(CoreError::BulkEmptyFilter),
        (false, MatchScope::AllDocuments) => Err(CoreError::BulkScopeMismatch),
        _ => Ok(()),
    }
}

/// Parses editor JSON into an update: an array is a pipeline, anything else is read as an
/// operator document. The result still goes through [`validate_update`].
pub fn parse_update(text: &str) -> Result<UpdateSpec> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| CoreError::InvalidJson(e.to_string()))?;
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(stage_to_doc)
            .collect::<Result<Vec<_>>>()
            .map(UpdateSpec::Pipeline),
        other => editor_json_to_doc(&other.to_string()).map(UpdateSpec::Operators),
    }
}

fn stage_to_doc(value: Value) -> Result<Document> {
    if !value.is_object() {
        return Err(CoreError::InvalidDocument(
            "expected a JSON object for a pipeline stage".into(),
        ));
    }
    editor_json_to_doc(&value.to_string())
}

/// Checks the update has the shape the server accepts. Never echoes a key or a value.
pub fn validate_update(spec: &UpdateSpec) -> Result<()> {
    let valid = match spec {
        UpdateSpec::Operators(doc) => !doc.is_empty() && doc.keys().all(|key| key.starts_with('$')),
        UpdateSpec::Pipeline(stages) => !stages.is_empty() && stages.iter().all(is_allowed_stage),
    };
    if valid {
        Ok(())
    } else {
        Err(CoreError::BulkInvalidUpdate)
    }
}

fn is_allowed_stage(stage: &Document) -> bool {
    let mut keys = stage.keys();
    matches!(
        (keys.next(), keys.next()),
        (Some(name), None) if PIPELINE_STAGES.contains(&name.as_str())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn failure_reasons_come_from_the_server_code_only() {
        use BulkFailureReason::*;
        let cases = [
            (Some(11000), DuplicateKey),
            (Some(121), ValidationFailed),
            (Some(66), ImmutableField),
            (Some(14), TypeMismatch),
            (Some(2), Other),
            (None, Other),
        ];
        for (code, want) in cases {
            assert_eq!(BulkFailureReason::from_code(code), want, "{code:?}");
        }
    }

    #[test]
    fn every_failure_reason_has_a_distinct_bulk_failure_key() {
        use BulkFailureReason::*;
        let all = [
            DuplicateKey,
            ValidationFailed,
            ImmutableField,
            TypeMismatch,
            Other,
        ];
        let mut keys: Vec<_> = all.iter().map(|r| r.i18n_key()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), all.len());
        assert!(keys.iter().all(|k| k.starts_with("bulk.failure.")));
    }

    #[test]
    fn empty_filter_without_flag_is_rejected() {
        assert_eq!(
            check_scope(&doc! {}, MatchScope::Filtered),
            Err(CoreError::BulkEmptyFilter)
        );
    }

    #[test]
    fn all_documents_with_empty_filter_passes() {
        assert_eq!(check_scope(&doc! {}, MatchScope::AllDocuments), Ok(()));
    }

    #[test]
    fn all_documents_with_nonempty_filter_is_rejected() {
        assert_eq!(
            check_scope(&doc! { "a": 1 }, MatchScope::AllDocuments),
            Err(CoreError::BulkScopeMismatch)
        );
    }

    #[test]
    fn expr_true_filter_counts_as_explicit() {
        assert_eq!(
            check_scope(&doc! { "$expr": true }, MatchScope::Filtered),
            Ok(())
        );
    }

    #[test]
    fn operators_update_requires_every_key_to_be_an_operator() {
        let check = |d| validate_update(&UpdateSpec::Operators(d));
        assert_eq!(check(doc! { "a": 1 }), Err(CoreError::BulkInvalidUpdate));
        assert_eq!(check(doc! { "$set": { "a": 1 } }), Ok(()));
        assert_eq!(
            check(doc! { "$set": { "a": 1 }, "b": 2 }),
            Err(CoreError::BulkInvalidUpdate)
        );
    }

    #[test]
    fn empty_operator_document_is_rejected() {
        assert_eq!(
            validate_update(&UpdateSpec::Operators(doc! {})),
            Err(CoreError::BulkInvalidUpdate)
        );
    }

    #[test]
    fn pipeline_allows_set_unset_project_replace_stages_only() {
        let stage = |name: &str| {
            let mut d = Document::new();
            d.insert(name, doc! {});
            validate_update(&UpdateSpec::Pipeline(vec![d]))
        };
        for ok in [
            "$set",
            "$addFields",
            "$unset",
            "$project",
            "$replaceRoot",
            "$replaceWith",
        ] {
            assert_eq!(stage(ok), Ok(()), "{ok}");
        }
        for bad in ["$lookup", "$out", "$merge", "$match"] {
            assert_eq!(stage(bad), Err(CoreError::BulkInvalidUpdate), "{bad}");
        }
    }

    #[test]
    fn pipeline_stage_must_have_exactly_one_key() {
        let two = doc! { "$set": { "a": 1 }, "$unset": "b" };
        assert_eq!(
            validate_update(&UpdateSpec::Pipeline(vec![two])),
            Err(CoreError::BulkInvalidUpdate)
        );
        assert_eq!(
            validate_update(&UpdateSpec::Pipeline(vec![doc! {}])),
            Err(CoreError::BulkInvalidUpdate)
        );
    }

    #[test]
    fn empty_pipeline_is_rejected() {
        assert_eq!(
            validate_update(&UpdateSpec::Pipeline(vec![])),
            Err(CoreError::BulkInvalidUpdate)
        );
    }

    #[test]
    fn parse_update_reads_object_and_array() {
        assert_eq!(
            parse_update(r#"{ "$set": { "a": 1 } }"#),
            Ok(UpdateSpec::Operators(doc! { "$set": { "a": 1 } }))
        );
        assert_eq!(
            parse_update(r#"[ { "$set": { "a": 1 } } ]"#),
            Ok(UpdateSpec::Pipeline(vec![doc! { "$set": { "a": 1 } }]))
        );
    }

    #[test]
    fn parse_update_rejects_other_json() {
        assert!(matches!(
            parse_update("nope"),
            Err(CoreError::InvalidJson(_))
        ));
        assert!(matches!(
            parse_update("3"),
            Err(CoreError::InvalidDocument(_))
        ));
        assert!(matches!(
            parse_update("[1]"),
            Err(CoreError::InvalidDocument(_))
        ));
    }

    #[test]
    fn errors_never_echo_values() {
        let spec = parse_update(r#"{ "$set": { "secret": "hunter2" }, "plain": "hunter2" }"#)
            .expect("parses");
        let err = validate_update(&spec).expect_err("rejected");
        assert!(!err.to_string().contains("hunter2"));
        let err = parse_update(r#"[ { "$lookup": "hunter2" } ]"#)
            .and_then(|s| validate_update(&s))
            .expect_err("rejected");
        assert!(!err.to_string().contains("hunter2"));
        let err = parse_update(r#"[ "hunter2" ]"#).expect_err("rejected");
        assert!(!err.to_string().contains("hunter2"));
    }
}
