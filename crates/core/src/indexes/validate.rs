//! Checks on an index the person is about to create; the server still has the last word.

use std::collections::HashSet;

use super::spec::{IndexSpec, KeyKind};

/// Why a new index cannot be created. The UI maps each to a message.
#[derive(Clone, Debug, PartialEq)]
pub enum IndexProblem {
    NoKeys,
    EmptyField,
    DuplicateField(String),
    /// A TTL index needs exactly one key, ascending or descending.
    TtlNeedsOneOrderedKey,
    /// Below zero; 0 is fine, MongoDB expires such documents at once.
    TtlNegative,
}

/// Checks a spec before it is sent to the server.
pub fn validate_new(spec: &IndexSpec) -> std::result::Result<(), IndexProblem> {
    if spec.keys.is_empty() {
        return Err(IndexProblem::NoKeys);
    }
    let mut seen = HashSet::new();
    for key in &spec.keys {
        if key.field.trim().is_empty() {
            return Err(IndexProblem::EmptyField);
        }
        if !seen.insert(key.field.as_str()) {
            return Err(IndexProblem::DuplicateField(key.field.clone()));
        }
    }
    if let Some(seconds) = spec.options.expire_after_seconds {
        let ordered = matches!(spec.keys[0].kind, KeyKind::Asc | KeyKind::Desc);
        if spec.keys.len() != 1 || !ordered {
            return Err(IndexProblem::TtlNeedsOneOrderedKey);
        }
        if seconds < 0 {
            return Err(IndexProblem::TtlNegative);
        }
    }
    Ok(())
}
