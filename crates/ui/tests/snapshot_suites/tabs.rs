//! Screenshots of the collection tab strip (#159): light and dark, at the smallest window with six
//! tabs, with the "All tabs" menu open, with production and read-only badges, and with a dormant
//! tab. Included from `snapshots.rs` (which is at its size limit) so CI compiles and runs them
//! with the other screenshots.
use crate::support::shots::Shot;
use crate::support::tabs_shots::{background_session, browsing_with_tabs, make_dormant, six_tabs};
use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::I18n;
use curlybrace_ui::theme::Mode;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

/// Three collections of one connection, the first shown.
fn three_tabs(state: &mut curlybrace_ui::app::state::AppState) {
    browsing_with_tabs(state, &["invoices", "notes"]);
}

shot!(tabs_strip, vec![], three_tabs);

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn tabs_strip_dark() {
    if !snapshots_enabled("tabs_strip_dark") {
        return;
    }
    Shot::new(vec![], three_tabs)
        .mode(Mode::Dark)
        .snapshot("tabs_strip_dark");
}

// The smallest supported window with six tabs: the strip scrolls and keeps the active one in view.
#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn tabs_strip_small() {
    if !snapshots_enabled("tabs_strip_small") {
        return;
    }
    Shot::sized((800.0, 500.0), vec![], six_tabs).snapshot("tabs_strip_small");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn tabs_all_menu() {
    if !snapshots_enabled("tabs_all_menu") {
        return;
    }
    let mut shot = Shot::new(vec![], six_tabs);
    let name = I18n::english().t("tabs.all_tabs");
    shot.harness
        .get_by_role_and_label(Role::Button, &name)
        .click();
    shot.harness.run();
    shot.snapshot("tabs_all_menu");
}

// A production tab, a read-only one and a production read-only one beside the plain ones.
shot!(tabs_production_read_only, vec![], |state| {
    browsing_with_tabs(state, &["invoices"]);
    background_session(state, "Staging", "orders", (true, false));
    background_session(state, "Reports", "sessions", (false, true));
    background_session(state, "Billing", "ledger", (true, true));
});

// The tab shown lost its connection: it keeps its query and waits for Run.
shot!(tabs_dormant, vec![], |state| {
    browsing_with_tabs(state, &["invoices"]);
    state.page = None;
    state.page_key = None;
    let shown = state.active_tab().expect("a shown tab");
    make_dormant(state, shown);
});
