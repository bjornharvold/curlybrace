//! Many connections, one history: the hourly thinning runs once per hour, not once per sampler.

use super::*;

const OTHER: Uuid = Uuid::from_u128(8);

/// A second sampler for the same history, with its own connection id.
/// The status receiver must stay alive: the sampler ends when nobody listens.
fn spawn_other(
    store: Arc<ProbeStore>,
    db: Arc<HistoryDb>,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::UnboundedReceiver<SamplerStatus>,
) {
    let (_second_cfg, cfg_rx) = watch::channel(SamplerConfig::default());
    let (status_tx, second_status) = unbounded_channel();
    let task = tokio::spawn(run(store, db, OTHER, cfg_rx, status_tx, controlled_clock()));
    (task, second_status)
}

#[tokio::test(start_paused = true)]
async fn two_samplers_on_one_history_thin_once_an_hour() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let db = history();
    let (first, mut status, _cfg) = start_with_clock(store.clone(), db.clone());
    let (second, _second_status) = spawn_other(store, db.clone());
    assert_eq!(status.recv().await, Some(SamplerStatus::Running));
    tokio::time::sleep(secs(1)).await;
    assert_eq!(db.thin_runs_for_tests(), 1, "both start, one thins");

    tokio::time::sleep(secs(3_600)).await;
    assert_eq!(db.thin_runs_for_tests(), 2, "one more for the next hour");
    first.abort();
    second.abort();
}

#[tokio::test(start_paused = true)]
async fn a_sampler_that_found_the_period_claimed_thins_once_the_claimant_is_gone() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let db = history();
    let (first, mut status, _cfg) = start_with_clock(store.clone(), db.clone());
    assert_eq!(status.recv().await, Some(SamplerStatus::Running));
    assert_eq!(db.thin_runs_for_tests(), 1, "the first sampler thins");

    // The second joins half an hour in and is told the period is taken.
    tokio::time::sleep(secs(1_800)).await;
    let (second, _second_status) = spawn_other(store, db.clone());
    tokio::time::sleep(secs(1)).await;
    assert_eq!(db.thin_runs_for_tests(), 1, "claimed by the first");

    // The first goes away. The hour after its thinning is over, the second does the work within
    // one interval, not an hour after the answer it was given.
    first.abort();
    tokio::time::sleep(secs(1_920)).await;
    assert_eq!(db.thin_runs_for_tests(), 2, "the second took over");
    second.abort();
}
