//! One poll at a time, driven by hand (no ticker, no timing): gaps recorded once, the pass
//! bounded in time, failures never reported as idle, marks forgotten when they stop applying.

use super::*;
use crate::runtime::profiler::{Marks, poll_once};

fn cfg() -> SamplerConfig {
    SamplerConfig::default()
}

async fn poll(
    probe: &Arc<Probe>,
    db: &Arc<HistoryDb>,
    marks: &mut Marks,
) -> std::result::Result<ProfilerStatus, String> {
    poll_with(probe, db, &cfg(), marks).await
}

async fn poll_with(
    probe: &Arc<Probe>,
    db: &Arc<HistoryDb>,
    cfg: &SamplerConfig,
    marks: &mut Marks,
) -> std::result::Result<ProfilerStatus, String> {
    let store: &dyn DocumentStore = &**probe;
    poll_once(store, db, CONN, cfg, NOW, marks)
        .await
        .map(|poll| poll.status())
        .map_err(|error| crate::error_name::variant_of(error.error()).to_owned())
}

fn gap_secs(db: &HistoryDb) -> Vec<u64> {
    summaries(db, "shop", "orders")
        .iter()
        .flat_map(|s| s.buckets.iter())
        .map(|b| b.gap_secs)
        .filter(|g| *g > 0)
        .collect()
}

fn shop(entries: Vec<ProfileEntry>) -> InMemoryStore {
    store_of("shop", 1, entries)
}

const LOST: i64 = 1_800_000;

// ------------------------------------------------------- gaps recorded once

#[tokio::test]
async fn a_gap_is_recorded_once_when_the_page_after_it_fails_to_record() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(0)), scan(at(9))]));
    let db = history();
    let mut marks = Marks::default();
    assert!(poll(&probe, &db, &mut marks).await.is_ok());

    probe.swap(shop(vec![scan(at(LOST)), scan(at(LOST + 1))]));
    db.exec_for_tests("ALTER TABLE shape_bucket RENAME TO shape_bucket_gone");
    let failed = poll(&probe, &db, &mut marks).await;
    assert!(failed.is_err(), "the page could not be recorded");
    db.exec_for_tests("ALTER TABLE shape_bucket_gone RENAME TO shape_bucket");

    let status = poll(&probe, &db, &mut marks).await.unwrap();
    assert_eq!(
        status,
        ProfilerStatus::MissedEntries { db: "shop".into() },
        "still reported while it applies"
    );
    let total: u64 = gap_secs(&db).iter().sum();
    assert_eq!(total, (LOST - 9) as u64 / 1_000, "one gap, not two");
    assert_eq!(totals(&db, "shop", "orders").0, 4, "each page once");
}

#[tokio::test]
async fn a_thirty_minute_loss_is_spread_over_three_buckets() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(0))]));
    let db = history();
    let mut marks = Marks::default();
    poll(&probe, &db, &mut marks).await.unwrap();
    probe.swap(shop(vec![scan(at(LOST))]));
    poll(&probe, &db, &mut marks).await.unwrap();
    assert_eq!(gap_secs(&db), vec![600, 600, 600]);
}

// ------------------------------------------------------------- bounded pass

/// Every profile read hangs; the profiler is on in every database.
struct Hanging {
    dbs: Vec<String>,
    reads: AtomicUsize,
}

#[async_trait::async_trait]
impl DocumentStore for Hanging {
    async fn list_databases(&self) -> Result<Vec<String>> {
        Ok(self.dbs.clone())
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
    async fn profiling_level(&self, _db: &str) -> Result<ProfilingState> {
        Ok(ProfilingState {
            level: 1,
            slow_ms: 100,
        })
    }
    async fn read_profile(&self, _db: &str, _since: i64) -> Result<ProfileRead> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        std::future::pending().await
    }
}

#[tokio::test(start_paused = true)]
async fn consecutive_timeouts_end_the_pass_early() {
    let dbs: Vec<String> = (0..10).map(|i| format!("db{i}")).collect();
    let store = Arc::new(Hanging {
        dbs,
        reads: AtomicUsize::new(0),
    });
    let config = SamplerConfig {
        command_timeout: secs(5),
        ..SamplerConfig::default()
    };
    let began = Instant::now();
    let mut marks = Marks::default();
    let result = poll_once(&*store, &history(), CONN, &config, NOW, &mut marks).await;
    assert!(result.is_err(), "nothing was read, so it is not idle");
    assert_eq!(
        store.reads.load(Ordering::SeqCst),
        crate::runtime::sampler::MAX_CONSECUTIVE_TIMEOUTS,
        "the cap, not every database"
    );
    assert!(began.elapsed() < secs(5 * 4), "{:?}", began.elapsed());
}

