//! A clock that ran ahead and was corrected must not cost real history: the roll-up recovers
//! and no raw row is deleted without having been folded into a rolled row.

use super::db::*;
use super::retention::ChartRange;
use super::series::{DEFAULT_MAX_CONTINUOUS_SECS, Sample};
use uuid::Uuid;

const DAY: i64 = 86_400;
const NOW: i64 = 1_800_000_000 / DAY * DAY;
const MAX: i64 = DEFAULT_MAX_CONTINUOUS_SECS;
const MINUTES_PER_DAY: i64 = 24 * 60;

fn key() -> SeriesKey {
    SeriesKey {
        connection: Uuid::from_u128(1),
        host: "node-a:27017".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "by_sku".into(),
        definition_hash: "h1".into(),
    }
}

fn sample(at_secs: i64, ops: i64) -> Sample {
    Sample {
        at_secs,
        ops,
        since_ms: 1_000,
    }
}

/// One sample a minute from `clock_from`, thinning hourly, as the sampler does.
fn run(db: &HistoryDb, clock_from: i64, minutes: i64, ops: &mut i64) {
    for m in 0..minutes {
        let t = clock_from + 60 * m;
        db.record(&key(), sample(t, *ops)).unwrap();
        *ops += 1;
        if m % 60 == 0 {
            db.thin(t, MAX).unwrap();
        }
    }
}

fn week_uses(db: &HistoryDb, end: i64) -> i64 {
    db.usage(&key(), ChartRange::Week, end, MAX)
        .unwrap()
        .iter()
        .map(|b| b.uses)
        .sum()
}

fn rolled_through(db: &HistoryDb) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row("SELECT rolled_through FROM series", [], |r| r.get(0))
        .unwrap()
}

fn ahead_then_corrected(ahead_days: i64) -> (HistoryDb, i64) {
    let db = HistoryDb::open_in_memory().unwrap();
    let mut ops = 0;
    let real0 = NOW - 20 * DAY;
    run(&db, real0, 3 * MINUTES_PER_DAY, &mut ops);
    run(
        &db,
        real0 + (3 + ahead_days) * DAY,
        3 * MINUTES_PER_DAY,
        &mut ops,
    );
    let corrected = real0 + 6 * DAY;
    run(&db, corrected, 10 * MINUTES_PER_DAY, &mut ops);
    (db, corrected + 10 * DAY)
}

#[test]
fn a_clock_30_days_ahead_for_3_days_then_corrected_keeps_the_real_history() {
    let (db, end) = ahead_then_corrected(30);

    let uses = week_uses(&db, end);

    assert!(uses > 6 * MINUTES_PER_DAY, "week uses {uses}");
    assert!(
        rolled_through(&db) <= end,
        "the seam recovered: {}d ahead",
        (rolled_through(&db) - end) / DAY
    );
}

#[test]
fn a_clock_one_day_ahead_for_3_days_then_corrected_keeps_the_real_history() {
    let (db, end) = ahead_then_corrected(1);

    let uses = week_uses(&db, end);

    assert!(uses > 6 * MINUTES_PER_DAY, "week uses {uses}");
}
