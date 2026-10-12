//! The sorted merge-join: two key-ordered streams are walked together, so memory use is bounded
//! by the batch size plus the differences found, never by the collection size.

use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use bson::Document;

use super::fingerprint::fingerprint_of;
use super::key::{KeySpec, compare_tuples};
use super::results::{Budget, Category, DiffEntry, ResultSet};
use super::rules::CompareRules;
use super::scan::KeyScanner;
use crate::error::Result;
use crate::store::{DocumentStore, Namespace};

pub struct PairInput {
    pub source: Arc<dyn DocumentStore>,
    pub source_ns: Namespace,
    pub source_filter: Document,
    pub target: Arc<dyn DocumentStore>,
    pub target_ns: Namespace,
    pub target_filter: Document,
    pub key: KeySpec,
    pub rules: CompareRules,
    pub budget: Budget,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub equal: u64,
    pub only_in_source: u64,
    pub only_in_target: u64,
    pub different: u64,
    pub scanned_source: u64,
    pub scanned_target: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub scanned_source: u64,
    pub scanned_target: u64,
    pub differences: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineOutcome {
    Completed,
    Cancelled,
}

/// A cheap, cloneable cancellation signal.
#[derive(Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, AtomicOrdering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(AtomicOrdering::SeqCst)
    }
}

/// The two scanners, the rules and what has been found so far.
struct Walk {
    source: KeyScanner,
    target: KeyScanner,
    rules: CompareRules,
    set: ResultSet,
    summary: Summary,
}

impl Walk {
    fn new(input: PairInput) -> Self {
        let PairInput {
            source,
            source_ns,
            source_filter,
            target,
            target_ns,
            target_filter,
            key,
            rules,
            budget,
        } = input;
        let batch = budget.limits().batch_size;
        Self {
            source: KeyScanner::new(source, source_ns, source_filter, key.clone(), batch),
            target: KeyScanner::new(target, target_ns, target_filter, key, batch),
            rules,
            set: ResultSet::new(budget),
            summary: Summary::default(),
        }
    }

    /// Which side holds the smaller next key; `None` when both streams are exhausted.
    async fn next_order(&mut self) -> Result<Option<Ordering>> {
        Ok(
            match (self.source.peek().await?, self.target.peek().await?) {
                (None, None) => None,
                (Some(_), None) => Some(Ordering::Less),
                (None, Some(_)) => Some(Ordering::Greater),
                (Some(s), Some(t)) => Some(compare_tuples(&s.0, &t.0)),
            },
        )
    }

    async fn step(&mut self, order: Ordering) -> Result<()> {
        match order {
            Ordering::Less => {
                let (key, doc) = self.source.next().await?.expect("peeked");
                let source = Some(fingerprint_of(&doc, &self.rules)?);
                let entry = DiffEntry {
                    key,
                    source,
                    target: None,
                };
                self.set.push(Category::OnlyInSource, entry)?;
                self.summary.only_in_source += 1;
            }
            Ordering::Greater => {
                let (key, doc) = self.target.next().await?.expect("peeked");
                let target = Some(fingerprint_of(&doc, &self.rules)?);
                let entry = DiffEntry {
                    key,
                    source: None,
                    target,
                };
                self.set.push(Category::OnlyInTarget, entry)?;
                self.summary.only_in_target += 1;
            }
            Ordering::Equal => self.step_match().await?,
        }
        Ok(())
    }

    async fn step_match(&mut self) -> Result<()> {
        let (key, s_doc) = self.source.next().await?.expect("peeked");
        let (_, t_doc) = self.target.next().await?.expect("peeked");
        let source = fingerprint_of(&s_doc, &self.rules)?;
        let target = fingerprint_of(&t_doc, &self.rules)?;
        if source == target {
            self.summary.equal += 1;
        } else {
            let entry = DiffEntry {
                key,
                source: Some(source),
                target: Some(target),
            };
            self.set.push(Category::Different, entry)?;
            self.summary.different += 1;
        }
        Ok(())
    }

    fn refresh_scanned(&mut self) {
        self.summary.scanned_source = self.source.scanned();
        self.summary.scanned_target = self.target.scanned();
    }

    fn progress(&self) -> Progress {
        Progress {
            scanned_source: self.summary.scanned_source,
            scanned_target: self.summary.scanned_target,
            differences: self.summary.only_in_source
                + self.summary.only_in_target
                + self.summary.different,
        }
    }

    fn finish(mut self, outcome: EngineOutcome) -> (EngineOutcome, Summary, ResultSet) {
        self.refresh_scanned();
        (outcome, self.summary, self.set)
    }
}

