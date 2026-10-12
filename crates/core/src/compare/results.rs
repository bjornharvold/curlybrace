//! Where a compare keeps what it found: keys and fingerprints, never documents. All result sets
//! of one compare session draw on one shared [`Budget`], and give their share back when dropped.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::fingerprint::Fingerprint;
use super::key::{KeyTuple, compare_tuples};
use crate::error::{CoreError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    OnlyInSource,
    OnlyInTarget,
    Different,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffEntry {
    pub key: KeyTuple,
    pub source: Option<Fingerprint>,
    pub target: Option<Fingerprint>,
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_differences: usize,
    pub max_result_bytes: usize,
    pub batch_size: u32,
    pub max_unindexed_documents: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_differences: 1_000_000,
            max_result_bytes: 256 * 1024 * 1024,
            batch_size: 1_000,
            max_unindexed_documents: 50_000,
        }
    }
}

struct BudgetInner {
    limits: Limits,
    entries: AtomicUsize,
    bytes: AtomicUsize,
}

/// A shared allowance of result entries and bytes.
#[derive(Clone)]
pub struct Budget(Arc<BudgetInner>);

/// Fixed overhead per entry (vector slot, two optional fingerprints, key vector header).
const ENTRY_OVERHEAD: usize = 40;

/// Size assumed for a key value that cannot be serialized.
const UNSERIALIZABLE_VALUE_COST: usize = 64;

fn entry_cost(entry: &DiffEntry) -> usize {
    let values: usize = entry
        .key
        .0
        .iter()
        .map(|v| {
            let wrapped = bson::Document::from_iter([("v".to_string(), v.clone())]);
            bson::to_vec(&wrapped).map_or(UNSERIALIZABLE_VALUE_COST, |b| b.len())
        })
        .sum();
    ENTRY_OVERHEAD + values
}

impl Budget {
    pub fn new(limits: Limits) -> Self {
        Self(Arc::new(BudgetInner {
            limits,
            entries: AtomicUsize::new(0),
            bytes: AtomicUsize::new(0),
        }))
    }

    pub fn limits(&self) -> Limits {
        self.0.limits
    }

    pub fn used_entries(&self) -> usize {
        self.0.entries.load(Ordering::SeqCst)
    }

    pub fn used_bytes(&self) -> usize {
        self.0.bytes.load(Ordering::SeqCst)
    }

    fn reserve(&self, bytes: usize) -> Result<()> {
        let inner = &self.0;
        let entries = inner.entries.fetch_add(1, Ordering::SeqCst) + 1;
        let total = inner.bytes.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if entries > inner.limits.max_differences || total > inner.limits.max_result_bytes {
            self.release(1, bytes);
            return Err(CoreError::CompareCapExceeded);
        }
        Ok(())
    }

    fn release(&self, entries: usize, bytes: usize) {
        self.0.entries.fetch_sub(entries, Ordering::SeqCst);
        self.0.bytes.fetch_sub(bytes, Ordering::SeqCst);
    }
}

/// The differences of one pair.
pub struct ResultSet {
    budget: Budget,
    entries: HashMap<Category, Vec<DiffEntry>>,
    held_bytes: usize,
}

impl ResultSet {
    pub fn new(budget: Budget) -> Self {
        Self {
            budget,
            entries: HashMap::new(),
            held_bytes: 0,
        }
    }

    pub fn push(&mut self, category: Category, entry: DiffEntry) -> Result<()> {
        let cost = entry_cost(&entry);
        self.budget.reserve(cost)?;
        self.held_bytes += cost;
        self.entries.entry(category).or_default().push(entry);
        Ok(())
    }

    pub fn count(&self, category: Category) -> usize {
        self.entries.get(&category).map_or(0, Vec::len)
    }

    pub fn page(&self, category: Category, offset: usize, limit: usize) -> &[DiffEntry] {
        let all = self.entries.get(&category).map_or(&[][..], Vec::as_slice);
        let start = offset.min(all.len());
        &all[start..start.saturating_add(limit).min(all.len())]
    }

