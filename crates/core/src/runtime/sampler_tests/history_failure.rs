//! A history failure ends the pass at once; the rest of the collections are not read for nothing.

use super::*;

fn many_collections(count: usize) -> InMemoryStore {
    let names: Vec<String> = (0..count).map(|i| format!("c{i}")).collect();
    let refs: Vec<(&str, &str)> = names.iter().map(|c| ("shop", c.as_str())).collect();
    store_with(&refs)
}

#[tokio::test]
async fn the_first_history_failure_stops_reading_index_stats() {
    let store = ProbeStore::new(many_collections(20));
    let db = history();
    db.break_writes_for_tests();
    let cfg = SamplerConfig {
        max_concurrency: 1,
        ..SamplerConfig::default()
    };

    let result = sample_once(&store, &db, CONN, &cfg, &|| NOW).await;

    assert!(matches!(result, Err(CoreError::Database(_))), "{result:?}");
    assert_eq!(
        store.called().len(),
        1,
        "only the first collection was read"
    );
}

#[tokio::test]
async fn a_wide_pass_reads_no_more_than_the_calls_in_flight_after_a_history_failure() {
    let store = ProbeStore::new(many_collections(40));
    let db = history();
    db.break_writes_for_tests();
    let cfg = SamplerConfig {
        max_concurrency: 4,
        ..SamplerConfig::default()
    };

    let result = sample_once(&store, &db, CONN, &cfg, &|| NOW).await;

    assert!(result.is_err());
    assert!(store.called().len() <= 8, "{} calls", store.called().len());
}
