//! "Last used" and "tracked since" come from real timestamps, not from the chart's buckets.

use super::indexes::series_for;
use super::*;
use crate::clock;
use crate::history::{ChartRange, HistoryDb, Sample, SeriesKey};
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
        let series = series_for(Some(history.clone()), session, &ns, range, &listed)
            .await
            .unwrap();
        assert_eq!(series.len(), 1);
        seen.push((series[0].last_used_at_secs, series[0].tracked_since_secs));
    }

    assert!(seen.iter().all(|f| *f == seen[0]), "{seen:?}");
    assert_eq!(seen[0], (Some(used), Some(first)));
}
