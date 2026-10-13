//! Pure planning of field-level writes: which paths of a document may be copied from the origin
//! to the destination, and the update that does it. No I/O. Values are data: nothing here puts a
//! value into an error.

use bson::{Bson, Document};

use crate::compare::apply::Direction;
use crate::compare::diff::{ChangeKind, FieldChange, PathSegment};
use crate::compare::normalize::apply_scope;
use crate::compare::rules::FieldScope;
use crate::error::{CoreError, Result};
use crate::store::WriteOp;

/// What copying one change does to the destination document.
#[derive(Clone, Debug, PartialEq)]
pub enum FieldWrite {
    Set(Vec<PathSegment>, Bson),
    Unset(Vec<PathSegment>),
}

/// Why a change cannot be copied field by field. `i18n_key` names the reason for the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unaddressable {
    /// The document key `_id` is immutable.
    Id,
    /// An element of an unordered array has no stable address.
    AnyElement,
    /// A field name that is empty, contains `.` or starts with `$` cannot be written by path.
    BadName,
    /// Array elements can only be replaced, never added or removed one by one.
    ArrayElement,
    /// The origin does not hold a value to copy (or there is no path at all).
    NotAChange,
}

impl Unaddressable {
    /// String literals on purpose: the i18n usage test scans production code for them.
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Id => "compare.apply.field_reason.id",
            Self::AnyElement => "compare.apply.field_reason.any_element",
            Self::BadName => "compare.apply.field_reason.bad_name",
            Self::ArrayElement => "compare.apply.field_reason.array_element",
            Self::NotAChange => "compare.apply.field_reason.not_a_change",
        }
    }
}

/// What a change does to the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Set,
    Unset,
}

/// Ruling 5: `Added` exists only in the target, `Removed` only in the source.
fn action(kind: ChangeKind, direction: Direction) -> Action {
    match (kind, direction) {
        (ChangeKind::Changed, _) => Action::Set,
        (ChangeKind::Removed, Direction::SourceToTarget)
        | (ChangeKind::Added, Direction::TargetToSource) => Action::Set,
        (ChangeKind::Added, Direction::SourceToTarget)
        | (ChangeKind::Removed, Direction::TargetToSource) => Action::Unset,
    }
}

fn is_id(segments: &[PathSegment]) -> bool {
    matches!(segments.first(), Some(PathSegment::Field(name)) if name == "_id")
}

fn bad_name(name: &str) -> bool {
    name.is_empty() || name.contains('.') || name.starts_with('$')
}

fn has_bad_name(segments: &[PathSegment]) -> bool {
    segments
        .iter()
        .any(|s| matches!(s, PathSegment::Field(name) if bad_name(name)))
}

/// Array elements can only be replaced: adding or removing one (or anything below such an
/// element) would pad with nulls or null the element instead of removing it. Only a `Changed`
/// path may contain an index, and `Added`/`Removed` must end in a field name.
fn array_refused(change: &FieldChange) -> bool {
    let has_index = change
        .segments
        .iter()
        .any(|s| matches!(s, PathSegment::Index(_)));
    match change.kind {
        ChangeKind::Changed => false,
        ChangeKind::Added | ChangeKind::Removed => {
            has_index || !matches!(change.segments.last(), Some(PathSegment::Field(_)))
        }
    }
}

/// Why a change cannot be copied in `direction`, or `None` when it can. The origin-free half of
/// [`field_write`]; the UI uses it to enable a checkbox, and `field_write` calls it so the two
/// cannot disagree. `direction` is reserved: today the rules are direction-independent.
pub fn can_copy(change: &FieldChange, _direction: Direction) -> Option<Unaddressable> {
    let segments = &change.segments;
    if segments.is_empty() {
        return Some(Unaddressable::NotAChange);
    }
    if is_id(segments) {
        return Some(Unaddressable::Id);
    }
    if segments.contains(&PathSegment::AnyElement) {
        return Some(Unaddressable::AnyElement);
    }
    if has_bad_name(segments) {
        return Some(Unaddressable::BadName);
    }
    array_refused(change).then_some(Unaddressable::ArrayElement)
}

/// The write that copies `change` from the origin to the destination. The value is read from the
/// ORIGINAL origin document, never from the normalized values the diff carries.
pub fn field_write(
    change: &FieldChange,
    direction: Direction,
    origin: &Document,
) -> std::result::Result<FieldWrite, Unaddressable> {
    if let Some(reason) = can_copy(change, direction) {
        return Err(reason);
    }
    let segments = change.segments.clone();
    match action(change.kind, direction) {
        Action::Unset => Ok(FieldWrite::Unset(segments)),
        Action::Set => value_at(origin, &segments)
            .cloned()
            .map(|value| FieldWrite::Set(segments, value))
            .ok_or(Unaddressable::NotAChange),
    }
}

