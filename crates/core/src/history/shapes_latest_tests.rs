//! `ShapeSummary::latest`: the verdict of the newest bucket that ran the shape, which is what a
//! row shows (the worst of the whole range stays available as `worst`). Gap bands belong to
//! shapes seen in the profiler only.

use super::db::HistoryDb;
use super::retention::ChartRange;
use super::shapes::{ShapeSource, ShapeSummary};
use super::shapes_tests::{NOW, conn_id, key, obs};
use crate::explain::VerdictKind;

fn read(db: &HistoryDb) -> Vec<ShapeSummary> {
    db.shapes_for(conn_id(1), "shop", "orders", ChartRange::Day, NOW)
        .unwrap()
}

#[test]
fn latest_is_the_newest_buckets_verdict_not_the_worst() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key(1, ShapeSource::App, "h");
    db.record_shapes(&[
        obs(
            k.clone(),
            NOW - 7_200,
            4,
            4,
            (40, 10),
            VerdictKind::CollectionScan,
        ),
        obs(k, NOW - 1_200, 5, 0, (5, 1), VerdictKind::IndexUsed),
    ])
    .unwrap();
    let got = read(&db);
    assert_eq!(
        got[0].worst,
        VerdictKind::CollectionScan,
        "worst of the range"
    );
    assert_eq!(
        got[0].latest,
        VerdictKind::IndexUsed,
        "the index now serves it"
    );
}

#[test]
fn latest_ignores_newer_buckets_that_did_not_execute_the_shape() {
    let db = HistoryDb::open_in_memory().unwrap();
    let k = key(1, ShapeSource::App, "h");
    let mut idle = obs(k.clone(), NOW - 600, 0, 0, (0, 0), VerdictKind::NoFilter);
    idle.gap_secs = 30;
    db.record_shapes(&[
        obs(k, NOW - 3_600, 2, 2, (20, 10), VerdictKind::SortNotCovered),
        idle,
    ])
    .unwrap();
    assert_eq!(read(&db)[0].latest, VerdictKind::SortNotCovered);
}

#[test]
fn the_same_bucket_on_two_hosts_takes_the_worse_verdict() {
    let db = HistoryDb::open_in_memory().unwrap();
    let mut other = key(1, ShapeSource::App, "h");
    other.host = "node-b:27017".into();
    db.record_shapes(&[
        obs(
            key(1, ShapeSource::App, "h"),
            NOW - 600,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ),
        obs(other, NOW - 600, 1, 1, (1, 1), VerdictKind::CollectionScan),
    ])
    .unwrap();
    assert_eq!(read(&db)[0].latest, VerdictKind::CollectionScan);
}

#[test]
fn typed_and_app_rows_each_keep_their_own_latest() {
    let db = HistoryDb::open_in_memory().unwrap();
    db.record_shapes(&[
        obs(
            key(1, ShapeSource::App, "h"),
            NOW - 3_600,
            1,
            1,
            (1, 1),
            VerdictKind::CollectionScan,
        ),
        obs(
            key(1, ShapeSource::App, "h"),
            NOW - 600,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ),
        obs(
            key(1, ShapeSource::Typed, "h"),
            NOW - 3_600,
            1,
            0,
            (0, 0),
            VerdictKind::IndexUsed,
        ),
        obs(
            key(1, ShapeSource::Typed, "h"),
            NOW - 600,
            1,
            1,
            (0, 0),
            VerdictKind::SortNotCovered,
        ),
    ])
    .unwrap();
    let got = read(&db);
    let latest = |source| got.iter().find(|s| s.source == source).unwrap().latest;
    assert_eq!(latest(ShapeSource::App), VerdictKind::IndexUsed);
    assert_eq!(latest(ShapeSource::Typed), VerdictKind::SortNotCovered);
}

#[test]
fn gap_bands_are_folded_into_app_shapes_only() {
    let db = HistoryDb::open_in_memory().unwrap();
    let at = NOW - 3_600;
    db.record_shapes(&[
        obs(
            key(1, ShapeSource::App, "a"),
            at,
            1,
            0,
            (1, 1),
            VerdictKind::IndexUsed,
        ),
        obs(
            key(1, ShapeSource::Typed, "t"),
            at,
            1,
            0,
            (0, 0),
            VerdictKind::IndexUsed,
        ),
    ])
    .unwrap();
    db.record_gap(conn_id(1), "node-a:27017", "shop", at, 120)
        .unwrap();
    let got = read(&db);
    let gap = |source| {
        got.iter()
            .find(|s| s.source == source)
            .unwrap()
            .buckets
            .iter()
            .map(|b| b.gap_secs)
            .sum::<u64>()
    };
    assert_eq!(gap(ShapeSource::App), 120);
    assert_eq!(
        gap(ShapeSource::Typed),
        0,
        "typed queries never ran in the profile"
    );
}
