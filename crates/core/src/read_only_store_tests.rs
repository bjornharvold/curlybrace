//! `ReadOnlyStore` refuses writes while locked and forwards everything else, including the
//! trait's defaulted methods, to the inner store.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bson::{Bson, Document, doc};

use crate::compare::key::{KeySpec, KeyTuple};
use crate::error::{CoreError, Result};
use crate::indexes::IndexSpec;
use crate::read_only_store::ReadOnlyStore;
use crate::store::{
    CollectionIndex, CollectionKind, CollectionOptions, DocumentStore, DuplicateGroup, IndexStatus,
    IndexUsage, Namespace, Page, Query, Topology, WriteFailure, WriteOp, WriteOutcome,
};
use crate::test_support::mock_objects::mock_index_spec;

/// Counts calls per method and answers every method with a non-default value, so a wrapper that
/// falls back to a trait default is caught. Implementing every method here also means a new
/// REQUIRED trait method breaks this file's compilation until it is handled.
#[derive(Default)]
struct RecordingStore {
    calls: Mutex<HashMap<&'static str, usize>>,
}

impl RecordingStore {
    fn hit(&self, name: &'static str) {
        *self.calls.lock().unwrap().entry(name).or_default() += 1;
    }
    fn count(&self, name: &str) -> usize {
        self.calls.lock().unwrap().get(name).copied().unwrap_or(0)
    }
    fn write_calls(&self) -> usize {
        WRITES.iter().map(|m| self.count(m)).sum()
    }
}

/// Every write method of `DocumentStore`. Adding a write to the trait? Name it here, give
/// `RecordingStore` a counter for it, refuse it in `ReadOnlyStore`, and cover it below.
const WRITES: [&str; 9] = [
    "insert",
    "replace",
    "delete",
    "bulk_write",
    "create_index",
    "drop_index",
    "set_index_hidden",
    "set_index_ttl",
    "create_collection",
];

fn ns() -> Namespace {
    Namespace::new("db", "c")
}

