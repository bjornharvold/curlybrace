//! Index usage history: pure logic that turns raw `$indexStats` samples into
//! plottable points. No I/O lives here.

pub mod series;
#[cfg(test)]
mod series_tests;

pub use series::{Bucket, GAP_INTERVALS, Point, SAMPLE_INTERVAL_SECS, Sample, bucketize, derive};
