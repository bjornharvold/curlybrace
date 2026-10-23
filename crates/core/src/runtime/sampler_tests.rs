use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::sync::watch;
use tokio::time::Instant;
use uuid::Uuid;

use super::sampler::*;
use crate::error::{CoreError, Result};
use crate::history::{HistoryDb, Sample, SeriesKey};
use crate::indexes::KeyKind;
use crate::store::DocumentStore;
use crate::store::{CollectionKind, IndexUsage, Namespace};
use crate::test_support::InMemoryStore;
use crate::test_support::mock_objects::{mock_doc, mock_index_spec_named};

const CONN: Uuid = Uuid::from_u128(7);
const NOW: i64 = 1_800_000_000;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn usage(name: &str, host: &str, ops: i64) -> IndexUsage {
    IndexUsage {
        name: name.into(),
        host: host.into(),
        ops,
        since_ms: 1_000,
        spec: mock_index_spec_named(name, &[("a", KeyKind::Asc)]),
    }
}

/// A store holding one document and one scripted index in each `db.collection`.
fn store_with(collections: &[(&str, &str)]) -> InMemoryStore {
    collections.iter().fold(InMemoryStore::new(), |s, (db, c)| {
        let ns = Namespace::new(*db, *c);
        s.with_docs(&ns, vec![mock_doc()])
            .with_index_usage(&ns, vec![usage("a_1", "h1:27017", 5)])
    })
}

fn history() -> Arc<HistoryDb> {
    Arc::new(HistoryDb::open_in_memory().unwrap())
}

fn series_of(db: &str, collection: &str, host: &str) -> SeriesKey {
    SeriesKey {
        connection: CONN,
        host: host.into(),
        db: db.into(),
        collection: collection.into(),
        index_name: "a_1".into(),
        definition_hash: mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]).definition_hash(),
    }
}

