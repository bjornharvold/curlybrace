//! Many connections, one history: the hourly thinning runs once per hour, not once per sampler.

use super::*;

const OTHER: Uuid = Uuid::from_u128(8);

#[tokio::test(start_paused = true)]
async fn two_samplers_on_one_history_thin_once_an_hour() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let db = history();
    let (first, mut status, _cfg) = start_with_clock(store.clone(), db.clone());
    let (_second_cfg, cfg_rx) = watch::channel(SamplerConfig::default());
    let (status_tx, _second_status) = unbounded_channel();
    let second = tokio::spawn(run(
        store,
        db.clone(),
        OTHER,
        cfg_rx,
        status_tx,
        controlled_clock(),
    ));
    assert_eq!(status.recv().await, Some(SamplerStatus::Running));
    tokio::time::sleep(secs(1)).await;
    assert_eq!(db.thin_runs_for_tests(), 1, "both start, one thins");

    tokio::time::sleep(secs(3_600)).await;
    assert_eq!(db.thin_runs_for_tests(), 2, "one more for the next hour");
    first.abort();
    second.abort();
}
