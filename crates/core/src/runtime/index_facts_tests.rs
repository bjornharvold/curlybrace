//! "Last used" and "tracked since" come from real timestamps, not from the chart's buckets.

use super::indexes::series_for;
use super::*;
use crate::clock;
use crate::history::{
    ChartRange, DEFAULT_MAX_CONTINUOUS_SECS, HistoryDb, Sample, SeriesKey, max_continuous_secs,
};
use crate::indexes::KeyKind;
use crate::store::CollectionIndex;
use crate::test_support::mock_objects::mock_index_spec_named;
use uuid::Uuid;

#[tokio::test]
async fn the_facts_are_identical_for_every_range() {
    let session = Uuid::from_u128(1);
    let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
    let key = SeriesKey {
        connection: session,
        host: "h1:27017".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "a_1".into(),
        definition_hash: spec.definition_hash(),
    };
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    let now = clock::now_secs();
    let used = now - 59 * 60;
    let first = now - 3 * 3_600;
    let mut at = first;
    while at < now {
        let ops = if at >= used { 3 } else { 0 };
        history
            .record(
                &key,
                Sample {
                    at_secs: at,
                    ops,
                    since_ms: 1,
                },
            )
            .unwrap();
        at += 60;
    }
    let listed = [CollectionIndex {
        spec,
        size_bytes: 0,
    }];
    let ns = Namespace::new("shop", "orders");

    let mut seen = Vec::new();
    for range in [ChartRange::Day, ChartRange::Week, ChartRange::Month] {
        let series = series_for(
            Some(history.clone()),
            session,
            &ns,
            range,
            &listed,
            DEFAULT_MAX_CONTINUOUS_SECS,
        )
        .await
        .unwrap();
        assert_eq!(series.len(), 1);
        seen.push((series[0].last_used_at_secs, series[0].tracked_since_secs));
    }

    assert!(seen.iter().all(|f| *f == seen[0]), "{seen:?}");
    assert_eq!(seen[0], (Some(used), Some(first)));
}

#[tokio::test]
async fn samples_five_minutes_apart_are_continuous_at_a_five_minute_interval() {
    let session = Uuid::from_u128(1);
    let spec = mock_index_spec_named("a_1", &[("a", KeyKind::Asc)]);
    let key = SeriesKey {
        connection: session,
        host: "h1:27017".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "a_1".into(),
        definition_hash: spec.definition_hash(),
    };
    let history = Arc::new(HistoryDb::open_in_memory().unwrap());
    let now = clock::now_secs();
    for n in 0..6 {
        let sample = Sample {
            at_secs: now - 3_600 + n * 300,
            ops: n,
            since_ms: 1,
        };
        history.record(&key, sample).unwrap();
    }
    let listed = [CollectionIndex {
        spec,
        size_bytes: 0,
    }];
    let ns = Namespace::new("shop", "orders");
    let gap_secs = |max_continuous| {
        let history = history.clone();
        let ns = ns.clone();
        let listed = listed.clone();
        async move {
            let series = series_for(
                Some(history),
                session,
                &ns,
                ChartRange::Day,
                &listed,
                max_continuous,
            )
            .await
            .unwrap();
            series[0].buckets.iter().map(|b| b.gap_secs).sum::<i64>()
        }
    };

    let tuned = max_continuous_secs(std::time::Duration::from_secs(300));
    assert_eq!(gap_secs(tuned).await, 0, "no silence is a gap");
    assert!(
        gap_secs(DEFAULT_MAX_CONTINUOUS_SECS).await > 0,
        "the default threshold would call every pair a gap"
    );
}