/// A boundary double around `InMemoryStore`: it can report other collection kinds, delay and
/// fail `index_stats`, and it records every call and how many were in flight at once.
struct ProbeStore {
    inner: InMemoryStore,
    kinds: Mutex<HashMap<String, Vec<(String, CollectionKind)>>>,
    stats_delay: Duration,
    /// Fails `index_stats` for this collection, or for every collection when the name is `None`.
    failure: Mutex<Option<(Option<String>, CoreError)>>,
    calls: Mutex<Vec<(Namespace, Instant)>>,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl ProbeStore {
    fn new(inner: InMemoryStore) -> Self {
        Self {
            inner,
            kinds: Mutex::default(),
            stats_delay: Duration::ZERO,
            failure: Mutex::default(),
            calls: Mutex::default(),
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.stats_delay = delay;
        self
    }

    fn with_kinds(self, db: &str, kinds: &[(&str, CollectionKind)]) -> Self {
        let listed = kinds.iter().map(|(n, k)| ((*n).to_string(), *k)).collect();
        self.kinds.lock().unwrap().insert(db.to_string(), listed);
        self
    }

    fn fail(&self, collection: Option<&str>, error: Option<CoreError>) {
        *self.failure.lock().unwrap() = error.map(|e| (collection.map(str::to_string), e));
    }

    fn called(&self) -> Vec<Namespace> {
        let calls = self.calls.lock().unwrap();
        calls.iter().map(|(ns, _)| ns.clone()).collect()
    }

    fn call_offsets(&self, start: Instant) -> Vec<u64> {
        let calls = self.calls.lock().unwrap();
        calls
            .iter()
            .map(|(_, at)| at.duration_since(start).as_secs())
            .collect()
    }
}

#[async_trait]
impl DocumentStore for ProbeStore {
    async fn list_databases(&self) -> Result<Vec<String>> {
        self.inner.list_databases().await
    }

    async fn list_collections(&self, db: &str) -> Result<Vec<String>> {
        self.inner.list_collections(db).await
    }

    async fn find(&self, ns: &Namespace, q: &crate::store::Query) -> Result<crate::store::Page> {
        self.inner.find(ns, q).await
    }

    async fn insert(&self, ns: &Namespace, doc: bson::Document) -> Result<bson::Bson> {
        self.inner.insert(ns, doc).await
    }

    async fn replace(&self, ns: &Namespace, id: &bson::Bson, doc: bson::Document) -> Result<()> {
        self.inner.replace(ns, id, doc).await
    }

    async fn delete(&self, ns: &Namespace, id: &bson::Bson) -> Result<()> {
        self.inner.delete(ns, id).await
    }

    async fn list_collection_kinds(&self, db: &str) -> Result<Vec<(String, CollectionKind)>> {
        let scripted = self.kinds.lock().unwrap().get(db).cloned();
        match scripted {
            Some(kinds) => Ok(kinds),
            None => self.inner.list_collection_kinds(db).await,
        }
    }

    async fn index_stats(&self, ns: &Namespace) -> Result<Vec<IndexUsage>> {
        self.calls
            .lock()
            .unwrap()
            .push((ns.clone(), Instant::now()));
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(self.stats_delay).await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        let failure = self.failure.lock().unwrap().clone();
        match failure {
            Some((only, error)) if only.as_deref().is_none_or(|c| c == ns.collection) => Err(error),
            _ => self.inner.index_stats(ns).await,
        }
    }
}

async fn sample(
    store: &dyn DocumentStore,
    db: &Arc<HistoryDb>,
    cfg: &SamplerConfig,
) -> Result<usize> {
    sample_once(store, db, CONN, cfg, NOW).await
}

fn start(
    store: Arc<ProbeStore>,
    db: Arc<HistoryDb>,
    cfg: SamplerConfig,
) -> (
    tokio::task::JoinHandle<()>,
    UnboundedReceiver<SamplerStatus>,
    watch::Sender<SamplerConfig>,
) {
    let (cfg_tx, cfg_rx) = watch::channel(cfg);
    let (status_tx, status_rx) = unbounded_channel();
    let handle = spawn(store, db, CONN, cfg_rx, status_tx);
    (handle, status_rx, cfg_tx)
}

#[tokio::test]
async fn it_samples_every_user_collection_and_skips_admin_local_config() {
    let store = store_with(&[
        ("shop", "orders"),
        ("shop", "users"),
        ("admin", "x"),
        ("local", "y"),
        ("config", "z"),
    ]);
    let db = history();

    let recorded = sample(&store, &db, &SamplerConfig::default())
        .await
        .unwrap();

    assert_eq!(recorded, 2);
    for collection in ["orders", "users"] {
        let key = series_of("shop", collection, "h1:27017");
        let got = db.latest(&key).unwrap();
        assert_eq!(
            got,
            Some(Sample {
                at_secs: NOW,
                ops: 5,
                since_ms: 1_000
            })
        );
    }
    for system in ["admin", "local", "config"] {
        assert!(db.series_for(CONN, system, "x").unwrap().is_empty());
    }
}

#[tokio::test]
async fn it_skips_user_excluded_databases() {
    let store = store_with(&[("shop", "orders"), ("scratch", "tmp")]);
    let db = history();
    let cfg = SamplerConfig {
        excluded_dbs: ["scratch".to_string()].into(),
        ..SamplerConfig::default()
    };

    assert_eq!(sample(&store, &db, &cfg).await.unwrap(), 1);

    assert!(db.series_for(CONN, "scratch", "tmp").unwrap().is_empty());
    assert_eq!(db.series_for(CONN, "shop", "orders").unwrap().len(), 1);
}

#[tokio::test]
async fn it_skips_views_and_timeseries_collections() {
    let store = ProbeStore::new(store_with(&[("shop", "orders")])).with_kinds(
        "shop",
        &[
            ("orders", CollectionKind::Collection),
            ("order_view", CollectionKind::View),
            ("readings", CollectionKind::Timeseries),
            ("system.views", CollectionKind::Collection),
            ("system.buckets.readings", CollectionKind::Collection),
        ],
    );
    let db = history();

    let recorded = sample(&store, &db, &SamplerConfig::default())
        .await
        .unwrap();

    assert_eq!(recorded, 1);
    assert_eq!(store.called(), vec![Namespace::new("shop", "orders")]);
}

#[tokio::test]
async fn it_records_one_series_per_node() {
    let ns = Namespace::new("shop", "orders");
    let store = InMemoryStore::new()
        .with_docs(&ns, vec![mock_doc()])
        .with_index_usage(
            &ns,
            vec![usage("a_1", "h1:27017", 5), usage("a_1", "h2:27017", 9)],
        );
    let db = history();

    assert_eq!(
        sample(&store, &db, &SamplerConfig::default())
            .await
            .unwrap(),
        2
    );

    let first = db.latest(&series_of("shop", "orders", "h1:27017")).unwrap();
    let second = db.latest(&series_of("shop", "orders", "h2:27017")).unwrap();
    assert_eq!(first.map(|s| s.ops), Some(5));
    assert_eq!(second.map(|s| s.ops), Some(9));
}

#[tokio::test]
async fn a_failing_collection_does_not_hide_the_others() {
    let store = ProbeStore::new(store_with(&[("shop", "orders"), ("shop", "users")]));
    store.fail(Some("orders"), Some(CoreError::Database("gone".into())));
    let db = history();

    let recorded = sample(&store, &db, &SamplerConfig::default())
        .await
        .unwrap();

    assert_eq!(recorded, 1);
    assert_eq!(db.series_for(CONN, "shop", "users").unwrap().len(), 1);
}

#[tokio::test]
async fn a_pause_class_error_is_returned_after_recording_what_succeeded() {
    let store = ProbeStore::new(store_with(&[("shop", "orders"), ("shop", "users")]));
    store.fail(Some("orders"), Some(CoreError::Unauthorized));
    let db = history();

    let result = sample(&store, &db, &SamplerConfig::default()).await;

    assert_eq!(result, Err(CoreError::Unauthorized));
    assert_eq!(db.series_for(CONN, "shop", "users").unwrap().len(), 1);
}

#[test]
fn pause_reason_classifies_errors() {
    use SamplerPause::*;
    let cases = [
        (CoreError::Unauthorized, Some(NotAuthorized)),
        (CoreError::AuthFailed, Some(NotAuthorized)),
        (CoreError::Unreachable, Some(ConnectionLost)),
        (CoreError::UnreachableSsh, Some(ConnectionLost)),
        (
            CoreError::ConnectionInterrupted("x".into()),
            Some(ConnectionLost),
        ),
        (CoreError::Database("x".into()), None),
        (CoreError::NotFound, None),
    ];
    for (error, want) in cases {
        assert_eq!(pause_reason(&error), want, "{error:?}");
    }
    let history_failure = TickError::History(CoreError::Database("history: full".into()));
    assert_eq!(history_failure.pause(), Some(HistoryUnavailable));
}

#[test]
fn backoff_doubles_and_stops_at_ten_minutes() {
    let waits: Vec<u64> = (1..=6).map(|n| backoff(secs(60), n).as_secs()).collect();
    assert_eq!(waits, vec![120, 240, 480, 600, 600, 600]);
    assert_eq!(backoff(secs(60), 200), secs(600));
}

#[tokio::test(start_paused = true)]
async fn a_not_authorized_error_pauses_with_not_authorized_and_keeps_the_task_alive() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    store.fail(None, Some(CoreError::Unauthorized));

    let (handle, mut status, _cfg) = start(store.clone(), history(), SamplerConfig::default());

    assert_eq!(
        status.recv().await,
        Some(SamplerStatus::Paused(SamplerPause::NotAuthorized))
    );
    tokio::time::sleep(secs(300)).await;
    assert!(!handle.is_finished());
    assert!(store.called().len() > 1, "it keeps trying while paused");
    assert!(
        status.try_recv().is_err(),
        "a repeated failure reports no new status"
    );
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn a_connection_error_pauses_and_backs_off_up_to_ten_minutes() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    store.fail(None, Some(CoreError::Unreachable));
    let began = Instant::now();

    let (handle, mut status, _cfg) = start(store.clone(), history(), SamplerConfig::default());
    tokio::time::sleep(secs(1_500)).await;

    assert_eq!(
        status.recv().await,
        Some(SamplerStatus::Paused(SamplerPause::ConnectionLost))
    );
    assert_eq!(store.call_offsets(began), vec![0, 120, 360, 840, 1_440]);
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn it_resumes_and_reports_running_after_a_good_tick() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    store.fail(None, Some(CoreError::Unreachable));

    let (handle, mut status, _cfg) = start(store.clone(), history(), SamplerConfig::default());
    assert_eq!(
        status.recv().await,
        Some(SamplerStatus::Paused(SamplerPause::ConnectionLost))
    );
    store.fail(None, None);
    assert_eq!(status.recv().await, Some(SamplerStatus::Running));

    tokio::time::sleep(secs(300)).await;
    assert!(
        status.try_recv().is_err(),
        "Running is reported on change only"
    );
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn a_good_first_tick_reports_running_once() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));

    let (handle, mut status, _cfg) = start(store, history(), SamplerConfig::default());

    assert_eq!(status.recv().await, Some(SamplerStatus::Running));
    tokio::time::sleep(secs(300)).await;
    assert!(status.try_recv().is_err());
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn concurrency_never_exceeds_the_configured_limit() {
    let collections: Vec<(String, String)> = (0..10)
        .map(|i| ("shop".to_string(), format!("c{i}")))
        .collect();
    let refs: Vec<(&str, &str)> = collections
        .iter()
        .map(|(d, c)| (d.as_str(), c.as_str()))
        .collect();
    let store = ProbeStore::new(store_with(&refs)).delayed(secs(10));
    let cfg = SamplerConfig {
        max_concurrency: 3,
        ..SamplerConfig::default()
    };
    let db = history();

    let recorded = sample(&store, &db, &cfg).await.unwrap();

    assert_eq!(recorded, 10);
    assert_eq!(store.peak.load(Ordering::SeqCst), 3);
}

