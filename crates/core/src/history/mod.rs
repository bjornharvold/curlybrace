//! Index usage history: pure logic that turns raw `$indexStats` samples into
//! plottable points (`series`), and the SQLite store that keeps and thins them (`db`).

pub mod db;
#[cfg(test)]
mod db_purge_tests;
mod db_schema;
#[cfg(test)]
mod db_schema_v2_tests;
#[cfg(test)]
mod db_schema_v3_tests;
#[cfg(test)]
mod db_tests;
#[cfg(test)]
mod db_wal_tests;
#[cfg(test)]
mod gap_span_tests;
mod profile_mark;
#[cfg(test)]
mod profile_mark_tests;
pub mod retention;
mod seam;
#[cfg(test)]
mod seam_tests;
pub mod series;
#[cfg(test)]
mod series_tests;
mod shape_schema;
mod shape_thin;
#[cfg(test)]
mod shape_thin_tests;
pub mod shapes;
#[cfg(test)]
mod shapes_latest_tests;
mod shapes_read;
#[cfg(test)]
mod shapes_tests;
mod shell_history;
#[cfg(test)]
mod shell_history_cap_tests;
#[cfg(test)]
mod shell_history_tests;
#[cfg(test)]
mod skew_roll_tests;
#[cfg(test)]
mod skew_tests;
mod thin;
#[cfg(test)]
mod thin_gate_tests;
#[cfg(test)]
mod thin_lock_tests;
#[cfg(test)]
mod thin_order_tests;
mod usage;

pub use db::{HistoryDb, Recovery, SeriesFacts, SeriesKey, SeriesView, is_history_busy};
pub use profile_mark::ProfileMark;
pub use retention::{ChartRange, MAX_DB_BYTES};
pub use shape_schema::{RAW_BUCKET_SECS, TYPED_HOST};
pub use shapes::{ShapeBucket, ShapeKey, ShapeObservation, ShapeSource, ShapeSummary, is_miss};
pub use shell_history::{
    MAX_STORED_SCRIPT_BYTES, SHELL_HISTORY_LIMIT, ShellHistoryEntry, ShellRunStatus,
};

pub use series::{
    Bucket, DEFAULT_MAX_CONTINUOUS_SECS, GAP_INTERVALS, MAX_BUCKETS, MAX_FUTURE_SKEW_SECS, Point,
    SAMPLE_INTERVAL_SECS, Sample, bucketize, bucketize_within, derive, max_continuous_secs,
};