    /// Entries arrive in key order, so a lookup is a binary search.
    pub fn find(&self, category: Category, key: &KeyTuple) -> Option<&DiffEntry> {
        let all = self.entries.get(&category)?;
        all.binary_search_by(|e| compare_tuples(&e.key, key))
            .ok()
            .map(|i| &all[i])
    }
}

impl Drop for ResultSet {
    fn drop(&mut self) {
        let count: usize = self.entries.values().map(Vec::len).sum();
        self.budget.release(count, self.held_bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::Bson;

    fn entry(n: i32) -> DiffEntry {
        DiffEntry {
            key: KeyTuple(vec![Bson::Int32(n)]),
            source: None,
            target: None,
        }
    }

    fn limits(max_differences: usize) -> Limits {
        Limits {
            max_differences,
            ..Limits::default()
        }
    }

    #[test]
    fn defaults_follow_the_spec() {
        let l = Limits::default();
        assert_eq!(l.max_differences, 1_000_000);
        assert_eq!(l.max_result_bytes, 256 * 1024 * 1024);
        assert_eq!(l.batch_size, 1_000);
        assert_eq!(l.max_unindexed_documents, 50_000);
    }

    #[test]
    fn entries_are_counted_paged_and_found_per_category() {
        let mut set = ResultSet::new(Budget::new(Limits::default()));
        for n in 0..5 {
            set.push(Category::OnlyInSource, entry(n)).unwrap();
        }
        set.push(Category::Different, entry(99)).unwrap();
        assert_eq!(set.count(Category::OnlyInSource), 5);
        assert_eq!(set.count(Category::OnlyInTarget), 0);
        assert_eq!(
            set.page(Category::OnlyInSource, 1, 2),
            &[entry(1), entry(2)]
        );
        assert!(set.page(Category::OnlyInSource, 10, 2).is_empty());
        assert_eq!(
            set.find(Category::Different, &entry(99).key),
            Some(&entry(99))
        );
        assert_eq!(set.find(Category::Different, &entry(1).key), None);
    }

    #[test]
    fn the_difference_cap_is_enforced_and_names_the_typed_error() {
        let mut set = ResultSet::new(Budget::new(limits(2)));
        set.push(Category::Different, entry(1)).unwrap();
        set.push(Category::Different, entry(2)).unwrap();
        assert_eq!(
            set.push(Category::Different, entry(3)),
            Err(CoreError::CompareCapExceeded)
        );
        assert_eq!(
            set.count(Category::Different),
            2,
            "the refused entry is not stored"
        );
    }

    #[test]
    fn the_byte_cap_is_enforced() {
        let budget = Budget::new(Limits {
            max_result_bytes: 100,
            ..Limits::default()
        });
        let mut set = ResultSet::new(budget);
        let big = DiffEntry {
            key: KeyTuple(vec![Bson::String("x".repeat(500))]),
            source: None,
            target: None,
        };
        assert_eq!(
            set.push(Category::Different, big),
            Err(CoreError::CompareCapExceeded)
        );
    }

    #[test]
    fn the_budget_is_shared_between_sets_and_returned_on_drop() {
        let budget = Budget::new(limits(3));
        let mut a = ResultSet::new(budget.clone());
        let mut b = ResultSet::new(budget.clone());
        a.push(Category::Different, entry(1)).unwrap();
        a.push(Category::Different, entry(2)).unwrap();
        b.push(Category::Different, entry(3)).unwrap();
        assert_eq!(
            b.push(Category::Different, entry(4)),
            Err(CoreError::CompareCapExceeded)
        );
        assert_eq!(budget.used_entries(), 3);
        drop(a);
        assert_eq!(budget.used_entries(), 1);
        b.push(Category::Different, entry(4)).unwrap();
        drop(b);
        assert_eq!(budget.used_entries(), 0);
        assert_eq!(budget.used_bytes(), 0, "no byte leaks");
    }
}