#[tokio::test(start_paused = true)]
async fn an_overrunning_tick_skips_the_next_instead_of_stacking() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])).delayed(secs(150)));
    let began = Instant::now();

    let (handle, _status, _cfg) = start(store.clone(), history(), SamplerConfig::default());
    tokio::time::sleep(secs(200)).await;

    // The first run ends at 150 s and missed the ticks at 60 s and 120 s. They are dropped: the
    // next run starts on the 180 s tick, not at once, and never two at the same time.
    assert_eq!(store.call_offsets(began), vec![0, 180]);
    assert_eq!(store.peak.load(Ordering::SeqCst), 1);
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn a_changed_interval_applies_from_the_next_tick() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let began = Instant::now();

    let (handle, _status, cfg) = start(store.clone(), history(), SamplerConfig::default());
    tokio::time::sleep(secs(10)).await;
    cfg.send(SamplerConfig {
        interval: secs(300),
        ..SamplerConfig::default()
    })
    .unwrap();
    tokio::time::sleep(secs(700)).await;

    let offsets = store.call_offsets(began);
    assert_eq!(offsets[0], 0);
    assert!(
        offsets.windows(2).skip(1).all(|w| w[1] - w[0] == 300),
        "{offsets:?}"
    );
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn the_task_stops_when_aborted() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let (handle, _status, _cfg) = start(store.clone(), history(), SamplerConfig::default());
    tokio::time::sleep(secs(1)).await;

    handle.abort();

    assert!(handle.await.unwrap_err().is_cancelled());
    let before = store.called().len();
    tokio::time::sleep(secs(600)).await;
    assert_eq!(store.called().len(), before);
}

#[tokio::test(start_paused = true)]
async fn the_task_ends_when_the_status_channel_is_closed() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let (handle, status, _cfg) = start(store, history(), SamplerConfig::default());
    drop(status);

    handle.await.unwrap();
}
