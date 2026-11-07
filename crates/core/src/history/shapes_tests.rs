//! Recording and reading query shapes: aggregation, sources, purge, ranges and gaps.

use super::db::HistoryDb;
use super::retention::ChartRange;
use super::shapes::{ShapeKey, ShapeObservation, ShapeSource, is_miss};
use crate::explain::VerdictKind;
use crate::store::Query;
use bson::doc;
use uuid::Uuid;

pub(super) const NOW: i64 = 1_800_000_000;

pub(super) fn conn_id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

pub(super) fn key(conn: u128, source: ShapeSource, hash: &str) -> ShapeKey {
    ShapeKey {
        connection: conn_id(conn),
        host: match source {
            ShapeSource::App => "node-a:27017".into(),
            ShapeSource::Typed => String::new(),
        },
        db: "shop".into(),
        collection: "orders".into(),
        hash: hash.into(),
        source,
    }
}

pub(super) fn obs(
    key: ShapeKey,
    at_secs: i64,
    executions: u64,
    misses: u64,
    (total_millis, max_millis): (u64, u64),
    worst: VerdictKind,
) -> ShapeObservation {
    ShapeObservation {
        key,
        shape_json: r#"{"filter":{"a":"$eq"}}"#.into(),
        at_secs,
        executions,
        misses,
        total_millis,
        max_millis,
        worst,
        gap_secs: 0,
    }
}

pub(super) fn count(db: &HistoryDb, sql: &str) -> i64 {
    db.conn
        .lock()
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

fn app(hash: &str) -> ShapeKey {
    key(1, ShapeSource::App, hash)
}

fn read(db: &HistoryDb, range: ChartRange) -> Vec<super::shapes::ShapeSummary> {
    db.shapes_for(conn_id(1), "shop", "orders", range, NOW)
        .unwrap()
}

#[test]
fn observations_aggregate_into_one_bucket() {
    let db = HistoryDb::open_in_memory().unwrap();
    let at = NOW - 3_600;
    db.record_shapes(&[
        obs(app("h1"), at, 2, 1, (30, 20), VerdictKind::IndexUsed),
        obs(
            app("h1"),
            at + 100,
            3,
            3,
            (700, 500),
            VerdictKind::CollectionScan,
        ),
        obs(app("h1"), at + 199, 1, 0, (5, 5), VerdictKind::IndexUsed),
    ])
    .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_bucket"), 1);
    let got = read(&db, ChartRange::Day);
    assert_eq!(got.len(), 1);
    let s = &got[0];
    assert_eq!(
        (s.executions, s.misses, s.total_millis, s.max_millis),
        (6, 4, 735, 500)
    );
    assert_eq!(s.worst, VerdictKind::CollectionScan);
    assert_eq!(s.buckets.len(), 1);
    assert_eq!(s.buckets[0].start_secs, at.div_euclid(600) * 600);
    assert_eq!(s.buckets[0].width_secs, 600);
}

#[test]
fn a_less_severe_later_observation_keeps_the_worst() {
    let db = HistoryDb::open_in_memory().unwrap();
    let at = NOW - 3_600;
    db.record_shapes(&[obs(app("h"), at, 1, 1, (1, 1), VerdictKind::SortNotCovered)])
        .unwrap();
    db.record_shapes(&[obs(app("h"), at, 1, 0, (1, 1), VerdictKind::NoFilter)])
        .unwrap();
    assert_eq!(
        read(&db, ChartRange::Day)[0].worst,
        VerdictKind::SortNotCovered
    );
}

#[test]
fn same_shape_different_source_are_separate_rows() {
    let db = HistoryDb::open_in_memory().unwrap();
    let at = NOW - 600;
    db.record_shapes(&[
        obs(
            key(1, ShapeSource::App, "h"),
            at,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ),
        obs(
            key(1, ShapeSource::Typed, "h"),
            at,
            1,
            1,
            (1, 1),
            VerdictKind::CollectionScan,
        ),
    ])
    .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 2);
    let got = read(&db, ChartRange::Day);
    assert_eq!(got.len(), 2);
    let sources: Vec<_> = got.iter().map(|s| s.source).collect();
    assert!(sources.contains(&ShapeSource::App) && sources.contains(&ShapeSource::Typed));
}

