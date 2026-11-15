//! Screenshot tests of the Copy as code dialog (light and dark): a fixed in-memory state rendered
//! with wgpu and compared with committed PNGs in `tests/snapshots/`. No database is needed.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test code_export_snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::theme::Mode;
use support::shots::Shot;

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn code_export_dialog() {
    if !snapshots_enabled("code_export_dialog") {
        return;
    }
    Shot::new(vec![], support::code_export_shots::code_export_dialog)
        .snapshot("code_export_dialog");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn code_export_dialog_dark() {
    if !snapshots_enabled("code_export_dialog_dark") {
        return;
    }
    Shot::new(vec![], support::code_export_shots::code_export_dialog)
        .mode(Mode::Dark)
        .snapshot("code_export_dialog_dark");
}