/// True when two paths are equal or one is a prefix of the other (a `$set` conflict).
pub fn paths_conflict(paths: &[Vec<PathSegment>]) -> bool {
    paths.iter().enumerate().any(|(n, a)| {
        paths[n + 1..]
            .iter()
            .any(|b| a.starts_with(b) || b.starts_with(a))
    })
}

fn step<'a>(value: &'a Bson, segment: &PathSegment) -> Option<&'a Bson> {
    match (value, segment) {
        (Bson::Document(d), PathSegment::Field(name)) => d.get(name),
        (Bson::Array(a), PathSegment::Index(i)) => a.get(*i),
        _ => None,
    }
}

/// The value at `segments` in `doc`, if present. An empty path or an `AnyElement` has none.
pub fn value_at<'a>(doc: &'a Document, segments: &[PathSegment]) -> Option<&'a Bson> {
    let (first, rest) = segments.split_first()?;
    let PathSegment::Field(name) = first else {
        return None;
    };
    rest.iter()
        .try_fold(doc.get(name)?, |value, s| step(value, s))
}

fn invalid() -> CoreError {
    CoreError::InvalidDocument(String::new())
}

/// The one reported change at exactly `path`; zero or several is a refusal.
fn unique_change<'a>(changes: &'a [FieldChange], path: &[PathSegment]) -> Result<&'a FieldChange> {
    let mut hits = changes.iter().filter(|c| c.segments == path);
    match (hits.next(), hits.next()) {
        (Some(change), None) => Ok(change),
        _ => Err(invalid()),
    }
}

/// Exact BSON equality of two optional values, presence included (`NaN` equals `NaN`).
fn same_value(a: Option<&Bson>, b: Option<&Bson>) -> bool {
    let bytes = |v: &Bson| bson::to_vec(&Document::from_iter([("v".to_string(), v.clone())])).ok();
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => bytes(x).is_some_and(|bx| Some(bx) == bytes(y)),
        _ => false,
    }
}

/// Every array above an `Index` of `path` has the same length in `doc` and in `scoped`. The scope
/// filter keeps order, so equal lengths mean no element was dropped and indexes line up.
fn arrays_aligned(path: &[PathSegment], doc: &Document, scoped: &Document) -> bool {
    (1..path.len()).all(|n| {
        if !matches!(path[n], PathSegment::Index(_)) {
            return true;
        }
        match (value_at(doc, &path[..n]), value_at(scoped, &path[..n])) {
            (Some(Bson::Array(a)), Some(Bson::Array(b))) => a.len() == b.len(),
            (None, None) => true,
            _ => false,
        }
    })
}

/// `path` addresses the same thing in `doc` and in its field-scoped projection: the arrays above
/// it line up and the value is identical, so writing or removing it cannot touch a field outside
/// the compared scope, and the address in the normalized diff points at the same value in the
/// original document.
fn scope_keeps(path: &[PathSegment], doc: &Document, scoped: &Document) -> bool {
    arrays_aligned(path, doc, scoped) && same_value(value_at(doc, path), value_at(scoped, path))
}

/// Builds the update for the requested `paths` from the fresh diff of the two documents. Every
/// path must be exactly one reported, copyable change whose value is untouched by the field
/// `scope` in both documents; otherwise nothing is planned. Callers must pass the full ORIGINAL
/// documents and the same `rules.field_scope` and `rules.ignore_array_order` used for the diff.
pub fn plan_fields(
    paths: &[Vec<PathSegment>],
    changes: &[FieldChange],
    direction: Direction,
    origin: &Document,
    dest: &Document,
    scope: &FieldScope,
) -> Result<WriteOp> {
    if paths.is_empty() || paths_conflict(paths) {
        return Err(invalid());
    }
    let dest_id = dest.get("_id").ok_or_else(invalid)?;
    let (scoped_origin, scoped_dest) = (apply_scope(origin, scope), apply_scope(dest, scope));
    let (mut set, mut unset) = (Vec::new(), Vec::new());
    for path in paths {
        if !scope_keeps(path, origin, &scoped_origin) || !scope_keeps(path, dest, &scoped_dest) {
            return Err(invalid());
        }
        let change = unique_change(changes, path)?;
        match field_write(change, direction, origin).map_err(|_| invalid())? {
            FieldWrite::Set(segments, value) => set.push((segments, value)),
            FieldWrite::Unset(segments) => unset.push(segments),
        }
    }
    Ok(WriteOp::UpdateFields {
        id: dest_id.clone(),
        set,
        unset,
    })
}
#[cfg(test)]
#[path = "apply_fields_tests.rs"]
mod tests;