#[test]
fn hosts_are_summed_into_one_summary() {
    let db = HistoryDb::open_in_memory().unwrap();
    let mut other = app("h");
    other.host = "node-b:27017".into();
    let at = NOW - 600;
    db.record_shapes(&[
        obs(app("h"), at, 2, 0, (2, 1), VerdictKind::IndexUsed),
        obs(other, at, 3, 0, (3, 2), VerdictKind::IndexUsed),
    ])
    .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 2);
    let got = read(&db, ChartRange::Day);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].executions, got[0].buckets.len()), (5, 1));
    assert_eq!(got[0].buckets[0].executions, 5);
}

#[test]
fn summaries_are_ordered_by_severity_then_misses_then_max_millis() {
    let db = HistoryDb::open_in_memory().unwrap();
    let at = NOW - 600;
    let scan = VerdictKind::CollectionScan;
    db.record_shapes(&[
        obs(app("fine"), at, 9, 0, (9, 99), VerdictKind::IndexUsed),
        obs(app("scan-few"), at, 9, 1, (9, 5), scan),
        obs(app("scan-many"), at, 9, 5, (9, 5), scan),
        obs(app("scan-slow"), at, 9, 5, (9, 50), scan),
        obs(app("sort"), at, 9, 9, (9, 5), VerdictKind::SortNotCovered),
    ])
    .unwrap();
    let got = read(&db, ChartRange::Day);
    let order: Vec<_> = got.iter().map(|s| s.hash.as_str()).collect();
    assert_eq!(
        order,
        ["scan-slow", "scan-many", "scan-few", "sort", "fine"]
    );
}

fn dump_all_text(db: &HistoryDb) -> String {
    let conn = db.conn.lock().unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut dump = String::new();
    for table in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{table}\"")).unwrap();
        let cols = stmt.column_count();
        let mut rows = stmt.query([]).unwrap();
        while let Some(row) = rows.next().unwrap() {
            for i in 0..cols {
                let v: rusqlite::types::Value = row.get(i).unwrap();
                use rusqlite::types::Value;
                match v {
                    Value::Text(t) => dump.push_str(&t),
                    Value::Integer(n) => dump.push_str(&n.to_string()),
                    Value::Real(n) => dump.push_str(&n.to_string()),
                    Value::Blob(b) => dump.push_str(&String::from_utf8_lossy(&b)),
                    Value::Null => {}
                }
                dump.push('\n');
            }
        }
    }
    dump
}

#[test]
fn literal_values_are_not_stored() {
    let db = HistoryDb::open_in_memory().unwrap();
    let shape = crate::shape::from_query(&Query {
        filter: doc! { "email": "SECRET-7f3a@example.com", "age": { "$gt": 918273645_i64 } },
        ..Query::default()
    });
    let k = ShapeKey {
        hash: shape.hash(),
        ..app("x")
    };
    let mut o = obs(k, NOW - 600, 1, 1, (1, 1), VerdictKind::CollectionScan);
    o.shape_json = shape.json();
    db.record_shapes(&[o]).unwrap();
    db.record_gap(conn_id(1), "node-a:27017", "shop", NOW - 600, 30)
        .unwrap();
    let dump = dump_all_text(&db);
    assert!(dump.contains("email"), "field names are kept: {dump}");
    for leak in ["SECRET", "7f3a", "example.com", "918273645"] {
        assert!(!dump.contains(leak), "{leak} leaked: {dump}");
    }
}

#[test]
fn purged_connection_is_not_resurrected_by_a_late_record() {
    let db = HistoryDb::open_in_memory().unwrap();
    let ok = VerdictKind::IndexUsed;
    db.mark_purged(conn_id(1));
    db.record_shapes(&[
        obs(key(1, ShapeSource::App, "h"), NOW, 1, 0, (1, 1), ok),
        obs(key(2, ShapeSource::App, "h"), NOW, 1, 0, (1, 1), ok),
    ])
    .unwrap();
    db.record_gap(conn_id(1), "h", "shop", NOW, 60).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_gap"), 0);
    db.allow_recording(conn_id(1));
    db.record_shapes(&[obs(key(1, ShapeSource::App, "h"), NOW, 1, 0, (1, 1), ok)])
        .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 2);
}