/// Walks both streams to the end (or until cancelled) and returns the outcome, the counts and
/// the differences. Errors (duplicate key, order, cap, store failures) abort the pair.
pub async fn run_pair(
    input: PairInput,
    cancel: &CancelFlag,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(EngineOutcome, Summary, ResultSet)> {
    let batch = u64::from(input.budget.limits().batch_size.max(1));
    let mut walk = Walk::new(input);
    let mut steps = 0u64;
    loop {
        if cancel.is_cancelled() {
            return Ok(walk.finish(EngineOutcome::Cancelled));
        }
        let Some(order) = walk.next_order().await? else {
            return Ok(walk.finish(EngineOutcome::Completed));
        };
        walk.step(order).await?;
        steps += 1;
        if steps % batch == 0 {
            walk.refresh_scanned();
            on_progress(walk.progress());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::fingerprint::fingerprint_of;
    use crate::compare::results::{Category, Limits};
    use crate::error::CoreError;
    use crate::test_support::InMemoryStore;
    use bson::{Bson, doc};
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn ns(c: &str) -> Namespace {
        Namespace::new("db", c)
    }

    fn input(source: Vec<Document>, target: Vec<Document>, limits: Limits) -> PairInput {
        let s = InMemoryStore::new().with_docs(&ns("a"), source);
        let t = InMemoryStore::new().with_docs(&ns("b"), target);
        PairInput {
            source: Arc::new(s),
            source_ns: ns("a"),
            source_filter: doc! {},
            target: Arc::new(t),
            target_ns: ns("b"),
            target_filter: doc! {},
            key: KeySpec::id(),
            rules: CompareRules::for_key(&KeySpec::id()),
            budget: Budget::new(limits),
        }
    }

    async fn run(i: PairInput) -> Result<(EngineOutcome, Summary, ResultSet)> {
        run_pair(i, &CancelFlag::default(), &mut |_| {}).await
    }

    fn ids(set: &ResultSet, c: Category) -> Vec<i32> {
        set.page(c, 0, usize::MAX)
            .iter()
            .map(|e| match &e.key.0[0] {
                Bson::Int32(n) => *n,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    #[tokio::test]
    async fn identical_collections_are_all_equal_and_keep_nothing() {
        let docs: Vec<Document> = (0..250).map(|n| doc! { "_id": n, "v": n * 2 }).collect();
        let limits = Limits {
            batch_size: 40,
            ..Limits::default()
        };
        let i = input(docs.clone(), docs, limits);
        let budget = i.budget.clone();
        let (outcome, summary, set) = run(i).await.unwrap();
        assert_eq!(outcome, EngineOutcome::Completed);
        assert_eq!(
            summary,
            Summary {
                equal: 250,
                scanned_source: 250,
                scanned_target: 250,
                ..Summary::default()
            }
        );
        assert_eq!(budget.used_entries(), 0, "equal documents cost no memory");
        assert_eq!(set.count(Category::Different), 0);
    }

    #[tokio::test]
    async fn classifies_missing_extra_and_different_documents() {
        let source = vec![
            doc! { "_id": 1, "v": 1 },
            doc! { "_id": 2, "v": 2 },
            doc! { "_id": 3, "v": 3 },
            doc! { "_id": 5, "v": 5 },
        ];
        let target = vec![
            doc! { "_id": 2, "v": 2 },
            doc! { "_id": 3, "v": 99 },
            doc! { "_id": 4, "v": 4 },
            doc! { "_id": 5, "v": 5 },
        ];
        let (_, summary, set) = run(input(
            source,
            target,
            Limits {
                batch_size: 2,
                ..Limits::default()
            },
        ))
        .await
        .unwrap();
        assert_eq!(ids(&set, Category::OnlyInSource), [1]);
        assert_eq!(ids(&set, Category::OnlyInTarget), [4]);
        assert_eq!(ids(&set, Category::Different), [3]);
        assert_eq!(
            (
                summary.equal,
                summary.only_in_source,
                summary.only_in_target,
                summary.different
            ),
            (2, 1, 1, 1)
        );
        let diff = set.page(Category::Different, 0, 1)[0].clone();
        assert!(diff.source.is_some() && diff.target.is_some() && diff.source != diff.target);
    }

    #[tokio::test]
    async fn one_empty_side_and_two_empty_sides() {
        let docs: Vec<Document> = (0..5).map(|n| doc! { "_id": n }).collect();
        let (_, s, set) = run(input(docs.clone(), vec![], Limits::default()))
            .await
            .unwrap();
        assert_eq!(
            (s.only_in_source, ids(&set, Category::OnlyInSource).len()),
            (5, 5)
        );
        let (_, s, _) = run(input(vec![], docs, Limits::default())).await.unwrap();
        assert_eq!(s.only_in_target, 5);
        let (outcome, s, _) = run(input(vec![], vec![], Limits::default())).await.unwrap();
        assert_eq!((outcome, s), (EngineOutcome::Completed, Summary::default()));
    }

    #[tokio::test]
    async fn rules_decide_what_counts_as_different() {
        let source = vec![doc! { "_id": 1, "n": 1_i32, "ts": 1 }];
        let target = vec![doc! { "_id": 1, "n": 1_i64, "ts": 2 }];
        let strict = run(input(source.clone(), target.clone(), Limits::default()))
            .await
            .unwrap();
        assert_eq!(strict.1.different, 1);
        let mut i = input(source, target, Limits::default());
        i.rules.numbers_equal_across_types = true;
        i.rules.field_scope = crate::compare::rules::FieldScope::AllExcept(vec!["ts".into()]);
        assert_eq!(run(i).await.unwrap().1.equal, 1);
    }

    #[tokio::test]
    async fn a_business_key_matches_documents_with_different_ids() {
        let key = KeySpec::new(vec!["email".into()]).unwrap();
        let mut i = input(
            vec![doc! { "_id": 1, "email": "a", "v": 1 }],
            vec![doc! { "_id": 77, "email": "a", "v": 1 }],
            Limits::default(),
        );
        i.rules = CompareRules::for_key(&key);
        i.key = key;
        assert_eq!(
            run(i).await.unwrap().1.equal,
            1,
            "_id is not compared when the key is a business key"
        );
    }

    #[tokio::test]
    async fn the_difference_cap_aborts_with_the_typed_error() {
        let source: Vec<Document> = (0..10).map(|n| doc! { "_id": n }).collect();
        let i = input(
            source,
            vec![],
            Limits {
                max_differences: 3,
                ..Limits::default()
            },
        );
        assert!(matches!(run(i).await, Err(CoreError::CompareCapExceeded)));
    }

    #[tokio::test]
    async fn a_duplicate_on_either_side_aborts() {
        let key = KeySpec::new(vec!["k".into()]).unwrap();
        let dup = vec![doc! { "_id": 1, "k": 1 }, doc! { "_id": 2, "k": 1 }];
        let mut i = input(dup.clone(), vec![], Limits::default());
        i.key = key.clone();
        i.rules = CompareRules::for_key(&key);
        assert!(matches!(run(i).await, Err(CoreError::CompareDuplicateKey)));
        let mut i = input(vec![], dup, Limits::default());
        i.key = key.clone();
        i.rules = CompareRules::for_key(&key);
        assert!(matches!(run(i).await, Err(CoreError::CompareDuplicateKey)));
    }

    #[tokio::test]
    async fn cancel_stops_the_run_and_reports_the_partial_summary() {
        let docs: Vec<Document> = (0..500).map(|n| doc! { "_id": n }).collect();
        let i = input(
            docs,
            vec![],
            Limits {
                batch_size: 10,
                ..Limits::default()
            },
        );
        let cancel = CancelFlag::default();
        let mut seen = 0;
        let c2 = cancel.clone();
        let (outcome, summary, _) = run_pair(i, &cancel, &mut |_| {
            seen += 1;
            if seen == 2 {
                c2.cancel();
            }
        })
        .await
        .unwrap();
        assert_eq!(outcome, EngineOutcome::Cancelled);
        assert!(summary.only_in_source < 500);
    }

    #[tokio::test]
    async fn progress_is_reported_and_monotonic() {
        let docs: Vec<Document> = (0..120).map(|n| doc! { "_id": n }).collect();
        let i = input(
            docs.clone(),
            docs,
            Limits {
                batch_size: 25,
                ..Limits::default()
            },
        );
        let mut reports: Vec<Progress> = Vec::new();
        run_pair(i, &CancelFlag::default(), &mut |p| reports.push(p))
            .await
            .unwrap();
        assert!(reports.len() >= 3);
        for pair in reports.windows(2) {
            assert!(pair[1].scanned_source >= pair[0].scanned_source);
        }
    }

    fn brute_force(
        source: &BTreeMap<i32, i32>,
        target: &BTreeMap<i32, i32>,
    ) -> (Vec<i32>, Vec<i32>, Vec<i32>, u64) {
        let (mut only_s, mut only_t, mut diff, mut equal) = (vec![], vec![], vec![], 0);
        for (k, v) in source {
            match target.get(k) {
                None => only_s.push(*k),
                Some(t) if t != v => diff.push(*k),
                Some(_) => equal += 1,
            }
        }
        only_t.extend(target.keys().filter(|k| !source.contains_key(k)));
        (only_s, only_t, diff, equal)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn the_merge_join_agrees_with_a_brute_force_comparison(
            source in proptest::collection::btree_map(0..40i32, 0..3i32, 0..30),
            target in proptest::collection::btree_map(0..40i32, 0..3i32, 0..30),
            batch in 1u32..8,
        ) {
            let to_docs = |m: &BTreeMap<i32, i32>| m.iter().map(|(k, v)| doc! { "_id": *k, "v": *v }).collect::<Vec<_>>();
            let i = input(to_docs(&source), to_docs(&target), Limits { batch_size: batch, ..Limits::default() });
            let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
            let (_, summary, set) = rt.block_on(run(i)).unwrap();
            let (os, ot, d, eq) = brute_force(&source, &target);
            prop_assert_eq!(ids(&set, Category::OnlyInSource), os);
            prop_assert_eq!(ids(&set, Category::OnlyInTarget), ot);
            prop_assert_eq!(ids(&set, Category::Different), d);
            prop_assert_eq!(summary.equal, eq);
        }
    }

    #[test]
    fn fingerprints_in_results_match_the_documents() {
        let rules = CompareRules::for_key(&KeySpec::id());
        let d = doc! { "_id": 3, "v": 3 };
        assert!(fingerprint_of(&d, &rules).is_ok());
    }
}
