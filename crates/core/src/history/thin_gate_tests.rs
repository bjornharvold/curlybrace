//! Thinning is global, so the samplers of every connection share one schedule.

use super::db::*;
use super::series::DEFAULT_MAX_CONTINUOUS_SECS;
use std::time::Duration;
use tokio::time::Instant;

const HOUR: Duration = Duration::from_secs(3_600);
const NOW: i64 = 1_800_000_000;

fn thin_at(db: &HistoryDb, at: Instant) -> crate::error::Result<bool> {
    db.thin_if_due(at, HOUR, NOW, DEFAULT_MAX_CONTINUOUS_SECS)
}

#[test]
fn a_second_caller_within_the_period_does_not_thin_again() {
    let db = HistoryDb::open_in_memory().unwrap();
    let start = Instant::now();
    assert_eq!(thin_at(&db, start), Ok(true));
    assert_eq!(thin_at(&db, start + Duration::from_secs(600)), Ok(false));
    assert_eq!(db.thin_runs_for_tests(), 1);
    assert_eq!(
        thin_at(&db, start + HOUR),
        Ok(true),
        "due again after the period"
    );
    assert_eq!(db.thin_runs_for_tests(), 2);
}

#[test]
fn a_failed_thin_is_retried_at_once() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.break_writes_for_tests();
    let start = Instant::now();
    assert!(thin_at(&db, start).is_err());
    assert!(
        thin_at(&db, start + Duration::from_secs(1)).is_err(),
        "the failure did not use up the period"
    );
}
