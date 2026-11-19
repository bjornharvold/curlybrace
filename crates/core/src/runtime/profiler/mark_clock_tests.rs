//! A watermark ahead of the server's own clock (a clock set back, a restored or copied profile)
//! would make every read return nothing until real time catches up. The poller moves it back to
//! the server's time, taken from the `hello` of the read, and never counts an entry stamped
//! later than that time (it can only be one already counted): nothing is read twice.

use std::sync::Arc;
use std::time::Duration;

use bson::doc;
use uuid::Uuid;

use super::mark_clock::{MARK_AHEAD_TOLERANCE_MS, clamp_ahead};
use super::{Marks, poll_once};
use crate::history::{ChartRange, HistoryDb, ProfileMark};
use crate::profile::ProfileEntry;
use crate::runtime::profiler_status::ProfilerStatus;
use crate::runtime::sampler::SamplerConfig;
use crate::store::Namespace;
use crate::test_support::InMemoryStore;

const CONN: Uuid = Uuid::from_u128(11);
const NOW_SECS: i64 = 1_800_000_000;
const SERVER_NOW: i64 = NOW_SECS * 1_000;
const HOUR_MS: i64 = 3_600_000;

fn entry(ts_millis: i64) -> ProfileEntry {
    ProfileEntry {
        ts_millis,
        ts_secs: ts_millis.div_euclid(1_000),
        ns: "shop.orders".into(),
        op: "query".into(),
        millis: 7,
        plan_summary: "COLLSCAN".into(),
        keys_examined: 0,
        docs_examined: 1_000,
        returned: 1,
        has_sort_stage: false,
        app_name: None,
        command: doc! { "find": "orders", "filter": { "a": 1 } },
    }
}

/// A server whose clock reads `server_now` and whose profile holds `entries`.
fn store_at(entries: Vec<ProfileEntry>, server_now: i64) -> InMemoryStore {
    let ns = Namespace {
        db: "shop".into(),
        collection: "orders".into(),
    };
    InMemoryStore::new()
        .with_docs(&ns, vec![doc! { "a": 1 }])
        .with_profiling_level("shop", 1)
        .with_profile("shop", entries)
        .with_profile_server_now("shop", server_now)
}

fn store(entries: Vec<ProfileEntry>) -> InMemoryStore {
    store_at(entries, SERVER_NOW)
}

fn history_with_mark(ts_millis: i64) -> Arc<HistoryDb> {
    let history = HistoryDb::open_in_memory().unwrap();
    history
        .record_page(
            &[],
            Some(&ProfileMark {
                connection: CONN,
                host: "localhost:27017".into(),
                db: "shop".into(),
                ts_millis,
            }),
        )
        .unwrap();
    Arc::new(history)
}

/// One poll with a fresh poller, as after a restart (the mark comes from the history).
async fn poll(store: &InMemoryStore, history: &Arc<HistoryDb>) -> ProfilerStatus {
    let cfg = SamplerConfig {
        command_timeout: Duration::from_secs(5),
        ..SamplerConfig::default()
    };
    let mut marks = Marks::default();
    match poll_once(store, history, CONN, &cfg, NOW_SECS, &mut marks).await {
        Ok(poll) => poll.status(),
        Err(_) => panic!("the poll failed"),
    }
}

fn executions(history: &HistoryDb) -> u64 {
    history
        .shapes_for(CONN, "shop", "orders", ChartRange::Day, NOW_SECS + 7_200)
        .unwrap()
        .iter()
        .map(|s| s.executions)
        .sum()
}

const FLOOR: i64 = 1_000;

#[test]
fn a_mark_within_tolerance_is_kept() {
    let now = 1_700_000_000_000;
    let edge = now + MARK_AHEAD_TOLERANCE_MS;
    assert_eq!(clamp_ahead(now, Some(now), FLOOR), (now, false));
    assert_eq!(clamp_ahead(edge, Some(now), FLOOR), (edge, false));
    let past = now - HOUR_MS;
    assert_eq!(clamp_ahead(past, Some(now), FLOOR), (past, false));
}

#[test]
fn a_mark_far_ahead_is_moved_to_server_now() {
    let now = 1_700_000_000_000;
    let past_edge = now + MARK_AHEAD_TOLERANCE_MS + 1;
    assert_eq!(clamp_ahead(past_edge, Some(now), FLOOR), (now, true));
    assert_eq!(clamp_ahead(now + HOUR_MS, Some(now), FLOOR), (now, true));
}

