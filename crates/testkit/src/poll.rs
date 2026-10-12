use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(500);

/// Blocks until `ready()` holds; panics naming `what` once `limit` has passed.
pub fn poll(limit: Duration, what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(STEP);
    }
}
