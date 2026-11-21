//! Screenshot tests of the History popup on the Browse page: the find bar's History button opened
//! on a collection with recent queries and favourites, rendered with wgpu and compared with
//! committed PNGs in `tests/snapshots/`. No database is needed.
//!
//! Run:    `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test history_snapshots -- --include-ignored`
//! Update: add `UPDATE_SNAPSHOTS=1` (deliberately, then review the PNG diff).
mod support;

use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::I18n;
use curlybrace_ui::theme::Mode;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use support::history_shots::history_page;
use support::shots::Shot;

fn popup_open() -> Shot {
    let mut shot = Shot::new(vec![], history_page);
    shot.harness.run_steps(3);
    let label = I18n::english().t("history.button");
    shot.harness
        .get_by_role_and_label(Role::Button, &label)
        .click();
    shot.harness.run_steps(4);
    shot
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn browser_history_popup() {
    if !snapshots_enabled("browser_history_popup") {
        return;
    }
    popup_open().snapshot("browser_history_popup");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn browser_history_popup_dark() {
    if !snapshots_enabled("browser_history_popup_dark") {
        return;
    }
    popup_open()
        .mode(Mode::Dark)
        .snapshot("browser_history_popup_dark");
}
