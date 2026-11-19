//! "Last run" of a recorded query shape, from the newest history bucket that ran it. A bucket
//! says only that the run happened somewhere inside it (10 minutes, 1 hour or 1 day wide, by
//! range), so the reading never claims more than that: an age counted from the bucket's end when
//! that is old enough to tell, otherwise "within the last" bucket width. It is never "just now".
//! The same shape can therefore read a little differently on the 24 h, 7 d and 30 d ranges: that
//! is the resolution the history keeps.
use super::index_detail::Age;
use curlybrace_core::history::ShapeSummary;

const TEN_MINUTES_SECS: i64 = 600;
const HOUR_SECS: i64 = 3_600;

/// The newest bucket of a shape that ran it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastRunBucket {
    /// Where the bucket ends (Unix seconds); after now while the bucket is still open.
    pub end_secs: i64,
    pub width_secs: i64,
}

/// The bucket width tier a reading is worded by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketWidth {
    TenMinutes,
    Hour,
    Day,
}

impl BucketWidth {
    /// The tier of a bucket `width_secs` wide: the smallest that holds it.
    pub fn of(width_secs: i64) -> Self {
        match width_secs {
            w if w <= TEN_MINUTES_SECS => Self::TenMinutes,
            w if w <= HOUR_SECS => Self::Hour,
            _ => Self::Day,
        }
    }

    /// The `en.json` key of "Last run within the last ...".
    pub fn within_key(self) -> &'static str {
        match self {
            Self::TenMinutes => "indexes.queries.last_run_within_ten_minutes",
            Self::Hour => "indexes.queries.last_run_within_hour",
            Self::Day => "indexes.queries.last_run_within_day",
        }
    }
}

/// How a last run reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastRun {
    /// This long after the bucket ended (at least a minute: never [`Age::Now`]).
    Ago(Age),
    /// Somewhere in the bucket, which is still open or ended too recently to count an age from.
    Within(BucketWidth),
}

/// The newest bucket that ran the shape; `None` when no bucket did.
pub fn last_run_bucket(summary: &ShapeSummary) -> Option<LastRunBucket> {
    summary
        .buckets
        .iter()
        .filter(|b| b.executions > 0)
        .map(|b| LastRunBucket {
            end_secs: b.start_secs + b.width_secs,
            width_secs: b.width_secs,
        })
        .max_by_key(|b| b.end_secs)
}

/// How `bucket` reads at `now_secs`.
pub fn last_run(bucket: LastRunBucket, now_secs: i64) -> LastRun {
    let width = BucketWidth::of(bucket.width_secs);
    match Age::of(now_secs - bucket.end_secs) {
        _ if bucket.end_secs > now_secs => LastRun::Within(width),
        Age::Now => LastRun::Within(width),
        age => LastRun::Ago(age),
    }
}