#[test]
fn no_server_time_keeps_the_mark() {
    assert_eq!(clamp_ahead(i64::MAX, None, FLOOR), (i64::MAX, false));
}

#[test]
fn a_bogus_past_server_time_never_drops_the_mark_below_the_floor() {
    let floor = 1_699_000_000_000;
    let mark = 1_700_000_000_000;
    assert_eq!(clamp_ahead(mark, Some(0), floor), (floor, true));
    // Already at the floor: nothing to move, so no loop.
    assert_eq!(clamp_ahead(floor, Some(0), floor), (floor, false));
}

#[tokio::test]
async fn a_clamped_mark_reports_no_missed_entries() {
    // The mark is an hour ahead; the real entries are just after the server's clock.
    let history = history_with_mark(SERVER_NOW + HOUR_MS);
    let store = store(vec![entry(SERVER_NOW + 5_000), entry(SERVER_NOW + 6_000)]);
    let status = poll(&store, &history).await;
    assert_eq!(status, ProfilerStatus::Reading { databases: 1 });
    assert_eq!(executions(&history), 2, "the entries the mark hid are read");
}

#[tokio::test]
async fn an_entry_counted_before_the_clock_went_back_is_not_counted_again() {
    let later = SERVER_NOW + HOUR_MS;
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    // The clock was an hour ahead: the entry was stamped then and counted.
    poll(&store_at(vec![entry(later)], later), &history).await;
    assert_eq!(executions(&history), 1);
    // The clock is set back; the same entry is still in the profile. Polled twice (a restart
    // each time): it must stay counted once.
    let reset = store_at(vec![entry(later)], SERVER_NOW);
    poll(&reset, &history).await;
    poll(&reset, &history).await;
    assert_eq!(executions(&history), 1);
}

#[tokio::test]
async fn a_server_time_of_zero_or_the_far_past_recounts_nothing_and_ends() {
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    let entries = vec![entry(SERVER_NOW - 5_000), entry(SERVER_NOW - 4_000)];
    poll(&store_at(entries.clone(), SERVER_NOW), &history).await;
    assert_eq!(executions(&history), 2);
    for broken_clock in [0, 86_400_000, 1_000_000_000_000] {
        let broken = store_at(entries.clone(), broken_clock);
        for _ in 0..2 {
            let status = poll(&broken, &history).await;
            assert_eq!(status, ProfilerStatus::Reading { databases: 1 });
        }
        assert_eq!(executions(&history), 2, "clock {broken_clock}");
    }
}

#[tokio::test]
async fn an_unparseable_future_document_never_moves_the_mark_ahead_of_the_clock() {
    let future = SERVER_NOW + HOUR_MS;
    let first = entry(SERVER_NOW - 5_000);
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    let page = store(vec![first.clone()]).with_profile_unparseable("shop", vec![future]);
    poll(&page, &history).await;
    assert_eq!(executions(&history), 1);
    // The position is the newest entry that could be read, not the stamp of the document that
    // could not.
    let stored = history.profile_mark(CONN, "shop").unwrap();
    assert_eq!(stored, Some(SERVER_NOW - 5_000));
    // After a restart (a fresh poller) a later real entry is still read, not skipped.
    let later = vec![first, entry(SERVER_NOW - 2_000)];
    let page = store(later).with_profile_unparseable("shop", vec![future]);
    poll(&page, &history).await;
    assert_eq!(executions(&history), 2);
    assert_eq!(
        history.profile_mark(CONN, "shop").unwrap(),
        Some(SERVER_NOW - 2_000)
    );
}

#[tokio::test]
async fn an_unparseable_future_document_alone_leaves_the_mark_where_it_was() {
    let history = history_with_mark(SERVER_NOW - HOUR_MS);
    let page = store(Vec::new()).with_profile_unparseable("shop", vec![SERVER_NOW + HOUR_MS]);
    poll(&page, &history).await;
    assert_eq!(
        history.profile_mark(CONN, "shop").unwrap(),
        Some(SERVER_NOW - HOUR_MS)
    );
}

#[tokio::test]
async fn a_mark_in_the_past_is_left_alone() {
    let past = SERVER_NOW - HOUR_MS;
    let history = history_with_mark(past);
    poll(&store(Vec::new()), &history).await;
    assert_eq!(history.profile_mark(CONN, "shop").unwrap(), Some(past));
}
