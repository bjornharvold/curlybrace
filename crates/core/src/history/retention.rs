//! How long each tier of the sample store is kept, and the chart ranges built on them.

/// Raw samples (one per minute) are kept this long: 48 hours.
pub const RAW_RETENTION_SECS: i64 = 48 * 3_600;

/// Hourly rolled rows are kept this long: 90 days.
pub const HOURLY_RETENTION_SECS: i64 = 90 * 86_400;

/// Daily rolled rows are kept this long: 2 years (730 days).
pub const DAILY_RETENTION_SECS: i64 = 2 * 365 * 86_400;

/// Width of an hourly rolled row.
pub const HOUR_SECS: i64 = 3_600;

/// Width of a daily rolled row.
pub const DAY_SECS: i64 = 86_400;

/// Hard cap on the history database: about 100 MB. When thinning leaves the file larger, the
/// oldest rows are dropped (daily, then hourly, then raw) until it fits.
pub const MAX_DB_BYTES: u64 = 100 * 1024 * 1024;

/// How much history a chart shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartRange {
    /// The last 24 hours, in 10 minute buckets.
    Day,
    /// The last 7 days, in hourly buckets.
    Week,
    /// The last 30 days, in daily buckets.
    Month,
}

impl ChartRange {
    /// Length of the window, in seconds.
    pub fn secs(self) -> i64 {
        match self {
            Self::Day => DAY_SECS,
            Self::Week => 7 * DAY_SECS,
            Self::Month => 30 * DAY_SECS,
        }
    }

    /// Width of one bucket, in seconds.
    pub fn bucket_width_secs(self) -> i64 {
        match self {
            Self::Day => 600,
            Self::Week => HOUR_SECS,
            Self::Month => DAY_SECS,
        }
    }
}
