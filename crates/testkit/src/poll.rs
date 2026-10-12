use std::time::{Duration, Instant};

const STEP: Duration = Duration::from_millis(500);

/// Blocks until `ready()` holds; panics naming `what`, the elapsed time and the last failure
/// reason once `limit` has passed.
pub fn poll_with(limit: Duration, what: &str, mut ready: impl FnMut() -> Result<(), String>) {
    let start = Instant::now();
    loop {
        match ready() {
            Ok(()) => return,
            Err(last) if start.elapsed() >= limit => {
                panic!(
                    "timed out after {:?} waiting for {what}; last status: {last}",
                    start.elapsed()
                );
            }
            Err(_) => std::thread::sleep(STEP),
        }
    }
}

/// [`poll_with`] for a plain yes/no condition.
pub fn poll(limit: Duration, what: &str, mut ready: impl FnMut() -> bool) {
    poll_with(limit, what, || {
        if ready() {
            Ok(())
        } else {
            Err("condition not yet true".to_string())
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "last status: still booting")]
    fn failure_message_has_the_last_reason() {
        poll_with(
            Duration::from_millis(1),
            "x",
            || Err("still booting".into()),
        );
    }

    #[test]
    fn returns_as_soon_as_ready() {
        let mut calls = 0;
        poll(Duration::from_secs(5), "x", || {
            calls += 1;
            calls == 2
        });
        assert_eq!(calls, 2);
    }
}
