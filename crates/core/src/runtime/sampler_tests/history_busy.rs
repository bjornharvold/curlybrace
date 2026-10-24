//! A history file that is busy for a moment (another process, a backup) delays sampling; it does
//! not pause it with `HistoryUnavailable`.

use super::*;

fn locked_history(dir: &tempfile::TempDir) -> (Arc<HistoryDb>, rusqlite::Connection) {
    let path = dir.path().join("history.sqlite");
    let (db, _) = HistoryDb::open(&path).unwrap();
    db.set_busy_timeout(Duration::from_millis(20));
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    (Arc::new(db), other)
}

#[tokio::test(start_paused = true)]
async fn a_busy_history_keeps_the_sampler_running_and_the_next_pass_records() {
    let dir = tempfile::tempdir().unwrap();
    let (db, holder) = locked_history(&dir);
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));

    let (handle, mut status, _cfg) = start(store.clone(), db.clone(), SamplerConfig::default());

    assert_eq!(status.recv().await, Some(SamplerStatus::Running));
    let key = series_of("shop", "orders", "h1:27017");
    assert_eq!(db.latest(&key).unwrap(), None);
    holder.execute_batch("ROLLBACK").unwrap();
    for _ in 0..50 {
        tokio::time::sleep(secs(60)).await;
        if db.latest(&key).unwrap().is_some() {
            break;
        }
    }
    assert!(db.latest(&key).unwrap().is_some(), "the retry recorded");
    assert!(store.called().len() >= 2, "the whole pass ran again");
    assert!(!handle.is_finished());
    handle.abort();
}

#[tokio::test(start_paused = true)]
async fn a_history_error_that_is_not_busy_still_pauses() {
    let store = Arc::new(ProbeStore::new(store_with(&[("shop", "orders")])));
    let db = history();
    db.break_writes_for_tests();

    let (handle, mut status, _cfg) = start(store, db, SamplerConfig::default());

    assert_eq!(
        status.recv().await,
        Some(SamplerStatus::Paused(SamplerPause::HistoryUnavailable))
    );
    handle.abort();
}
