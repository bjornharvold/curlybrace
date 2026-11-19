//! A watermark ahead of the server's own clock (a skewed or restored clock, a profile copied from
//! another server) would make every read return nothing for ever. The poller moves it back to
//! the server's time, taken from the `hello` of the read, and trusts nothing before that.

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

fn store(entries: Vec<ProfileEntry>) -> InMemoryStore {
    let ns = Namespace {
        db: "shop".into(),
        collection: "orders".into(),
    };
    InMemoryStore::new()
        .with_docs(&ns, vec![doc! { "a": 1 }])
        .with_profiling_level("shop", 1)
        .with_profile("shop", entries)
        .with_profile_server_now("shop", SERVER_NOW)
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
        .shapes_for(CONN, "shop", "orders", ChartRange::Day, NOW_SECS + 600)
        .unwrap()
        .iter()
        .map(|s| s.executions)
        .sum()
}

#[test]
fn a_mark_within_tolerance_is_kept() {
    let now = 1_700_000_000_000;
    let edge = now + MARK_AHEAD_TOLERANCE_MS;
    assert_eq!(clamp_ahead(now, Some(now)), (now, false));
    assert_eq!(clamp_ahead(edge, Some(now)), (edge, false));
    assert_eq!(
        clamp_ahead(now - HOUR_MS, Some(now)),
        (now - HOUR_MS, false)
    );
}

#[test]
fn a_mark_far_ahead_is_moved_to_server_now() {
    let now = 1_700_000_000_000;
    let past_edge = now + MARK_AHEAD_TOLERANCE_MS + 1;
    assert_eq!(clamp_ahead(past_edge, Some(now)), (now, true));
    assert_eq!(clamp_ahead(now + HOUR_MS, Some(now)), (now, true));
}

#[test]
fn no_server_time_keeps_the_mark() {
    assert_eq!(clamp_ahead(i64::MAX, None), (i64::MAX, false));
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
async fn the_clamp_is_persisted_with_the_next_page() {
    let history = history_with_mark(SERVER_NOW + HOUR_MS);
    poll(&store(vec![entry(SERVER_NOW + 5_000)]), &history).await;
    // The stored mark followed the page, not the old position an hour ahead.
    let stored = history.profile_mark(CONN, "shop").unwrap();
    assert_eq!(stored, Some(SERVER_NOW + 5_000));
}

#[tokio::test]
async fn the_clamp_is_persisted_even_when_nothing_was_read() {
    let history = history_with_mark(SERVER_NOW + HOUR_MS);
    poll(&store(Vec::new()), &history).await;
    let stored = history.profile_mark(CONN, "shop").unwrap();
    assert_eq!(stored, Some(SERVER_NOW));
}

#[tokio::test]
async fn a_mark_in_the_past_is_left_alone() {
    let past = SERVER_NOW - HOUR_MS;
    let history = history_with_mark(past);
    poll(&store(Vec::new()), &history).await;
    assert_eq!(history.profile_mark(CONN, "shop").unwrap(), Some(past));
}
