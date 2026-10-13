//! The `DuplicateKeys` command against in-memory stores: no server, no network.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bson::{Bson, Document, doc};
use tokio::sync::{Notify, mpsc::UnboundedSender};

use super::tests::{MapConnector, cfg, next_compare, ns, pair, start_with};
use crate::compare::key::{KeySpec, KeyTuple};
use crate::compare::protocol::{
    CompareCommand, CompareEvent, DUPLICATE_KEYS_LIMIT, DuplicateReport, PairSpec,
};
use crate::compare::results::Category;
use crate::compare::rules::{CompareRules, FieldScope};
use crate::error::{CoreError, Result};
use crate::runtime::{Command, RuntimeConfig, RuntimeHandle};
use crate::store::{
    CollectionKind, DocumentStore, DuplicateGroup, IndexStatus, Namespace, Page, Query,
};
use crate::test_support::InMemoryStore;

type Tx = UnboundedSender<Command>;

/// What `duplicate_keys` does on a [`ProbeStore`].
#[derive(Clone)]
enum Mode {
    Plain,
    /// Announces itself, then waits for the test to release it.
    Gated(Arc<Gate>),
    Panics,
    Never,
}

#[derive(Default)]
struct Gate {
    entered: Notify,
    release: Notify,
}

struct ProbeStore {
    inner: InMemoryStore,
    mode: Mode,
}

#[async_trait]
impl DocumentStore for ProbeStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }
    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: Document) -> Result<Bson> {
        self.inner.insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }
    async fn estimated_count(&self, ns: &Namespace) -> Result<u64> {
        self.inner.estimated_count(ns).await
    }
    async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus> {
        self.inner.key_index_status(ns, key).await
    }
    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        self.inner.list_collection_kinds(db).await
    }
    async fn duplicate_keys(
        &self,
        ns: &Namespace,
        filter: &Document,
        key: &KeySpec,
        limit: u32,
    ) -> Result<Vec<DuplicateGroup>> {
        match &self.mode {
            Mode::Plain => {}
            Mode::Gated(gate) => {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            Mode::Panics => panic!("store blew up"),
            Mode::Never => std::future::pending().await,
        }
        self.inner.duplicate_keys(ns, filter, key, limit).await
    }
}

fn store(docs: Vec<Document>, mode: Mode) -> Arc<dyn DocumentStore> {
    let inner = InMemoryStore::new().with_docs(&ns("d", "c"), docs);
    Arc::new(ProbeStore { inner, mode })
}

fn key_k() -> KeySpec {
    KeySpec::new(vec!["k".to_string()]).unwrap()
}

fn dup_pair() -> PairSpec {
    let mut spec = pair(7, ns("d", "c"), ns("d", "c"));
    spec.rules = CompareRules::for_key(&key_k());
    spec.key = key_k();
    spec
}

/// `groups` keys, each present twice, as `{ "k": n }` documents.
fn twice_each(groups: i32) -> Vec<Document> {
    (0..groups)
        .flat_map(|n| [doc! { "k": n }, doc! { "k": n }])
        .collect()
}

fn send(tx: &Tx, command: CompareCommand) {
    tx.send(Command::Compare(command)).unwrap();
}

async fn rig(
    source: Arc<dyn DocumentStore>,
    target: Arc<dyn DocumentStore>,
    config: RuntimeConfig,
) -> (Tx, RuntimeHandle) {
    let stores = HashMap::from([
        ("mongodb://s".to_string(), source),
        ("mongodb://t".to_string(), target),
    ]);
    let connector = MapConnector {
        stores,
        ..MapConnector::default()
    };
    let (tx, mut h) = start_with(connector, config);
    send(
        &tx,
        CompareCommand::Open {
            session: 1,
            source: cfg("mongodb://s"),
            target: cfg("mongodb://t"),
        },
    );
    assert_eq!(
        next_compare(&mut h).await,
        CompareEvent::Opened { session: 1 }
    );
    (tx, h)
}

async fn rig_default(source: Vec<Document>, target: Vec<Document>) -> (Tx, RuntimeHandle) {
    let (s, t) = (store(source, Mode::Plain), store(target, Mode::Plain));
    rig(s, t, RuntimeConfig::default()).await
}

fn ask(tx: &Tx, pair: PairSpec) {
    send(tx, CompareCommand::DuplicateKeys { session: 1, pair });
}

async fn reports(h: &mut RuntimeHandle) -> (DuplicateReport, DuplicateReport) {
    match next_compare(h).await {
        CompareEvent::DuplicateKeys {
            session: 1,
            pair: 7,
            source,
            target,
        } => (source, target),
        other => panic!("{other:?}"),
    }
}

async fn failure(h: &mut RuntimeHandle) -> (&'static str, CoreError) {
    match next_compare(h).await {
        CompareEvent::Failed { context, error, .. } => (context, error),
        other => panic!("{other:?}"),
    }
}

async fn assert_silent(h: &mut RuntimeHandle) {
    let late = tokio::time::timeout(Duration::from_millis(200), h.rx.recv()).await;
    assert!(late.is_err(), "a closed session stays silent: {late:?}");
}

fn counts(report: &DuplicateReport) -> Vec<u64> {
    report.groups.iter().map(|g| g.count).collect()
}