#[async_trait]
impl DocumentStore for RecordingStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.hit("list_databases");
        Ok(vec!["db".into()])
    }
    async fn list_collections(&self, _db: &str) -> Result<Vec<String>> {
        self.hit("list_collections");
        Ok(vec!["c".into()])
    }
    async fn find(&self, _ns: &Namespace, _q: &Query) -> Result<Page> {
        self.hit("find");
        Ok(Page {
            docs: vec![doc! {"_id": 1}],
            skip: 0,
            has_more: true,
        })
    }
    async fn insert(&self, _ns: &Namespace, _doc: Document) -> Result<Bson> {
        self.hit("insert");
        Ok(Bson::Int32(7))
    }
    async fn replace(&self, _ns: &Namespace, _id: &Bson, _doc: Document) -> Result<()> {
        self.hit("replace");
        Ok(())
    }
    async fn delete(&self, _ns: &Namespace, _id: &Bson) -> Result<()> {
        self.hit("delete");
        Ok(())
    }
    async fn ping(&self) -> Result<()> {
        self.hit("ping");
        Err(CoreError::Unsupported("inner ping".into()))
    }
    async fn topology(&self) -> Result<Option<Topology>> {
        self.hit("topology");
        Ok(Some(Topology::Sharded))
    }
    async fn server_version(&self) -> Result<Option<String>> {
        self.hit("server_version");
        Ok(Some("9.9.9".into()))
    }
    async fn bulk_write(&self, _ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> {
        self.hit("bulk_write");
        Ok(ops
            .iter()
            .map(|_| Err(WriteFailure::Rejected(42)))
            .collect())
    }
    async fn estimated_count(&self, _ns: &Namespace) -> Result<u64> {
        self.hit("estimated_count");
        Ok(123)
    }
    async fn scan_sorted(
        &self,
        _ns: &Namespace,
        _filter: &Document,
        _key: &KeySpec,
        _from: Option<&KeyTuple>,
        _limit: u32,
    ) -> Result<Vec<Document>> {
        self.hit("scan_sorted");
        Ok(vec![doc! {"via": "scan_sorted"}])
    }
    async fn scan_sorted_primary(
        &self,
        _ns: &Namespace,
        _filter: &Document,
        _key: &KeySpec,
        _from: Option<&KeyTuple>,
        _limit: u32,
    ) -> Result<Vec<Document>> {
        self.hit("scan_sorted_primary");
        Ok(vec![doc! {"via": "scan_sorted_primary"}])
    }
    async fn key_index_status(&self, _ns: &Namespace, _key: &KeySpec) -> Result<IndexStatus> {
        self.hit("key_index_status");
        Ok(IndexStatus::NotIndexed)
    }
    async fn list_collection_kinds(&self, _db: &str) -> Result<Vec<(String, CollectionKind)>> {
        self.hit("list_collection_kinds");
        Ok(vec![("v".into(), CollectionKind::View)])
    }
    async fn duplicate_keys(
        &self,
        _ns: &Namespace,
        _filter: &Document,
        _key: &KeySpec,
        _limit: u32,
    ) -> Result<Vec<DuplicateGroup>> {
        self.hit("duplicate_keys");
        Ok(vec![DuplicateGroup {
            key: KeyTuple(vec![Bson::Int32(1)]),
            count: 2,
        }])
    }
    async fn list_indexes(&self, _ns: &Namespace) -> Result<Vec<CollectionIndex>> {
        self.hit("list_indexes");
        Ok(vec![CollectionIndex {
            spec: mock_index_spec(),
            size_bytes: Some(4096),
        }])
    }
    async fn list_index_specs(&self, _ns: &Namespace) -> Result<Vec<IndexSpec>> {
        self.hit("list_index_specs");
        Ok(vec![mock_index_spec()])
    }
    async fn index_stats(&self, _ns: &Namespace) -> Result<Vec<IndexUsage>> {
        self.hit("index_stats");
        Ok(vec![IndexUsage {
            name: "via_index_stats".into(),
            host: "h:27017".into(),
            ops: 5,
            since_ms: 1_000,
            spec: mock_index_spec(),
        }])
    }
    async fn create_index(&self, _ns: &Namespace, _spec: &IndexSpec) -> Result<()> {
        self.hit("create_index");
        Ok(())
    }
    async fn drop_index(&self, _ns: &Namespace, _name: &str) -> Result<()> {
        self.hit("drop_index");
        Ok(())
    }
    async fn set_index_hidden(&self, _ns: &Namespace, _name: &str, _hidden: bool) -> Result<()> {
        self.hit("set_index_hidden");
        Ok(())
    }
    async fn set_index_ttl(&self, _ns: &Namespace, _name: &str, _seconds: i64) -> Result<()> {
        self.hit("set_index_ttl");
        Ok(())
    }
    async fn collection_options(&self, _ns: &Namespace) -> Result<Option<CollectionOptions>> {
        self.hit("collection_options");
        Ok(Some(recorded_options()))
    }
    async fn create_collection(&self, _ns: &Namespace, _options: &CollectionOptions) -> Result<()> {
        self.hit("create_collection");
        Ok(())
    }
}

/// Options no trait default would ever produce.
fn recorded_options() -> CollectionOptions {
    CollectionOptions {
        validation_level: Some("via_collection_options".into()),
        ..CollectionOptions::default()
    }
}

fn wrapped(locked: bool) -> (Arc<RecordingStore>, Arc<AtomicBool>, Arc<dyn DocumentStore>) {
    let inner = Arc::new(RecordingStore::default());
    let flag = Arc::new(AtomicBool::new(locked));
    let store = ReadOnlyStore::wrap(inner.clone(), flag.clone());
    (inner, flag, store)
}

