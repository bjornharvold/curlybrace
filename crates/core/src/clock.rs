//! Wall-clock seconds, kept in one place so callers can pass `now` in tests.

/// Seconds since the Unix epoch; 0 if the clock is before it.
pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
