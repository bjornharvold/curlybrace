//! Screenshot tests of the find bar's Query options: a page found with a collation, a hint, a time
//! limit and a read preference, the section open with one problem under the comment, rendered
//! with wgpu and compared with committed PNGs in `tests/snapshots/`. No database is needed.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test find_options_snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::theme::Mode;
use support::find_options_shots::find_options_page;
use support::shots::Shot;

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn browser_find_options() {
    if !snapshots_enabled("browser_find_options") {
        return;
    }
    Shot::new(vec![], find_options_page).snapshot("browser_find_options");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn browser_find_options_dark() {
    if !snapshots_enabled("browser_find_options_dark") {
        return;
    }
    Shot::new(vec![], find_options_page)
        .mode(Mode::Dark)
        .snapshot("browser_find_options_dark");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn browser_find_options_small() {
    if !snapshots_enabled("browser_find_options_small") {
        return;
    }
    Shot::sized((800.0, 500.0), vec![], find_options_page).snapshot("browser_find_options_small");
}
