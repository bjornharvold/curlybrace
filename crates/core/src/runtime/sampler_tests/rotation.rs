//! A long healthy pass must not starve the tail, and a burst of in-flight failures is one blip.

use super::*;
use crate::runtime::sampler::MAX_CONSECUTIVE_TIMEOUTS;

fn many_collections(count: usize) -> InMemoryStore {
    let names: Vec<String> = (0..count).map(|i| format!("c{i}")).collect();
    let refs: Vec<(&str, &str)> = names.iter().map(|c| ("shop", c.as_str())).collect();
    store_with(&refs)
}

#[tokio::test(start_paused = true)]
async fn a_slow_healthy_pass_reports_running_and_covers_every_collection_across_passes() {
    // 30 collections at 100 s each, one at a time: a pass reaches about six before its deadline.
    let store = Arc::new(ProbeStore::new(many_collections(30)).delayed(secs(100)));
    let db = history();
    let cfg = SamplerConfig {
        max_concurrency: 1,
        command_timeout: secs(200),
        ..SamplerConfig::default()
    };

    let (handle, mut status, _cfg) = start(store.clone(), db.clone(), cfg);
    let first = tokio::time::timeout(secs(700), status.recv()).await;
    tokio::time::sleep(secs(6 * 700)).await;
    handle.abort();

    assert_eq!(first, Ok(Some(SamplerStatus::Running)));
    for i in 0..30 {
        let key = series_of("shop", &format!("c{i}"), "h1:27017");
        assert!(db.latest(&key).unwrap().is_some(), "c{i} was never sampled");
    }
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_failures_as_wide_as_the_concurrency_does_not_abandon_the_pass() {
    let store = ProbeStore::new(many_collections(20));
    store.fail_first_calls(4, CoreError::Unreachable);
    let cfg = SamplerConfig {
        max_concurrency: 4,
        ..SamplerConfig::default()
    };

    let recorded = sample(&store, &history(), &cfg).await.unwrap();

    assert_eq!(store.called().len(), 20, "the pass went on");
    assert_eq!(recorded, 16);
}

#[tokio::test(start_paused = true)]
async fn a_long_run_of_failures_still_abandons_the_pass() {
    let store = ProbeStore::new(many_collections(100));
    store.fail_first_calls(100, CoreError::Unreachable);
    let cfg = SamplerConfig {
        max_concurrency: 4,
        ..SamplerConfig::default()
    };

    let result = sample(&store, &history(), &cfg).await;

    assert!(result.is_err());
    let limit = MAX_CONSECUTIVE_TIMEOUTS * 4;
    assert!(
        store.called().len() <= limit + 4,
        "{}",
        store.called().len()
    );
}
