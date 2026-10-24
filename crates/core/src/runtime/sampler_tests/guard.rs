//! The guards of a pass against a dead connection and a skewed clock.

use super::*;
use crate::runtime::sampler::{MAX_CLOCK_AHEAD_SECS, MAX_CONSECUTIVE_TIMEOUTS};

/// A clock that reads `first` once and `later` after that, like one that was stepped back (by NTP,
/// or a person) while a pass was running.
fn stepped_clock(first: i64, later: i64) -> impl Fn() -> i64 + Send + Sync + 'static {
    let reads = AtomicUsize::new(0);
    move || match reads.fetch_add(1, Ordering::SeqCst) {
        0 => first,
        _ => later,
    }
}

#[tokio::test]
async fn a_sample_stamped_by_a_clock_that_was_far_ahead_is_not_recorded() {
    let store = store_with(&[("shop", "orders")]);
    let db = history();
    let clock = stepped_clock(NOW + 10 * 86_400, NOW);

    let recorded = sample_once(&store, &db, CONN, &SamplerConfig::default(), &clock)
        .await
        .unwrap();

    assert_eq!(recorded, 0, "skipped, not failed");
    assert_eq!(
        db.latest(&series_of("shop", "orders", "h1:27017")).unwrap(),
        None
    );
}

#[tokio::test]
async fn a_small_clock_skew_is_tolerated() {
    let store = store_with(&[("shop", "orders")]);
    let db = history();
    let clock = stepped_clock(NOW + MAX_CLOCK_AHEAD_SECS, NOW);

    let recorded = sample_once(&store, &db, CONN, &SamplerConfig::default(), &clock)
        .await
        .unwrap();

    assert_eq!(recorded, 1);
}

fn many_collections(count: usize) -> InMemoryStore {
    let names: Vec<String> = (0..count).map(|i| format!("c{i}")).collect();
    let refs: Vec<(&str, &str)> = names.iter().map(|c| ("shop", c.as_str())).collect();
    store_with(&refs)
}

#[tokio::test(start_paused = true)]
async fn collections_that_never_answer_end_the_pass_after_a_few_timeouts() {
    let stalled = ProbeStore::new(many_collections(200)).delayed(secs(86_400));
    let store = Arc::new(stalled);
    let cfg = SamplerConfig {
        max_concurrency: 1,
        command_timeout: secs(10),
        ..SamplerConfig::default()
    };
    let began = Instant::now();

    let (handle, mut status, _cfg) = start(store.clone(), history(), cfg);

    let first = tokio::time::timeout(
        secs(10 * MAX_CONSECUTIVE_TIMEOUTS as u64 + 5),
        status.recv(),
    )
    .await;
    assert_eq!(first, Ok(Some(SamplerStatus::Failing)));
    assert_eq!(store.called().len(), MAX_CONSECUTIVE_TIMEOUTS);
    assert!(began.elapsed() < secs(10 * MAX_CONSECUTIVE_TIMEOUTS as u64 + 5));
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn calls_slower_than_the_old_fixed_bound_but_within_the_configured_one_are_not_timeouts() {
    // Calls slower than the old fixed bound but under the configured one are not timeouts.
    let store = ProbeStore::new(many_collections(8)).delayed(secs(10));
    let cfg = SamplerConfig {
        max_concurrency: 1,
        command_timeout: secs(20),
        ..SamplerConfig::default()
    };
    let recorded = sample(&store, &history(), &cfg).await.unwrap();
    assert_eq!(recorded, 8, "every call answered within the timeout");
}

#[tokio::test(start_paused = true)]
async fn the_configured_command_timeout_bounds_a_call_not_the_fixed_default() {
    let slow = ProbeStore::new(store_with(&[("shop", "orders")])).delayed(secs(45));
    let cfg = SamplerConfig {
        command_timeout: secs(60),
        ..SamplerConfig::default()
    };
    assert_eq!(sample(&slow, &history(), &cfg).await.unwrap(), 1);
}
