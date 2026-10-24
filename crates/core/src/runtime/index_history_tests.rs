//! `IndexHistory` needs only the names and definitions of the listed indexes, not their sizes.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use super::*;
use crate::clock;
use crate::history::{ChartRange, HistoryDb, Sample, SeriesKey};
use crate::indexes::{IndexSpec, KeyKind};
use crate::store::{CollectionIndex, Page, Query};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc, mock_index_spec_named};
use bson::Bson;

/// Counts which of the two listings is asked for; the full one reads `collStats` on a server.
struct ListingProbe {
    inner: Arc<InMemoryStore>,
    full: AtomicUsize,
    specs: AtomicUsize,
}

#[async_trait]
impl DocumentStore for ListingProbe {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }
    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }
    async fn find(&self, ns: &Namespace, q: &Query) -> Result<Page> {
        self.inner.find(ns, q).await
    }
    async fn insert(&self, ns: &Namespace, doc: bson::Document) -> Result<Bson> {
        self.inner.insert(ns, doc).await
    }
    async fn replace(&self, ns: &Namespace, id: &Bson, doc: bson::Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }
    async fn delete(&self, ns: &Namespace, id: &Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }
    async fn list_indexes(&self, ns: &Namespace) -> Result<Vec<CollectionIndex>> {
        self.full.fetch_add(1, Ordering::SeqCst);
        self.inner.list_indexes(ns).await
    }
    async fn list_index_specs(&self, ns: &Namespace) -> Result<Vec<IndexSpec>> {
        self.specs.fetch_add(1, Ordering::SeqCst);
        self.inner.list_index_specs(ns).await
    }
}

#[tokio::test]
async fn index_history_lists_the_specs_without_the_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let ns = Namespace::new("testdb", "people");
    let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
    let key = SeriesKey {
        connection: EMBEDDED_SESSION,
        host: "h1:27017".into(),
        db: "testdb".into(),
        collection: "people".into(),
        index_name: "a_1".into(),
        definition_hash: spec.definition_hash(),
    };
    let sample = Sample {
        at_secs: clock::now_secs() - 60,
        ops: 3,
        since_ms: 1_000,
    };
    HistoryDb::open(&path)
        .unwrap()
        .0
        .record(&key, sample)
        .unwrap();
    let inner = Arc::new(InMemoryStore::new().with_docs(&ns, vec![mock_doc()]));
    inner.create_index(&ns, &spec).await.unwrap();
    let probe = Arc::new(ListingProbe {
        inner,
        full: AtomicUsize::new(0),
        specs: AtomicUsize::new(0),
    });
    let config = RuntimeConfig {
        history_path: Some(path),
        ..RuntimeConfig::default()
    };
    let mut h = spawn_runtime_with_config(None, Some(probe.clone()), Arc::new(|| {}), config);

    h.tx.send(Command::IndexHistory {
        session: EMBEDDED_SESSION,
        ns,
        range: ChartRange::Day,
    })
    .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), h.rx.recv())
        .await
        .expect("event within bound")
        .expect("channel open");

    let Event::IndexHistory { series, .. } = event else {
        panic!("expected IndexHistory, got {event:?}");
    };
    assert_eq!(series.len(), 1, "the listed index keeps its series");
    assert_eq!(probe.full.load(Ordering::SeqCst), 0, "no sizes were read");
    assert_eq!(probe.specs.load(Ordering::SeqCst), 1);
}
