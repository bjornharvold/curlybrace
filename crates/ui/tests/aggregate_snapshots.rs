//! Screenshot tests of the Aggregate tab: fixed in-memory states rendered with wgpu and compared
//! with committed PNGs in `tests/snapshots/`. No database is needed.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test aggregate_snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::theme::Mode;
use support::shots::Shot;

/// Declares a `#[ignore]`d, env-gated test that builds a [`Shot`] and snapshots it.
macro_rules! shot {
    ($name:ident, $connections:expr, $setup:expr) => {
        #[test]
        #[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
        fn $name() {
            if !snapshots_enabled(stringify!($name)) {
                return;
            }
            Shot::new($connections, $setup).snapshot(stringify!($name));
        }
    };
}

// The Aggregate tab: `$match`, `$group`, `$sort` with stage 2 previewed (four documents).
shot!(
    aggregate_tab,
    vec![],
    support::aggregate_shots::aggregate_tab
);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn aggregate_tab_dark() {
    if !snapshots_enabled("aggregate_tab_dark") {
        return;
    }
    Shot::new(vec![], support::aggregate_shots::aggregate_tab)
        .mode(Mode::Dark)
        .snapshot("aggregate_tab_dark");
}

// The Aggregate header with an opened, modified saved pipeline (light and dark).
shot!(
    aggregate_opened,
    vec![],
    support::aggregate_shots::aggregate_opened_tab
);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn aggregate_opened_dark() {
    if !snapshots_enabled("aggregate_opened_dark") {
        return;
    }
    Shot::new(vec![], support::aggregate_shots::aggregate_opened_tab)
        .mode(Mode::Dark)
        .snapshot("aggregate_opened_dark");
}

// The confirmation dialog of a `$out` run on a production connection (light and dark).
shot!(
    aggregate_run_dialog,
    vec![],
    support::aggregate_shots::aggregate_run_dialog_tab
);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn aggregate_run_dialog_dark() {
    if !snapshots_enabled("aggregate_run_dialog_dark") {
        return;
    }
    Shot::new(vec![], support::aggregate_shots::aggregate_run_dialog_tab)
        .mode(Mode::Dark)
        .snapshot("aggregate_run_dialog_dark");
}
