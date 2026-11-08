//! The size cap deletes raw usage samples in insertion order, so "which set goes first" must
//! compare against the row that delete takes next, not against the oldest timestamp.

use super::db::{HistoryDb, SeriesKey};
use super::series::Sample;
use super::shape_thin::CapStep;
use super::shapes::ShapeSource;
use super::shapes_tests::{NOW, conn_id, key, obs};
use super::thin::shape_goes_first;
use crate::explain::VerdictKind;

fn series() -> SeriesKey {
    SeriesKey {
        connection: conn_id(1),
        host: "h".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "i".into(),
        definition_hash: "x".into(),
    }
}

fn sample(at_secs: i64) -> Sample {
    Sample {
        at_secs,
        ops: 1,
        since_ms: 1,
    }
}

#[test]
fn shape_goes_first_agrees_with_the_sample_the_usage_delete_takes_next() {
    let db = HistoryDb::open_in_memory().unwrap();
    // Inserted first (so deleted first) but stamped newer than the second: a clock that stepped back.
    db.record(&series(), sample(NOW - 100)).unwrap();
    db.record(&series(), sample(NOW - 5_000)).unwrap();
    db.record_shapes(&[obs(
        key(1, ShapeSource::App, "a"),
        NOW - 3_000,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )])
    .unwrap();
    let conn = db.conn.lock().unwrap();
    // The usage delete takes the NOW - 100 row next; the shape row (NOW - 3000) is older than it.
    assert!(shape_goes_first(&conn, CapStep::OldestRaw).unwrap());
}

#[test]
fn an_empty_usage_history_never_sends_the_shapes_second() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_shapes(&[obs(
        key(1, ShapeSource::App, "a"),
        NOW - 3_000,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )])
    .unwrap();
    let conn = db.conn.lock().unwrap();
    assert!(!shape_goes_first(&conn, CapStep::OldestRaw).unwrap());
}
