use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use super::*;
use crate::error::{CoreError, Result};
use crate::explain::VerdictKind;
use crate::store::{DocumentStore, Explain, Namespace, Page, Query};

fn scan_plan() -> Document {
    doc! { "queryPlanner": { "namespace": "db.c", "winningPlan": { "stage": "COLLSCAN" } } }
}

fn index_plan() -> Document {
    doc! { "queryPlanner": { "namespace": "db.c", "winningPlan": {
        "stage": "FETCH",
        "inputStage": { "stage": "IXSCAN", "indexName": "a_1", "keyPattern": { "a": 1 } }
    } } }
}

#[derive(Clone)]
struct Script {
    explain: std::result::Result<Document, ()>,
    count: std::result::Result<u64, ()>,
    delay: Duration,
}

impl Script {
    fn scan(count: u64) -> Self {
        Self {
            explain: Ok(scan_plan()),
            count: Ok(count),
            delay: Duration::ZERO,
        }
    }
}

#[derive(Default)]
struct FakeStore {
    scripts: HashMap<String, Script>,
    explains: Mutex<Vec<String>>,
    counts: Mutex<Vec<String>>,
}

impl FakeStore {
    fn with(mut self, collection: &str, script: Script) -> Self {
        self.scripts.insert(collection.to_owned(), script);
        self
    }

    fn explained(&self) -> Vec<String> {
        self.explains.lock().unwrap().clone()
    }

    fn counted(&self) -> Vec<String> {
        self.counts.lock().unwrap().clone()
    }
}

#[async_trait]
impl DocumentStore for FakeStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        Err(CoreError::Unsupported("unused".into()))
    }
    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.counts.lock().unwrap().push(ns.collection.clone());
        match self.scripts.get(&ns.collection).map(|s| s.count) {
            Some(Ok(n)) => Ok(n),
            _ => Err(CoreError::Database("count failed".into())),
        }
    }
    async fn explain_plan(&self, ns: &Namespace, _q: &Query) -> Result<Explain> {
        self.explains.lock().unwrap().push(ns.collection.clone());
        let script = self
            .scripts
            .get(&ns.collection)
            .cloned()
            .ok_or_else(|| CoreError::Unsupported("unscripted".into()))?;
        tokio::time::sleep(script.delay).await;
        script
            .explain
            .map(|document| Explain {
                document,
                plan_only: true,
            })
            .map_err(|()| CoreError::Database("explain failed".into()))
    }
}

fn query() -> Query {
    Query {
        filter: doc! { "a": 1 },
        ..Query::default()
    }
}

fn target(collection: &str) -> GuardTarget {
    GuardTarget {
        ns: Namespace::new("db", collection),
        query: query(),
    }
}

const BUDGET: Duration = Duration::from_secs(10);

#[test]
fn warns_only_for_scan_and_uncovered_sort() {
    for kind in VerdictKind::ALL {
        let expected = matches!(
            kind,
            VerdictKind::CollectionScan | VerdictKind::SortNotCovered
        );
        assert_eq!(warns(kind), expected, "{kind:?}");
    }
}

#[tokio::test]
async fn a_scan_on_a_collection_of_exactly_min_docs_is_reported() {
    let store = FakeStore::default().with("c", Script::scan(1000));
    let report = check(&store, vec![target("c")], Some(1000), BUDGET).await;
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].kind, VerdictKind::CollectionScan);
    assert_eq!(report.findings[0].ns, Namespace::new("db", "c"));
    assert_eq!(report.findings[0].query, query());
    assert!(report.findings[0].suggestion.is_some());
    assert_eq!(report.unchecked, 0);
}

#[tokio::test]
async fn a_scan_on_a_collection_one_below_min_docs_is_not_reported() {
    let store = FakeStore::default().with("c", Script::scan(999));
    let report = check(&store, vec![target("c")], Some(1000), BUDGET).await;
    assert!(report.findings.is_empty());
    assert_eq!(report.unchecked, 0);
}

