//! The size cap and the shell history: the cap trims shell history first (oldest rows across all
//! connections, never below the newest [`CAP_KEEP_PER_CONNECTION`] of a connection), so large
//! scripts cannot push the usage and shape history out of the file.

use super::db::{HistoryDb, SeriesKey};
use super::series::{DEFAULT_MAX_CONTINUOUS_SECS, Sample};
use super::shapes::ShapeSource;
use super::shapes_tests::{NOW, count, key, obs};
use super::shell_history::{CAP_KEEP_PER_CONNECTION, MAX_STORED_SCRIPT_BYTES, ShellRunStatus};
use crate::explain::VerdictKind;
use uuid::Uuid;

/// Rows of usage and shape history the cap must leave alone, all recent (thinning keeps them).
fn usage_and_shapes(db: &HistoryDb) {
    let series = SeriesKey {
        connection: Uuid::from_u128(1),
        host: "h".into(),
        db: "shop".into(),
        collection: "orders".into(),
        index_name: "by_sku".into(),
        definition_hash: "h1".into(),
    };
    let samples: Vec<_> = (0..200)
        .map(|n| {
            let sample = Sample {
                at_secs: NOW - 3_600 + n * 10,
                ops: n,
                since_ms: 1_000,
            };
            (series.clone(), sample)
        })
        .collect();
    db.record_many(&samples).unwrap();
    let shapes: Vec<_> = (0..50)
        .map(|s| {
            let k = key(1, ShapeSource::App, &format!("shape-{s}"));
            obs(k, NOW - 1_800, 1, 0, (1, 1), VerdictKind::IndexUsed)
        })
        .collect();
    db.record_shapes(&shapes).unwrap();
}

/// `n` big scripts for each connection, interleaved, so the oldest rows span both.
fn big_scripts(db: &HistoryDb, connections: &[Uuid], n: usize) {
    for i in 0..n {
        for (c, connection) in connections.iter().enumerate() {
            // Distinct per row, so no run replaces the one before it.
            let script = format!("// {i} {c}\n{}", "x".repeat(MAX_STORED_SCRIPT_BYTES - 64));
            db.record_shell_run(*connection, &script, ShellRunStatus::Succeeded, NOW)
                .unwrap();
        }
    }
}

/// The counts the cap must not change: samples, rolled rows, shape buckets.
fn kept(db: &HistoryDb) -> (i64, i64, i64) {
    (
        count(db, "SELECT COUNT(*) FROM samples"),
        count(db, "SELECT COUNT(*) FROM rolled"),
        count(db, "SELECT COUNT(*) FROM shape_bucket"),
    )
}

/// The first line of each stored script of `connection` (it names the row), newest first.
fn rows_of(db: &HistoryDb, connection: Uuid) -> Vec<String> {
    db.shell_history(connection)
        .unwrap()
        .into_iter()
        .map(|e| e.script.lines().next().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn a_file_over_the_cap_because_of_shell_history_is_trimmed_without_touching_other_history() {
    let db = HistoryDb::open_in_memory().unwrap();
    usage_and_shapes(&db);
    let (a, b) = (Uuid::from_u128(10), Uuid::from_u128(11));
    big_scripts(&db, &[a, b], 100);
    let before = kept(&db);
    let (newest_a, newest_b) = (rows_of(&db, a)[0].clone(), rows_of(&db, b)[0].clone());
    // About half the scripts have to go.
    let cap = db.used_bytes().unwrap() / 2;
    let db = db.with_max_bytes(cap);

    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();

    assert!(
        db.used_bytes().unwrap() <= cap,
        "the file ends under the cap"
    );
    assert_eq!(kept(&db), before, "no usage or shape row was deleted");
    let (left_a, left_b) = (rows_of(&db, a), rows_of(&db, b));
    assert!(
        left_a.len() < 100 && left_b.len() < 100,
        "scripts were trimmed"
    );
    assert!(left_a.len() >= CAP_KEEP_PER_CONNECTION);
    assert!(left_b.len() >= CAP_KEEP_PER_CONNECTION);
    assert_eq!(left_a[0], newest_a, "the newest stay");
    assert_eq!(left_b[0], newest_b, "the newest stay");
}

#[test]
fn the_cap_keeps_the_newest_scripts_of_every_connection() {
    let db = HistoryDb::open_in_memory().unwrap();
    usage_and_shapes(&db);
    let (a, b) = (Uuid::from_u128(10), Uuid::from_u128(11));
    big_scripts(&db, &[a, b], 60);
    let newest_a: Vec<_> = rows_of(&db, a)
        .into_iter()
        .take(CAP_KEEP_PER_CONNECTION)
        .collect();
    // Far below what the kept scripts alone take: the cap goes on to the other history.
    let db = db.with_max_bytes(64 * 1024);

    db.thin(NOW, DEFAULT_MAX_CONTINUOUS_SECS).unwrap();

    assert_eq!(rows_of(&db, a), newest_a, "the newest of a, in order");
    assert_eq!(rows_of(&db, b).len(), CAP_KEEP_PER_CONNECTION);
}