#[tokio::test]
async fn both_sides_are_asked_and_each_reports_its_own_duplicates() {
    let source = vec![
        doc! { "k": 1_i32 },
        doc! { "k": 1_i64 },
        doc! { "k": 2 },
        doc! { "k": 3 },
    ];
    let target = vec![
        doc! { "k": "a" },
        doc! { "k": "a" },
        doc! { "k": "a" },
        doc! { "k": "b" },
    ];
    let (tx, mut h) = rig_default(source, target).await;
    ask(&tx, dup_pair());
    let (source, target) = reports(&mut h).await;
    assert_eq!(counts(&source), vec![2]);
    assert_eq!(source.groups[0].key.0.len(), 1);
    assert_eq!(
        target.groups,
        vec![DuplicateGroup {
            key: KeyTuple(vec![Bson::String("a".into())]),
            count: 3
        }]
    );
    assert!(!source.more && !target.more);
}

async fn source_report_for(groups: i32) -> DuplicateReport {
    let (tx, mut h) = rig_default(twice_each(groups), vec![]).await;
    ask(&tx, dup_pair());
    reports(&mut h).await.0
}

#[tokio::test]
async fn more_is_set_only_when_there_are_more_groups_than_the_limit() {
    let limit = DUPLICATE_KEYS_LIMIT as usize;
    let over = source_report_for(DUPLICATE_KEYS_LIMIT as i32 + 1).await;
    assert_eq!((over.groups.len(), over.more), (limit, true));
    let exact = source_report_for(DUPLICATE_KEYS_LIMIT as i32).await;
    assert_eq!((exact.groups.len(), exact.more), (limit, false));
}

#[tokio::test]
async fn a_side_without_duplicates_reports_an_empty_report() {
    let source = vec![doc! { "k": 1 }, doc! { "k": 2 }];
    let target = vec![doc! { "k": 5 }, doc! { "k": 5 }];
    let (tx, mut h) = rig_default(source, target).await;
    ask(&tx, dup_pair());
    let (source, target) = reports(&mut h).await;
    let empty = DuplicateReport {
        groups: vec![],
        more: false,
    };
    assert_eq!(source, empty);
    assert_eq!(counts(&target), vec![2]);
}

#[tokio::test]
async fn an_invalid_key_or_rules_is_refused_before_any_store_call() {
    // A gated store announces itself when asked; it must never be.
    let gate = Arc::new(Gate::default());
    let source = store(twice_each(2), Mode::Gated(gate.clone()));
    let target = store(vec![], Mode::Plain);
    let (tx, mut h) = rig(source, target, RuntimeConfig::default()).await;
    let mut bad_key = dup_pair();
    bad_key.key = KeySpec {
        fields: vec![String::new()],
    };
    ask(&tx, bad_key);
    let expected = ("compare_duplicates", CoreError::CompareSetupInvalid);
    assert_eq!(failure(&mut h).await, expected);
    let mut bad_rules = dup_pair();
    bad_rules.rules.field_scope = FieldScope::Only(vec![]);
    ask(&tx, bad_rules);
    assert_eq!(failure(&mut h).await, expected);
    let wait = gate.entered.notified();
    let asked = tokio::time::timeout(Duration::from_millis(100), wait).await;
    assert!(asked.is_err(), "the store was never asked");
}

#[tokio::test]
async fn unknown_or_closed_sessions_get_a_failed_answer() {
    let (tx, mut h) = rig_default(vec![], vec![]).await;
    let unknown = CompareCommand::DuplicateKeys {
        session: 99,
        pair: dup_pair(),
    };
    send(&tx, unknown);
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 99,
            context: "compare_duplicates",
            ..
        }
    ));
    send(&tx, CompareCommand::Close { session: 1 });
    ask(&tx, dup_pair());
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            session: 1,
            context: "compare_duplicates",
            ..
        }
    ));
}

#[tokio::test]
async fn a_closed_session_stays_silent_for_an_in_flight_request() {
    let gate = Arc::new(Gate::default());
    let source = store(twice_each(2), Mode::Gated(gate.clone()));
    let target = store(vec![], Mode::Plain);
    let (tx, mut h) = rig(source, target, RuntimeConfig::default()).await;
    ask(&tx, dup_pair());
    gate.entered.notified().await;
    send(&tx, CompareCommand::Close { session: 1 });
    // Barrier: a Rows command for the closed session is answered at once.
    let rows = CompareCommand::Rows {
        session: 1,
        pair: 7,
        category: Category::OnlyInSource,
        offset: 0,
        limit: 10,
    };
    send(&tx, rows);
    assert!(matches!(
        next_compare(&mut h).await,
        CompareEvent::Failed {
            context: "compare_rows",
            ..
        }
    ));
    gate.release.notify_one();
    assert_silent(&mut h).await;
}

#[tokio::test]
async fn a_panicking_store_ends_in_the_commands_failed_event() {
    let source = store(vec![], Mode::Panics);
    let target = store(vec![], Mode::Plain);
    let (tx, mut h) = rig(source, target, RuntimeConfig::default()).await;
    ask(&tx, dup_pair());
    let internal = CoreError::Database("internal error".into());
    assert_eq!(failure(&mut h).await, ("compare_duplicates", internal));
}

#[tokio::test]
async fn a_slow_aggregation_times_out_with_its_own_timeout() {
    let config = RuntimeConfig {
        duplicate_timeout: Duration::from_millis(50),
        command_timeout: Duration::from_secs(5),
        ..RuntimeConfig::default()
    };
    let source = store(vec![], Mode::Never);
    let target = store(vec![], Mode::Plain);
    let (tx, mut h) = rig(source, target, config).await;
    ask(&tx, dup_pair());
    let outcome = tokio::time::timeout(Duration::from_secs(1), failure(&mut h)).await;
    let timeout = CoreError::Database("timeout".into());
    assert_eq!(
        outcome.expect("fails within the duplicate timeout"),
        ("compare_duplicates", timeout)
    );
}