#[test]
fn purge_connection_deletes_shape_rows_including_typed() {
    let db = HistoryDb::open_in_memory().unwrap();
    let ok = VerdictKind::IndexUsed;
    db.record_shapes(&[
        obs(key(1, ShapeSource::App, "a"), NOW, 1, 0, (1, 1), ok),
        obs(key(1, ShapeSource::Typed, "t"), NOW, 1, 0, (1, 1), ok),
        obs(key(2, ShapeSource::Typed, "t"), NOW, 1, 0, (1, 1), ok),
    ])
    .unwrap();
    db.record_gap(conn_id(1), "h", "shop", NOW, 60).unwrap();
    db.record_gap(conn_id(2), "h", "shop", NOW, 60).unwrap();
    db.purge_connection(conn_id(1)).unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_bucket"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape_gap"), 1);
    assert!(read(&db, ChartRange::Day).is_empty());
    let other = db
        .shapes_for(conn_id(2), "shop", "orders", ChartRange::Day, NOW)
        .unwrap();
    assert_eq!(other.len(), 1);
}

#[test]
fn shapes_for_respects_range_and_reports_gaps() {
    let db = HistoryDb::open_in_memory().unwrap();
    let day = 86_400;
    let ok = VerdictKind::IndexUsed;
    db.record_shapes(&[
        obs(
            app("h"),
            NOW - 1_800,
            1,
            1,
            (10, 10),
            VerdictKind::CollectionScan,
        ),
        obs(app("h"), NOW - 3 * day, 10, 0, (100, 40), ok),
        obs(app("h"), NOW - 20 * day, 100, 0, (1_000, 90), ok),
        obs(app("h"), NOW - 40 * day, 1_000, 0, (1, 1), ok),
    ])
    .unwrap();
    let gap_at = NOW - 1_800;
    let host = "node-a:27017";
    db.record_gap(conn_id(1), host, "shop", gap_at, 120)
        .unwrap();
    // A gap with no execution in its bucket still shows.
    db.record_gap(conn_id(1), host, "shop", NOW - 7_200, 60)
        .unwrap();
    // Another database's gap does not.
    db.record_gap(conn_id(1), host, "other", gap_at, 500)
        .unwrap();

    let d = &read(&db, ChartRange::Day)[0];
    assert_eq!((d.executions, d.max_millis), (1, 10));
    assert!(d.buckets.iter().all(|b| b.width_secs == 600));
    let with_exec = d.buckets.iter().find(|b| b.executions == 1).unwrap();
    assert_eq!(with_exec.gap_secs, 120);
    assert_eq!(with_exec.start_secs, gap_at.div_euclid(600) * 600);
    let gap_only = d.buckets.iter().find(|b| b.executions == 0).unwrap();
    assert_eq!(gap_only.gap_secs, 60);

    let w = &read(&db, ChartRange::Week)[0];
    assert_eq!(w.executions, 11);
    assert!(w.buckets.iter().all(|b| b.width_secs == 3_600));
    let m = &read(&db, ChartRange::Month)[0];
    assert_eq!((m.executions, m.max_millis), (111, 90));
    assert!(m.buckets.iter().all(|b| b.width_secs == 86_400));
    assert_eq!(m.worst, VerdictKind::CollectionScan);
}

#[test]
fn unknown_worst_code_reads_back_neutral() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_shapes(&[obs(
        app("h"),
        NOW - 600,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )])
    .unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE shape_bucket SET worst = 'from_the_future'", [])
        .unwrap();
    assert_eq!(read(&db, ChartRange::Day)[0].worst, VerdictKind::NoFilter);
    // A later known observation still wins over the unknown code.
    let slow = VerdictKind::InefficientIndex;
    db.record_shapes(&[obs(app("h"), NOW - 600, 1, 1, (1, 1), slow)])
        .unwrap();
    assert_eq!(read(&db, ChartRange::Day)[0].worst, slow);
}