#[tokio::test]
async fn all_reads_failing_reports_failing_not_idle() {
    let boom = CoreError::Database("boom".into());
    let probe = Probe::new(&["a", "b"], InMemoryStore::new());
    probe.fail_levels(Some(boom));
    let db = history();
    let mut marks = Marks::default();
    assert!(poll(&probe, &db, &mut marks).await.is_err());

    let (status_tx, mut status) = unbounded_channel();
    let (_cfg, cfg_rx) = watch::channel(SamplerConfig::default());
    let store: Arc<dyn DocumentStore> = probe.clone();
    let task = tokio::spawn(run(store, db, CONN, cfg_rx, status_tx, clock()));
    let first = tokio::time::timeout(secs(500), status.recv()).await;
    assert_eq!(first, Ok(Some(ProfilerStatus::Failing)));
    task.abort();
}

#[tokio::test]
async fn nothing_to_read_is_idle() {
    let probe = Probe::new(&["shop"], store_of("shop", 0, vec![]));
    let status = poll(&probe, &history(), &mut Marks::default()).await;
    assert_eq!(status.ok(), Some(ProfilerStatus::Idle));
}

#[tokio::test]
async fn a_db_failing_while_another_reads_is_still_reading() {
    let inner = store_of("good", 1, vec![entry_on(at(1), "good.t", doc! { "a": 1 })])
        .with_profile_error("bad", CoreError::Database("boom".into()));
    let probe = Probe::new(&["bad", "good"], inner);
    let status = poll(&probe, &history(), &mut Marks::default()).await;
    assert_eq!(status.ok(), Some(ProfilerStatus::Reading { databases: 1 }));
}

// -------------------------------------------------------------- more cases

#[tokio::test(start_paused = true)]
async fn unauthorised_recovers_after_access_is_restored() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(1))]));
    probe.fail_levels(Some(CoreError::Unauthorized));
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Paused(SamplerPause::NotAuthorized)
    );
    probe.fail_levels(None);
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    assert_eq!(totals(&db, "shop", "orders").0, 1);
    running.handle.abort();
}

#[tokio::test(start_paused = true)]
async fn roll_off_reports_missed_entries_once_then_returns_to_reading_and_records_the_gap_once() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(0)), scan(at(9))]));
    let db = history();
    let mut running = start(&probe, &db, SamplerConfig::default());
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    probe.swap(shop(vec![scan(at(LOST)), scan(at(LOST + 1))]));
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::MissedEntries { db: "shop".into() }
    );
    assert_eq!(
        next_status(&mut running).await,
        ProfilerStatus::Reading { databases: 1 }
    );
    until("two more reads", || probe.read_count() >= 5).await;
    let total: u64 = gap_secs(&db).iter().sum();
    assert_eq!(total, (LOST - 9) as u64 / 1_000);
    running.handle.abort();
}

#[tokio::test]
async fn page_cap_of_100_leaves_the_mark_and_the_next_tick_continues() {
    let entries: Vec<ProfileEntry> = (1..=100_500).map(|i| scan(at(i))).collect();
    let probe = Probe::new(&["shop"], shop(entries));
    let db = history();
    let mut marks = Marks::default();
    poll(&probe, &db, &mut marks).await.unwrap();
    assert_eq!(probe.read_count(), 100, "the cap");
    assert_eq!(totals(&db, "shop", "orders").0, 100_000);
    poll(&probe, &db, &mut marks).await.unwrap();
    assert_eq!(totals(&db, "shop", "orders").0, 100_500, "none skipped");
    assert!(gap_secs(&db).is_empty(), "the backlog is no loss");
}

#[tokio::test]
async fn profiler_off_then_on_again_records_no_gap() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(1))]));
    let db = history();
    let mut marks = Marks::default();
    poll(&probe, &db, &mut marks).await.unwrap();
    probe.swap(store_of("shop", 0, vec![]));
    assert_eq!(
        poll(&probe, &db, &mut marks).await.ok(),
        Some(ProfilerStatus::Idle)
    );
    probe.swap(shop(vec![scan(at(LOST))]));
    poll(&probe, &db, &mut marks).await.unwrap();
    assert!(gap_secs(&db).is_empty());
    assert_eq!(totals(&db, "shop", "orders").0, 2);
}

#[tokio::test]
async fn host_change_resets_the_anchor_without_a_gap() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(1))]));
    let db = history();
    let mut marks = Marks::default();
    poll(&probe, &db, &mut marks).await.unwrap();
    probe.swap(shop(vec![scan(at(LOST))]));
    probe.set_host("other:27017");
    let status = poll(&probe, &db, &mut marks).await.unwrap();
    assert_eq!(status, ProfilerStatus::Reading { databases: 1 });
    assert!(gap_secs(&db).is_empty(), "another node's profile is no loss");
}

#[tokio::test]
async fn dropped_databases_forget_their_marks() {
    let probe = Probe::new(&["shop"], shop(vec![scan(at(1))]));
    let db = history();
    let mut marks = Marks::default();
    poll(&probe, &db, &mut marks).await.unwrap();
    assert!(marks.tracks("shop"));
    let gone = Probe::new(&[], InMemoryStore::new());
    poll(&gone, &db, &mut marks).await.unwrap();
    assert!(!marks.tracks("shop"));
}
