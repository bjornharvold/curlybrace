//! Which rows of a results tab are ticked, and which action a row offers. Pure.
//!
//! Selection is per (pair, category) and lives here rather than in `ResultsView`, because the
//! latter is replaced whenever the person pages.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use curlybrace_core::compare::apply::{ApplyKind, Direction};
use curlybrace_core::compare::key::{KeyTuple, compare_tuples};
use curlybrace_core::compare::protocol::PairId;
use curlybrace_core::compare::results::Category;

/// A key tuple ordered by core's `compare_tuples`, so it can live in a `BTreeSet`.
#[derive(Clone, Debug)]
pub struct OrdKey(pub KeyTuple);

impl PartialEq for OrdKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for OrdKey {}

impl PartialOrd for OrdKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrdKey {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_tuples(&self.0, &other.0)
    }
}

/// An orderable copy of core's `Category`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum CatKey {
    OnlyInSource,
    OnlyInTarget,
    Different,
}

impl From<Category> for CatKey {
    fn from(c: Category) -> Self {
        match c {
            Category::OnlyInSource => Self::OnlyInSource,
            Category::OnlyInTarget => Self::OnlyInTarget,
            Category::Different => Self::Different,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CategorySelection {
    pub keys: BTreeSet<OrdKey>,
    /// Every row of the tab, including pages that are not loaded.
    pub all: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    by: BTreeMap<(PairId, CatKey), CategorySelection>,
}

impl Selection {
    fn entry(&self, pair: PairId, cat: Category) -> CategorySelection {
        self.by
            .get(&(pair, cat.into()))
            .cloned()
            .unwrap_or_default()
    }

    fn with_entry(&self, pair: PairId, cat: Category, entry: CategorySelection) -> Self {
        let mut by = self.by.clone();
        if entry.all || !entry.keys.is_empty() {
            by.insert((pair, cat.into()), entry);
        } else {
            by.remove(&(pair, cat.into()));
        }
        Self { by }
    }

    /// Ticks or unticks one row. Unticking under `all` turns `all` off and keeps the keys that
    /// were ticked explicitly (the loaded page's other rows).
    pub fn toggled(&self, pair: PairId, cat: Category, key: &KeyTuple) -> Self {
        let mut entry = self.entry(pair, cat);
        let ord = OrdKey(key.clone());
        if entry.all {
            entry.all = false;
            entry.keys.remove(&ord);
        } else if !entry.keys.remove(&ord) {
            entry.keys.insert(ord);
        }
        self.with_entry(pair, cat, entry)
    }

    /// Ticks every key of the loaded page.
    pub fn page_selected(&self, pair: PairId, cat: Category, keys: &[KeyTuple]) -> Self {
        let mut entry = self.entry(pair, cat);
        entry.keys.extend(keys.iter().cloned().map(OrdKey));
        self.with_entry(pair, cat, entry)
    }

    /// Every row of the tab, loaded or not.
    pub fn all_selected(&self, pair: PairId, cat: Category) -> Self {
        let entry = CategorySelection {
            keys: BTreeSet::new(),
            all: true,
        };
        self.with_entry(pair, cat, entry)
    }

    pub fn cleared(&self, pair: PairId, cat: Category) -> Self {
        self.with_entry(pair, cat, CategorySelection::default())
    }

    pub fn without_pair(&self, pair: PairId) -> Self {
        Self {
            by: self
                .by
                .iter()
                .filter(|((p, _), _)| *p != pair)
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
        }
    }

    pub fn is_selected(&self, pair: PairId, cat: Category, key: &KeyTuple) -> bool {
        self.by
            .get(&(pair, cat.into()))
            .is_some_and(|e| e.all || e.keys.contains(&OrdKey(key.clone())))
    }

    /// How many rows are ticked; `all` counts the whole category without enumerating it.
    pub fn count(&self, pair: PairId, cat: Category, total_in_category: usize) -> usize {
        match self.by.get(&(pair, cat.into())) {
            Some(e) if e.all => total_in_category,
            Some(e) => e.keys.len(),
            None => 0,
        }
    }

    pub fn is_empty_for(&self, pair: PairId, cat: Category) -> bool {
        self.by
            .get(&(pair, cat.into()))
            .is_none_or(|e| !e.all && e.keys.is_empty())
    }
}

/// A write the person can queue for a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    CopyToTarget,
    CopyToSource,
    DeleteFromSource,
    DeleteFromTarget,
}

impl RowAction {
    /// The kind for a row that exists on one side only: a copy inserts, a delete deletes.
    pub fn kind(self) -> ApplyKind {
        match self {
            Self::CopyToTarget | Self::CopyToSource => ApplyKind::Insert,
            Self::DeleteFromSource | Self::DeleteFromTarget => ApplyKind::Delete,
        }
    }

    /// The kind for a row of `cat`: a copy over a different row replaces the destination.
    pub fn kind_for(self, cat: Category) -> ApplyKind {
        match (self.kind(), cat) {
            (ApplyKind::Insert, Category::Different) => ApplyKind::Replace,
            (kind, _) => kind,
        }
    }

    /// Origin to destination: deleting from the source reads the target as the origin.
    pub fn direction(self) -> Direction {
        match self {
            Self::CopyToTarget | Self::DeleteFromTarget => Direction::SourceToTarget,
            Self::CopyToSource | Self::DeleteFromSource => Direction::TargetToSource,
        }
    }

    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::CopyToTarget => "compare.apply.action.copy_to_target",
            Self::CopyToSource => "compare.apply.action.copy_to_source",
            Self::DeleteFromSource => "compare.apply.action.delete_from_source",
            Self::DeleteFromTarget => "compare.apply.action.delete_from_target",
        }
    }

    /// The actions a row of `cat` offers.
    pub fn offered_for(cat: Category) -> &'static [RowAction] {
        match cat {
            Category::OnlyInSource => &[Self::CopyToTarget, Self::DeleteFromSource],
            Category::OnlyInTarget => &[Self::CopyToSource, Self::DeleteFromTarget],
            Category::Different => &[Self::CopyToTarget, Self::CopyToSource],
        }
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
