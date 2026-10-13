//! Which fields of one differing document are ticked in the detail pane. Pure.
//!
//! The selection is per (pair, key) and holds field paths as segment lists. It is only meaningful
//! while that document's detail is loaded; the reducer clears it when the detail goes away.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use curlybrace_core::compare::diff::PathSegment;
use curlybrace_core::compare::key::KeyTuple;
use curlybrace_core::compare::protocol::PairId;

use super::selection::OrdKey;

/// A field path ordered so it can live in a `BTreeSet`: `Field(name)` sorts before `Index(i)`
/// before `AnyElement`, names and indexes by value, and paths lexicographically (a prefix first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathKey(pub Vec<PathSegment>);

fn rank(segment: &PathSegment) -> u8 {
    match segment {
        PathSegment::Field(_) => 0,
        PathSegment::Index(_) => 1,
        PathSegment::AnyElement => 2,
    }
}

fn compare_segments(a: &PathSegment, b: &PathSegment) -> Ordering {
    match (a, b) {
        (PathSegment::Field(x), PathSegment::Field(y)) => x.cmp(y),
        (PathSegment::Index(x), PathSegment::Index(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

impl Ord for PathKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .iter()
            .zip(&other.0)
            .map(|(a, b)| compare_segments(a, b))
            .find(|o| o.is_ne())
            .unwrap_or_else(|| self.0.len().cmp(&other.0.len()))
    }
}

impl PartialOrd for PathKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The ticked field paths of every document of every pair.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FieldSelection {
    by: BTreeMap<(PairId, OrdKey), BTreeSet<PathKey>>,
}

impl FieldSelection {
    fn slot(pair: PairId, key: &KeyTuple) -> (PairId, OrdKey) {
        (pair, OrdKey(key.clone()))
    }

    /// Ticks the path, or unticks it when it is ticked. An emptied document is dropped.
    pub fn toggled(&self, pair: PairId, key: &KeyTuple, segments: &[PathSegment]) -> Self {
        let slot = Self::slot(pair, key);
        let path = PathKey(segments.to_vec());
        let mut by = self.by.clone();
        let mut set = by.remove(&slot).unwrap_or_default();
        if !set.remove(&path) {
            set.insert(path);
        }
        if !set.is_empty() {
            by.insert(slot, set);
        }
        Self { by }
    }

    pub fn cleared(&self, pair: PairId, key: &KeyTuple) -> Self {
        let mut by = self.by.clone();
        by.remove(&Self::slot(pair, key));
        Self { by }
    }

    pub fn without_pair(&self, pair: PairId) -> Self {
        Self {
            by: self
                .by
                .iter()
                .filter(|((p, _), _)| *p != pair)
                .map(|(slot, set)| (slot.clone(), set.clone()))
                .collect(),
        }
    }

    pub fn is_selected(&self, pair: PairId, key: &KeyTuple, segments: &[PathSegment]) -> bool {
        self.by
            .get(&Self::slot(pair, key))
            .is_some_and(|set| set.contains(&PathKey(segments.to_vec())))
    }

    /// The ticked paths of one document in `PathKey` order.
    pub fn paths(&self, pair: PairId, key: &KeyTuple) -> Vec<Vec<PathSegment>> {
        self.by
            .get(&Self::slot(pair, key))
            .map(|set| set.iter().map(|p| p.0.clone()).collect())
            .unwrap_or_default()
    }

    pub fn count(&self, pair: PairId, key: &KeyTuple) -> usize {
        self.by.get(&Self::slot(pair, key)).map_or(0, BTreeSet::len)
    }
}

#[cfg(test)]
#[path = "field_select_tests.rs"]
mod tests;
