//! A deleted connection's history stays deleted: writes queued before the delete are refused.

use super::db::*;
use super::series::Sample;
use uuid::Uuid;

const NOW: i64 = 1_800_000_000;

fn key_for(connection: Uuid, host: &str, hash: &str) -> SeriesKey {
    SeriesKey {
        connection,
        host: host.into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "by_sku".into(),
        definition_hash: hash.into(),
    }
}

fn s(at_secs: i64, ops: i64) -> Sample {
    Sample {
        at_secs,
        ops,
        since_ms: 1_000,
    }
}

#[test]
fn a_write_queued_after_a_purge_does_not_bring_the_series_back() {
    let db = HistoryDb::open_in_memory().unwrap();
    let gone = key_for(Uuid::from_u128(1), "h", "x");
    let kept = key_for(Uuid::from_u128(2), "h", "x");
    db.record(&gone, s(NOW - 120, 0)).unwrap();
    db.purge_connection(gone.connection).unwrap();
    // The sampler's last batch was already queued when the connection was deleted.
    db.record_many(&[
        (gone.clone(), s(NOW - 60, 1)),
        (kept.clone(), s(NOW - 60, 1)),
    ])
    .unwrap();
    assert!(
        db.series_for(gone.connection, "shop", "orders")
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.latest(&gone).unwrap(), None);
    // Other connections in the same batch are stored.
    assert!(db.latest(&kept).unwrap().is_some());
}

#[test]
fn marking_a_connection_purged_refuses_writes_before_the_delete_runs() {
    let db = HistoryDb::open_in_memory().unwrap();
    let gone = key_for(Uuid::from_u128(1), "h", "x");
    db.record(&gone, s(NOW - 120, 0)).unwrap();
    db.mark_purged(gone.connection);
    db.record(&gone, s(NOW - 60, 1)).unwrap();
    // Not purged yet, but the late write did not add to the series either.
    assert_eq!(db.latest(&gone).unwrap().map(|l| l.ops), Some(0));
    db.purge_connection(gone.connection).unwrap();
    assert!(
        db.series_for(gone.connection, "shop", "orders")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_new_sampler_for_a_purged_id_records_again() {
    let db = HistoryDb::open_in_memory().unwrap();
    let id = key_for(Uuid::from_u128(1), "h", "x");
    db.record(&id, s(NOW - 120, 0)).unwrap();
    db.purge_connection(id.connection).unwrap();
    db.allow_recording(id.connection);
    db.record(&id, s(NOW - 60, 1)).unwrap();
    assert_eq!(
        db.series_for(id.connection, "shop", "orders")
            .unwrap()
            .len(),
        1
    );
}