/// Runs every write; true per write when it came back `ReadOnly`.
async fn refused_writes(store: &dyn DocumentStore) -> [bool; WRITES.len()] {
    let id = Bson::Int32(1);
    let is_ro = |e: Option<CoreError>| e == Some(CoreError::ReadOnly);
    [
        is_ro(store.insert(&ns(), doc! {}).await.err()),
        is_ro(store.replace(&ns(), &id, doc! {}).await.err()),
        is_ro(store.delete(&ns(), &id).await.err()),
        is_ro(
            store
                .bulk_write(&ns(), vec![WriteOp::Delete { id: id.clone() }])
                .await
                .err(),
        ),
        is_ro(store.create_index(&ns(), &mock_index_spec()).await.err()),
        is_ro(store.drop_index(&ns(), "a_1").await.err()),
        is_ro(store.set_index_hidden(&ns(), "a_1", true).await.err()),
        is_ro(store.set_index_ttl(&ns(), "a_1", 60).await.err()),
        is_ro(
            store
                .create_collection(&ns(), &CollectionOptions::default())
                .await
                .err(),
        ),
    ]
}

#[tokio::test]
async fn locked_store_refuses_every_write_with_read_only() {
    let (inner, _flag, store) = wrapped(true);
    assert_eq!(refused_writes(store.as_ref()).await, [true; WRITES.len()]);
    assert_eq!(inner.write_calls(), 0);
    assert!(store.is_read_only());
}

#[tokio::test]
async fn unlocked_store_forwards_writes() {
    let (inner, _flag, store) = wrapped(false);
    let id = Bson::Int32(1);
    assert_eq!(store.insert(&ns(), doc! {}).await, Ok(Bson::Int32(7)));
    assert_eq!(store.replace(&ns(), &id, doc! {}).await, Ok(()));
    assert_eq!(store.delete(&ns(), &id).await, Ok(()));
    let out = store
        .bulk_write(&ns(), vec![WriteOp::Delete { id }])
        .await
        .unwrap();
    assert_eq!(out, vec![Err(WriteFailure::Rejected(42))]);
    for w in ["insert", "replace", "delete", "bulk_write"] {
        assert_eq!(inner.count(w), 1, "{w}");
    }
    assert!(!store.is_read_only());
}

#[tokio::test]
async fn read_only_refuses_create_drop_hide_and_ttl() {
    let (inner, _flag, store) = wrapped(true);
    let spec = mock_index_spec();
    let refused = [
        store.create_index(&ns(), &spec).await,
        store.drop_index(&ns(), "a_1").await,
        store.set_index_hidden(&ns(), "a_1", true).await,
        store.set_index_ttl(&ns(), "a_1", 60).await,
    ];
    assert!(refused.iter().all(|r| *r == Err(CoreError::ReadOnly)));
    assert_eq!(inner.write_calls(), 0);

    let (inner, _flag, store) = wrapped(false);
    assert_eq!(store.create_index(&ns(), &spec).await, Ok(()));
    assert_eq!(store.drop_index(&ns(), "a_1").await, Ok(()));
    assert_eq!(store.set_index_hidden(&ns(), "a_1", true).await, Ok(()));
    assert_eq!(store.set_index_ttl(&ns(), "a_1", 60).await, Ok(()));
    for w in [
        "create_index",
        "drop_index",
        "set_index_hidden",
        "set_index_ttl",
    ] {
        assert_eq!(inner.count(w), 1, "{w}");
    }
}

#[tokio::test]
async fn create_collection_is_refused_when_locked() {
    let (inner, _flag, store) = wrapped(true);
    let created = store
        .create_collection(&ns(), &CollectionOptions::default())
        .await;
    assert_eq!(created, Err(CoreError::ReadOnly));
    assert_eq!(inner.count("create_collection"), 0);

    let (inner, _flag, store) = wrapped(false);
    let created = store
        .create_collection(&ns(), &CollectionOptions::default())
        .await;
    assert_eq!(created, Ok(()));
    assert_eq!(inner.count("create_collection"), 1);
}

#[tokio::test]
async fn collection_options_is_forwarded_when_locked() {
    let (inner, _flag, store) = wrapped(true);
    let options = store.collection_options(&ns()).await.unwrap();
    assert_eq!(options, Some(recorded_options()));
    assert_eq!(inner.count("collection_options"), 1);
}

#[tokio::test]
async fn read_only_forwards_list_index_specs_to_the_store_that_can_skip_the_sizes() {
    let (inner, _flag, store) = wrapped(true);
    let specs = store.list_index_specs(&ns()).await.unwrap();
    assert_eq!(specs, vec![mock_index_spec()]);
    assert_eq!(inner.count("list_index_specs"), 1);
    assert_eq!(inner.count("list_indexes"), 0, "no sizes were asked for");
}

