//! Index usage history: pure logic that turns raw `$indexStats` samples into
//! plottable points (`series`), and the SQLite store that keeps and thins them (`db`).

pub mod db;
mod db_schema;
#[cfg(test)]
mod db_tests;
pub mod retention;
pub mod series;
#[cfg(test)]
mod series_tests;
mod thin;
mod usage;

pub use db::{HistoryDb, Recovery, SeriesFacts, SeriesKey};
pub use retention::{ChartRange, MAX_DB_BYTES};

pub use series::{
    Bucket, DEFAULT_MAX_CONTINUOUS_SECS, GAP_INTERVALS, Point, SAMPLE_INTERVAL_SECS, Sample,
    bucketize, derive, max_continuous_secs,
};
