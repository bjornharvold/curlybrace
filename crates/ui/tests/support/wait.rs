//! Bounded waiting for the asynchronous runtime. There are no fixed sleeps used as assertions:
//! every wait ends as soon as its condition holds and FAILS with context at the deadline.
use egui_kittest::Harness;
use mongostudio_ui::app::MongoStudioApp;
use std::time::{Duration, Instant};

/// Real-time pause between frames while waiting for the runtime thread.
const FRAME_PAUSE: Duration = Duration::from_millis(15);
/// Default bound for ordinary round trips to a local server.
pub const SHORT: Duration = Duration::from_secs(20);

/// Steps frames (pumping runtime events into the UI) until `cond` holds.
///
/// # Panics
/// Panics with the app state summary once `timeout` has passed.
#[track_caller]
pub fn wait_until(
    harness: &mut Harness<'_, MongoStudioApp>,
    what: &str,
    timeout: Duration,
    mut cond: impl FnMut(&Harness<'_, MongoStudioApp>) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        harness.step();
        if cond(harness) {
            return;
        }
        if Instant::now() >= deadline {
            let s = harness.state().state();
            panic!(
                "timed out after {timeout:?} waiting for {what}: status={:?} banner={:?} \
                 pending={} page_docs={:?}",
                s.status,
                s.banner,
                s.pending.is_some(),
                s.page.as_ref().map(|p| p.docs.len()),
            );
        }
        std::thread::sleep(FRAME_PAUSE);
    }
}

/// Runs a few frames so a just-queued click or key is processed and the accessibility tree
/// reflects it. Frames are cheap; this is not a substitute for [`wait_until`] on async work.
pub fn settle(harness: &mut Harness<'_, MongoStudioApp>) {
    harness.run_steps(3);
}
