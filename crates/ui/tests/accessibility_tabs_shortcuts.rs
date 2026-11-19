//! When the tab shortcuts act and when they must not (#159, review fixes): never while a text
//! field has the keyboard, never behind a dialog, only on the Browse page of a connected session.
mod support;

use curlybrace_ui::app::AggregateAction;
use curlybrace_ui::app::state::{AppState, ConnStatus, HostEndpoint, Workspace};
use curlybrace_ui::viewmodel::doc_actions::DeleteConfirm;
use curlybrace_ui::viewmodel::{CollectionTab, FindAction};
use eframe::egui::{Key, Modifiers, accesskit::Role};
use egui_kittest::kittest::Queryable;
use std::collections::BTreeMap;
use support::browse_page::{BrowsePage, SIZE_1000, browse_page};
use support::tabs_shots::browsing_with_tabs;

fn t(key: &str) -> &'static str {
    Box::leak(support::browse_page::t(key).into_boxed_str())
}

fn two_tabs() -> BrowsePage {
    browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &["invoices"]);
    })
}

fn tab_count(page: &BrowsePage) -> usize {
    page.harness.state().state().tabs.order.len()
}

fn press(page: &mut BrowsePage, mods: Modifiers, key: Key) {
    page.harness.key_press_modifiers(mods, key);
    page.harness.run_steps(3);
}

fn app_state(page: &mut BrowsePage) -> &mut AppState {
    page.harness.state_mut().state_mut()
}

/// Cmd+W with `open` having just opened a dialog: no tab closes.
fn nothing_closes_behind(open: impl FnOnce(&mut BrowsePage)) {
    let mut page = two_tabs();
    open(&mut page);
    page.harness.run_steps(4);
    let before = tab_count(&page);
    press(&mut page, Modifiers::COMMAND, Key::W);
    assert_eq!(tab_count(&page), before, "a tab closed behind a dialog");
    press(&mut page, Modifiers::COMMAND, Key::T);
    assert_eq!(tab_count(&page), before, "a tab opened behind a dialog");
}

#[test]
fn typing_in_the_filter_field_is_never_taken() {
    let mut page = two_tabs();
    page.harness
        .get_by_role_and_label(Role::TextInput, t("findbar.filter"))
        .click();
    page.harness.run_steps(4);
    press(&mut page, Modifiers::COMMAND, Key::W);
    assert_eq!(tab_count(&page), 2, "Cmd+W while typing closed a tab");
    press(&mut page, Modifiers::COMMAND, Key::T);
    assert_eq!(tab_count(&page), 2, "Cmd+T while typing opened a tab");
}

#[test]
fn with_a_tab_button_focused_the_shortcuts_work() {
    let mut page = two_tabs();
    page.harness
        .query_all_by_role(Role::Tab)
        .next()
        .expect("a tab")
        .focus();
    page.harness.run_steps(4);
    press(&mut page, Modifiers::COMMAND, Key::W);
    assert_eq!(tab_count(&page), 1, "a focused button is not typing");
}

#[test]
fn nothing_acts_behind_the_copy_as_code_dialog() {
    nothing_closes_behind(|page| {
        page.harness
            .state_mut()
            .dispatch(curlybrace_ui::app::reducer::Input::Find(
                FindAction::CopyAsCode,
            ));
    });
}

#[test]
fn nothing_acts_behind_the_saved_pipeline_name_dialog() {
    nothing_closes_behind(|page| {
        use curlybrace_ui::app::reducer::Input;
        let app = page.harness.state_mut();
        app.dispatch(Input::Tab(CollectionTab::Aggregate));
        app.dispatch(Input::Aggregate(AggregateAction::AddStage("$match".into())));
        app.dispatch(Input::Aggregate(AggregateAction::SaveClicked));
    });
}

#[test]
fn nothing_acts_behind_the_delete_confirm() {
    nothing_closes_behind(|page| {
        // What the context menu's Delete parks: the confirmation of this page's document.
        let state = app_state(page);
        let key = state.page_key.expect("a page is shown");
        state.tree.confirm = DeleteConfirm::Pending {
            id: bson::Bson::Int32(1),
            page: key,
        };
    });
}

#[test]
fn nothing_acts_behind_the_forget_host_confirm() {
    nothing_closes_behind(|page| {
        app_state(page).forget_confirm = Some(HostEndpoint {
            host: "db.example.com".into(),
            port: 22,
        });
    });
}

#[test]
fn nothing_acts_behind_the_tool_settings() {
    nothing_closes_behind(|page| {
        let state = app_state(page);
        state.tools = std::mem::take(&mut state.tools).open_settings(BTreeMap::new());
    });
}
#[test]
fn the_shortcuts_stay_off_the_shell_and_compare_pages() {
    for workspace in [Workspace::Shell, Workspace::Compare] {
        let mut page = two_tabs();
        app_state(&mut page).workspace = workspace;
        page.harness.run_steps(3);
        press(&mut page, Modifiers::COMMAND, Key::T);
        press(&mut page, Modifiers::COMMAND, Key::W);
        assert_eq!(tab_count(&page), 2, "{workspace:?}");
    }
}

#[test]
fn a_new_tab_needs_a_connection_but_close_and_switch_do_not() {
    for status in [ConnStatus::Failed, ConnStatus::Connecting] {
        let mut page = browse_page(SIZE_1000, |state| {
            browsing_with_tabs(state, &["invoices"]);
        });
        app_state(&mut page).status = status;
        page.harness.run_steps(3);
        // Cmd+T has nothing to read from: it does nothing.
        press(&mut page, Modifiers::COMMAND, Key::T);
        assert_eq!(tab_count(&page), 2, "{status:?}");
        // Cmd+W closes the dormant-to-be tab even though the connection is down.
        press(&mut page, Modifiers::COMMAND, Key::W);
        assert_eq!(tab_count(&page), 1, "Cmd+W while {status:?}");
    }
}

#[test]
fn after_a_lost_connection_the_shortcuts_still_reach_another_connections_tab() {
    let mut page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &["invoices"]);
        support::tabs_shots::background_session(state, "Staging", "orders", (false, false));
    });
    let other = *app_state(&mut page).tabs.order.last().expect("a tab");
    app_state(&mut page).status = ConnStatus::Failed;
    page.harness.run_steps(3);
    // Cmd+9 is the last tab: the other connection's, which is still connected.
    press(&mut page, Modifiers::COMMAND, Key::Num9);
    assert_eq!(app_state(&mut page).active_tab(), Some(other));
    assert_eq!(app_state(&mut page).status, ConnStatus::Connected);
}