#[test]
fn is_miss_table() {
    use VerdictKind::*;
    for (kind, miss) in [
        (CollectionScan, true),
        (SortNotCovered, true),
        (InefficientIndex, true),
        (IndexUsed, false),
        (CoveredByIndex, false),
        (NoFilter, false),
        (EmptyCollection, false),
    ] {
        assert_eq!(is_miss(kind), miss, "{kind:?}");
    }
    assert!(
        VerdictKind::ALL
            .iter()
            .all(|k| is_miss(*k) == (k.severity() >= 2))
    );
}

#[test]
fn source_text_round_trips() {
    for s in [ShapeSource::App, ShapeSource::Typed] {
        assert_eq!(s.as_str().parse::<ShapeSource>(), Ok(s));
    }
    assert!("other".parse::<ShapeSource>().is_err());
}

#[test]
fn purge_is_all_or_nothing() {
    let db = HistoryDb::open_in_memory().unwrap();
    let usage = super::db::SeriesKey {
        connection: conn_id(1),
        host: "h".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "i".into(),
        definition_hash: "x".into(),
    };
    let sample = super::series::Sample {
        at_secs: NOW,
        ops: 1,
        since_ms: 1,
    };
    db.record(&usage, sample).unwrap();
    db.record_shapes(&[obs(app("a"), NOW, 1, 0, (1, 1), VerdictKind::IndexUsed)])
        .unwrap();
    db.record_gap(conn_id(1), "h", "shop", NOW, 60).unwrap();
    let tables = ["series", "shape", "shape_bucket", "shape_gap"];
    let counts = |db: &HistoryDb| -> Vec<i64> {
        tables
            .iter()
            .map(|t| count(db, &format!("SELECT COUNT(*) FROM {t}")))
            .collect()
    };
    // The gap delete is the last statement: make it fail.
    db.conn
        .lock()
        .unwrap()
        .execute("ALTER TABLE shape_gap RENAME TO shape_gap_away", [])
        .unwrap();
    assert!(db.purge_connection(conn_id(1)).is_err());
    db.conn
        .lock()
        .unwrap()
        .execute("ALTER TABLE shape_gap_away RENAME TO shape_gap", [])
        .unwrap();
    assert_eq!(counts(&db), [1, 1, 1, 1], "nothing was deleted");
    db.purge_connection(conn_id(1)).unwrap();
    assert_eq!(counts(&db), [0, 0, 0, 0]);
}

#[test]
fn gap_is_the_worst_host_not_the_sum_of_hosts() {
    let db = HistoryDb::open_in_memory().unwrap();
    let at = NOW - 3_600;
    let mut other = app("h");
    other.host = "node-b:27017".into();
    let seen = ShapeObservation {
        gap_secs: 30,
        ..obs(app("h"), at, 1, 0, (1, 1), VerdictKind::IndexUsed)
    };
    db.record_shapes(&[seen, obs(other, at, 1, 0, (1, 1), VerdictKind::IndexUsed)])
        .unwrap();
    // Host a: 30 (observation) + 60 (recorded) = 90. Host b: 60. The bucket gets 90, not 150.
    db.record_gap(conn_id(1), "node-a:27017", "shop", at, 60)
        .unwrap();
    db.record_gap(conn_id(1), "node-b:27017", "shop", at, 60)
        .unwrap();
    let got = read(&db, ChartRange::Day);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].buckets.len(), 1);
    assert_eq!(got[0].buckets[0].gap_secs, 90);
    // Never more than the bucket is wide.
    db.record_gap(conn_id(1), "node-a:27017", "shop", at, 5_000)
        .unwrap();
    assert_eq!(read(&db, ChartRange::Day)[0].buckets[0].gap_secs, 600);
}

#[test]
fn typed_shapes_always_have_the_empty_host() {
    let db = HistoryDb::open_in_memory().unwrap();
    let mut k = key(1, ShapeSource::Typed, "t");
    k.host = "node-a:27017".into();
    db.record_shapes(&[obs(
        k.clone(),
        NOW - 600,
        1,
        0,
        (1, 1),
        VerdictKind::IndexUsed,
    )])
    .unwrap();
    k.host = String::new();
    db.record_shapes(&[obs(k, NOW - 600, 1, 0, (1, 1), VerdictKind::IndexUsed)])
        .unwrap();
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape WHERE host = ''"), 1);
    assert_eq!(count(&db, "SELECT COUNT(*) FROM shape"), 1);
}