#[tokio::test]
async fn min_docs_none_reports_a_scan_at_any_size() {
    let store = FakeStore::default().with("c", Script::scan(3));
    let report = check(&store, vec![target("c")], None, BUDGET).await;
    assert_eq!(report.findings.len(), 1);
    assert!(store.counted().is_empty(), "no count when min_docs is None");
}

#[tokio::test]
async fn an_indexed_query_is_neither_reported_nor_counted() {
    let script = Script {
        explain: Ok(index_plan()),
        ..Script::scan(5000)
    };
    let store = FakeStore::default().with("c", script);
    let report = check(&store, vec![target("c")], Some(1), BUDGET).await;
    assert!(report.findings.is_empty());
    assert!(store.counted().is_empty());
}

#[tokio::test]
async fn a_failing_count_still_reports_the_scan() {
    let script = Script {
        count: Err(()),
        ..Script::scan(0)
    };
    let store = FakeStore::default().with("c", script);
    let report = check(&store, vec![target("c")], Some(1000), BUDGET).await;
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.unchecked, 0);
}

#[tokio::test]
async fn an_explain_error_counts_as_unchecked_and_is_not_a_finding() {
    let broken = Script {
        explain: Err(()),
        ..Script::scan(10)
    };
    let store = FakeStore::default()
        .with("bad", broken)
        .with("ok", Script::scan(10));
    let targets = vec![target("bad"), target("ok")];
    let report = check(&store, targets, None, BUDGET).await;
    assert_eq!(report.unchecked, 1);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].ns.collection, "ok");
}

#[tokio::test]
async fn targets_beyond_max_targets_count_as_unchecked() {
    let names: Vec<String> = (0..25).map(|i| format!("c{i}")).collect();
    let store = names
        .iter()
        .fold(FakeStore::default(), |s, n| s.with(n, Script::scan(10)));
    let targets = names.iter().map(|n| target(n)).collect();
    let report = check(&store, targets, None, BUDGET).await;
    assert_eq!(MAX_TARGETS, 20);
    assert_eq!(store.explained().len(), 20);
    assert_eq!(report.findings.len(), 20);
    assert_eq!(report.unchecked, 5);
}

#[tokio::test(start_paused = true)]
async fn targets_not_reached_in_the_budget_count_as_unchecked() {
    let slow = Script {
        delay: Duration::from_millis(30),
        ..Script::scan(10)
    };
    let names = ["a", "b", "c", "d"];
    let store = names
        .iter()
        .fold(FakeStore::default(), |s, n| s.with(n, slow.clone()));
    let targets = names.iter().map(|n| target(n)).collect();
    let report = check(&store, targets, None, Duration::from_millis(50)).await;
    // `a` finishes at 30 ms; `b` is cut off at 50 ms; `c` and `d` are never started.
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.unchecked, 3);
    assert_eq!(store.explained(), vec!["a", "b"]);
}

#[tokio::test]
async fn a_target_without_filter_and_sort_is_skipped_without_an_explain() {
    let store = FakeStore::default().with("c", Script::scan(10));
    let bare = GuardTarget {
        ns: Namespace::new("db", "c"),
        query: Query::default(),
    };
    let report = check(&store, vec![bare], None, BUDGET).await;
    assert!(report.findings.is_empty());
    assert_eq!(report.unchecked, 0);
    assert!(store.explained().is_empty());
}

#[tokio::test]
async fn a_sort_alone_is_explained() {
    let store = FakeStore::default().with("c", Script::scan(10));
    let sorted = GuardTarget {
        ns: Namespace::new("db", "c"),
        query: Query {
            sort: Some(doc! { "a": 1 }),
            ..Query::default()
        },
    };
    let report = check(&store, vec![sorted], None, BUDGET).await;
    assert_eq!(store.explained(), vec!["c"]);
    assert_eq!(report.findings.len(), 1);
}