#[tokio::test]
async fn read_only_forwards_list_indexes_and_index_stats() {
    // Locked on purpose: reading indexes must work on a read-only connection.
    let (inner, _flag, store) = wrapped(true);
    let listed = store.list_indexes(&ns()).await.unwrap();
    assert_eq!(listed[0].size_bytes, Some(4096));
    let stats = store.index_stats(&ns()).await.unwrap();
    assert_eq!(stats[0].name, "via_index_stats");
    assert_eq!(inner.count("list_indexes"), 1);
    assert_eq!(inner.count("index_stats"), 1);
}

#[tokio::test]
async fn locking_after_wrap_refuses_the_next_write() {
    let (inner, flag, store) = wrapped(false);
    assert!(store.insert(&ns(), doc! {}).await.is_ok());
    flag.store(true, Ordering::SeqCst);
    assert!(store.is_read_only());
    assert_eq!(store.insert(&ns(), doc! {}).await, Err(CoreError::ReadOnly));
    assert_eq!(inner.count("insert"), 1);
    flag.store(false, Ordering::SeqCst);
    assert!(store.insert(&ns(), doc! {}).await.is_ok());
}

#[tokio::test]
async fn every_write_method_is_covered() {
    // The four calls in `refused_writes` must reach exactly the methods named in WRITES, both
    // when forwarded and when refused.
    let (inner, _flag, store) = wrapped(false);
    refused_writes(store.as_ref()).await;
    for w in WRITES {
        assert_eq!(inner.count(w), 1, "{w} was not forwarded when unlocked");
    }
    let (inner, _flag, store) = wrapped(true);
    assert_eq!(refused_writes(store.as_ref()).await, [true; WRITES.len()]);
    assert_eq!(inner.write_calls(), 0);
}

#[tokio::test]
async fn reads_and_defaulted_methods_forward_to_inner() {
    // Locked on purpose: reads must work on a read-only connection.
    let (inner, _flag, store) = wrapped(true);
    let (n, key) = (ns(), KeySpec::id());
    let (f, q) = (doc! {}, Query::default());

    assert_eq!(store.list_databases().await.unwrap(), vec!["db"]);
    assert_eq!(store.list_collections("db").await.unwrap(), vec!["c"]);
    assert!(store.find(&n, &q).await.unwrap().has_more);
    assert!(store.ping().await.is_err());
    assert_eq!(store.topology().await.unwrap(), Some(Topology::Sharded));
    assert_eq!(
        store.server_version().await.unwrap().as_deref(),
        Some("9.9.9")
    );
    assert_eq!(store.estimated_count(&n).await.unwrap(), 123);
    assert_eq!(
        store.scan_sorted(&n, &f, &key, None, 5).await.unwrap(),
        vec![doc! {"via": "scan_sorted"}]
    );
    assert_eq!(
        store
            .scan_sorted_primary(&n, &f, &key, None, 5)
            .await
            .unwrap(),
        vec![doc! {"via": "scan_sorted_primary"}]
    );
    assert_eq!(
        store.key_index_status(&n, &key).await.unwrap(),
        IndexStatus::NotIndexed
    );
    assert_eq!(
        store.list_collection_kinds("db").await.unwrap(),
        vec![("v".to_string(), CollectionKind::View)]
    );
    assert_eq!(
        store.duplicate_keys(&n, &f, &key, 3).await.unwrap().len(),
        1
    );
    assert_eq!(
        store.collection_options(&n).await.unwrap(),
        Some(recorded_options())
    );

    for m in [
        "list_databases",
        "list_collections",
        "find",
        "ping",
        "topology",
        "server_version",
        "estimated_count",
        "scan_sorted",
        "scan_sorted_primary",
        "key_index_status",
        "list_collection_kinds",
        "duplicate_keys",
        "collection_options",
    ] {
        assert_eq!(inner.count(m), 1, "{m}");
    }
}
