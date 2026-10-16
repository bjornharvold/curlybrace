//! Unique connection names: `base`, then `base 2`, `base 3`, ... A suffixed name never exceeds
//! `MAX_NAME_LEN` characters; the base is cut (on a char boundary) to make room, and the cut
//! head loses any trailing whitespace so the suffix never follows a space.
use crate::connection::{ConnectionConfig, MAX_NAME_LEN};
use std::collections::{HashMap, HashSet};

/// Hands out names that are free, remembering them, so many names can be allocated in one pass
/// without rescanning the connections. Names are only ever added, so for one base the smallest
/// free number never goes down: a per-base cursor skips what is known to be taken and the
/// whole pass stays linear in the number of names.
#[derive(Debug, Clone, Default)]
pub struct NameAllocator {
    taken: HashSet<String>,
    next: HashMap<String, usize>,
}

impl NameAllocator {
    pub fn new(existing: &[ConnectionConfig]) -> Self {
        Self {
            taken: existing.iter().map(|c| c.name.clone()).collect(),
            next: HashMap::new(),
        }
    }

    /// A free name for `base` (cut to `MAX_NAME_LEN`), which is marked taken from now on.
    pub fn allocate(&mut self, base: &str) -> String {
        let base: String = base.chars().take(MAX_NAME_LEN).collect();
        if self.taken.insert(base.clone()) {
            return base;
        }
        let start = self.next.get(&base).copied().unwrap_or(2);
        let (n, name) = (start..)
            .map(|n| (n, suffixed(&base, n)))
            .find(|(_, candidate)| !self.taken.contains(candidate))
            .expect("an unbounded range always yields a free name");
        self.taken.insert(name.clone());
        self.next.insert(base, n + 1);
        name
    }
}

fn suffixed(base: &str, n: usize) -> String {
    let suffix = format!(" {n}");
    let keep = MAX_NAME_LEN.saturating_sub(suffix.chars().count());
    let head: String = base.chars().take(keep).collect();
    format!("{}{suffix}", head.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(name: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, "mongodb://h")
    }

    /// The scan-everything algorithm the allocator replaces (plus the trailing-space trim).
    fn reference(base: &str, existing: &[ConnectionConfig]) -> String {
        let taken: HashSet<&str> = existing.iter().map(|c| c.name.as_str()).collect();
        let base: String = base.chars().take(MAX_NAME_LEN).collect();
        if !taken.contains(base.as_str()) {
            return base;
        }
        (2usize..)
            .map(|n| suffixed(&base, n))
            .find(|c| !taken.contains(c.as_str()))
            .unwrap()
    }

    #[test]
    fn the_allocator_gives_the_names_the_scanning_algorithm_gives() {
        let long = "L".repeat(MAX_NAME_LEN + 10);
        let spaced = format!("{} tail", "s".repeat(MAX_NAME_LEN - 3));
        let bases = [
            "a", "a", "b", "a", "a 2", "a", "b", &long, &long, &long, &spaced, &spaced, "a 3", "a",
        ];
        let existing = vec![cfg("a"), cfg("a 3"), cfg("c")];
        let mut scanned = existing.clone();
        let mut allocator = NameAllocator::new(&existing);
        for base in bases {
            let expected = reference(base, &scanned);
            assert_eq!(allocator.allocate(base), expected, "{base:?}");
            scanned.push(cfg(&expected));
        }
    }

    #[test]
    fn a_free_name_is_kept_and_taken_afterwards() {
        let mut allocator = NameAllocator::default();
        assert_eq!(allocator.allocate("x"), "x");
        assert_eq!(allocator.allocate("x"), "x 2");
        assert_eq!(allocator.allocate("x"), "x 3");
    }

    #[test]
    fn a_name_that_appears_later_is_still_skipped() {
        let mut allocator = NameAllocator::new(&[cfg("x")]);
        assert_eq!(allocator.allocate("x"), "x 2");
        assert_eq!(allocator.allocate("x 3"), "x 3");
        assert_eq!(allocator.allocate("x"), "x 4");
    }
}
