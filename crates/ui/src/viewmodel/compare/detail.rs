//! One differing document: field-level change lines with formatted values.
use bson::{Bson, Document};
use curlybrace_core::compare::apply::Direction;
use curlybrace_core::compare::apply_fields::{Unaddressable, can_copy};
use curlybrace_core::compare::diff::{ChangeKind, FieldChange, PathSegment};
use curlybrace_core::compare::key::KeyTuple;

/// One changed field. `kind` is an i18n key; the texts are relaxed extended JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeLine {
    pub path: String,
    pub kind: &'static str,
    pub source_text: Option<String>,
    pub target_text: Option<String>,
    /// The structured address of the field; what a field copy sends.
    pub segments: Vec<PathSegment>,
    pub change: ChangeKind,
}

impl ChangeLine {
    /// Why this field cannot be copied in `direction`, or `None` when it can. The rules are
    /// core's `can_copy`; a throwaway `FieldChange` carries the address and kind, the values
    /// play no part.
    pub fn copy_block(&self, direction: Direction) -> Option<Unaddressable> {
        can_copy(
            &FieldChange {
                path: self.path.clone(),
                segments: self.segments.clone(),
                kind: self.change,
                source: None,
                target: None,
            },
            direction,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailView {
    pub key: KeyTuple,
    /// Requested, not yet answered.
    pub loading: bool,
    pub source: Option<Document>,
    pub target: Option<Document>,
    pub changes: Vec<ChangeLine>,
}

pub fn kind_key(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => "compare.detail.added",
        ChangeKind::Removed => "compare.detail.removed",
        ChangeKind::Changed => "compare.detail.changed",
    }
}

/// One value as compact relaxed extended JSON (the same dialect as the document viewer).
pub fn value_text(value: &Bson) -> String {
    value.clone().into_relaxed_extjson().to_string()
}

/// A whole document as pretty CANONICAL extended JSON, so number types stay visible (an Int32
/// 1 and an Int64 1 are different values to the comparison and must look different).
pub fn canonical_json(doc: &Document) -> String {
    let value = Bson::Document(doc.clone()).into_canonical_extjson();
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

impl DetailView {
    pub fn loading(key: KeyTuple) -> Self {
        Self {
            key,
            loading: true,
            source: None,
            target: None,
            changes: Vec::new(),
        }
    }

    pub fn is_for(&self, key: &KeyTuple) -> bool {
        self.key == *key
    }

    pub fn loaded(
        self,
        source: Option<Document>,
        target: Option<Document>,
        changes: &[FieldChange],
    ) -> Self {
        Self {
            loading: false,
            source,
            target,
            changes: changes.iter().map(line).collect(),
            ..self
        }
    }
}

fn line(change: &FieldChange) -> ChangeLine {
    ChangeLine {
        path: change.path.clone(),
        kind: kind_key(change.kind),
        source_text: change.source.as_ref().map(value_text),
        target_text: change.target.as_ref().map(value_text),
        segments: change.segments.clone(),
        change: change.kind,
    }
}

#[cfg(test)]
#[path = "detail_tests.rs"]
mod tests;
