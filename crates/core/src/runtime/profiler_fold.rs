//! Folds profile entries into shape observations. Pure: no store, no history, no clock.
//!
//! Only the literal-free [`Shape`] of an entry leaves this module; the entry's `command` (which
//! holds the literal values) is read to build the shape and never copied.

use std::collections::BTreeMap;

use uuid::Uuid;

use crate::explain::{VerdictKind, worst};
use crate::history::{ShapeKey, ShapeObservation, ShapeSource, is_miss};
use crate::profile::{ProfileEntry, classify_entry, entry_has_filter, is_own_traffic};
use crate::shape::from_profile_command;

/// What one (shape, bucket) cell has seen so far.
struct Cell {
    shape_json: String,
    executions: u64,
    misses: u64,
    total_millis: u64,
    max_millis: u64,
    worst: VerdictKind,
}

/// A cell's place in the output: host, db, collection, shape and bucket start, in sort order.
type CellKey = (String, String, String, String, i64);

/// The database and collection of a namespace `db.collection`.
fn split_ns(ns: &str) -> Option<(&str, &str)> {
    ns.split_once('.')
        .filter(|(db, c)| !db.is_empty() && !c.is_empty())
}

/// Groups `entries` of `db` into observations per shape and `bucket_secs` bucket.
///
/// Skipped: cursor continuations, our own traffic, entries of another database (defensive: a
/// profile holds its own database only) and entries without a shape (an op without a filter, and
/// an aggregate whose `$match` is not its first stage). The output is sorted by collection,
/// shape and bucket, whatever the order of `entries`.
pub fn fold_entries(
    connection: Uuid,
    host: &str,
    db: &str,
    entries: &[ProfileEntry],
    bucket_secs: i64,
) -> Vec<ShapeObservation> {
    let mut cells: BTreeMap<CellKey, Cell> = BTreeMap::new();
    for entry in entries {
        if entry.is_continuation() || is_own_traffic(entry) {
            continue;
        }
        let Some((entry_db, collection)) = split_ns(&entry.ns).filter(|(d, _)| *d == db) else {
            continue;
        };
        let Some(shape) = from_profile_command(&entry.op, &entry.command) else {
            continue;
        };
        let kind = classify_entry(entry, entry_has_filter(entry));
        let bucket = entry.ts_secs - entry.ts_secs.rem_euclid(bucket_secs);
        let key = (
            host.to_owned(),
            entry_db.to_owned(),
            collection.to_owned(),
            shape.hash(),
            bucket,
        );
        cells
            .entry(key)
            .and_modify(|cell| cell.add(entry, kind))
            .or_insert_with(|| Cell::first(shape.json(), entry, kind));
    }
    cells
        .into_iter()
        .map(
            |((host, db, collection, hash, at_secs), cell)| ShapeObservation {
                key: ShapeKey {
                    connection,
                    host,
                    db,
                    collection,
                    hash,
                    source: ShapeSource::App,
                },
                shape_json: cell.shape_json,
                at_secs,
                executions: cell.executions,
                misses: cell.misses,
                total_millis: cell.total_millis,
                max_millis: cell.max_millis,
                worst: cell.worst,
                gap_secs: 0,
            },
        )
        .collect()
}

impl Cell {
    fn first(shape_json: String, entry: &ProfileEntry, kind: VerdictKind) -> Self {
        Self {
            shape_json,
            executions: 1,
            misses: u64::from(is_miss(kind)),
            total_millis: entry.millis,
            max_millis: entry.millis,
            worst: kind,
        }
    }

    fn add(&mut self, entry: &ProfileEntry, kind: VerdictKind) {
        self.executions = self.executions.saturating_add(1);
        self.misses = self.misses.saturating_add(u64::from(is_miss(kind)));
        self.total_millis = self.total_millis.saturating_add(entry.millis);
        self.max_millis = self.max_millis.max(entry.millis);
        self.worst = worst(self.worst, kind);
    }
}
