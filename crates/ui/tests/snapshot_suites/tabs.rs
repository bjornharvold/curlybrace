//! Screenshots of the collection tab strip (#159): light and dark, at the smallest window with six
//! tabs, with the "All tabs" menu open, with production and read-only badges, and with a dormant
//! tab. Included from `snapshots.rs` (which is at its size limit) so CI compiles and runs them
//! with the other screenshots.
use crate::support::shots::{Shot, browsing};
use crate::support::tabs_shots::{
    background_session, browsing_with_tabs, make_dormant, six_tabs, split_two,
    split_two_connections,
};
use curlybrace_testkit::snapshots_enabled;
use curlybrace_ui::I18n;
use curlybrace_ui::app::tools_state::ToolsState;
use curlybrace_ui::theme::Mode;
use curlybrace_ui::viewmodel::ViewMode;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;

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

// The split view (#159, slice 3): the standard window with the panes stacked (the size a person
// first meets it at), the same in the dark theme, a wide window with two connections side by
// side, and the smallest window, which has no room for two panes.
#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn split_stacked_1000x700() {
    if !snapshots_enabled("split_stacked_1000x700") {
        return;
    }
    Shot::sized((1000.0, 700.0), vec![], |state| {
        split_two(state, Orientation::Stacked)
    })
    .snapshot("split_stacked_1000x700");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn split_stacked_1000x700_dark() {
    if !snapshots_enabled("split_stacked_1000x700_dark") {
        return;
    }
    Shot::sized((1000.0, 700.0), vec![], |state| {
        split_two(state, Orientation::Stacked)
    })
    .mode(Mode::Dark)
    .snapshot("split_stacked_1000x700_dark");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn split_side_1400x900() {
    if !snapshots_enabled("split_side_1400x900") {
        return;
    }
    Shot::sized((1400.0, 900.0), vec![], |state| {
        split_two_connections(state, Orientation::SideBySide)
    })
    .snapshot("split_side_1400x900");
}

#[test]
#[ignore = "needs a GPU: set CURLYBRACE_SNAPSHOTS=1"]
fn split_single_800x500() {
    if !snapshots_enabled("split_single_800x500") {
        return;
    }
    Shot::sized((800.0, 500.0), vec![], |state| {
        split_two(state, Orientation::SideBySide)
    })
    .snapshot("split_single_800x500");
}

// The tab shown lost its connection: it keeps its query and waits for Run.
shot!(tabs_dormant, vec![], |state| {
    browsing_with_tabs(state, &["invoices"]);
    state.page = None;
    state.page_key = None;
    let shown = state.active_tab().expect("a shown tab");
    make_dormant(state, shown);
});

// A session restored from the saved file (#159, slice 4): the tabs are back and dormant, and the
// one shown says nothing has run yet.
shot!(tabs_restored_dormant, vec![], |state| {
    browsing_with_tabs(state, &["invoices", "notes"]);
    state.page = None;
    state.page_key = None;
    for view in state.parked_tabs.values_mut() {
        view.page = None;
        view.page_key = None;
    }
    let session = state.active.expect("an active session");
    for tab in state.tabs_of(session) {
        state.tabs = std::mem::take(&mut state.tabs).made_restored(tab, false);
    }
});

// The Settings dialog with the "Reopen tabs at launch" switch under the tool paths.
shot!(settings_restore_switch, vec![], |state| {
    browsing(ViewMode::Table)(state);
    let (tools, _) = ToolsState::default()
        .open_settings(BTreeMap::new())
        .take_queue();
    state.tools = tools;
});
