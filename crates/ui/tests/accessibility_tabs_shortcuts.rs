//! When the tab shortcuts act and when they must not (#159, review fixes): never while a text
//! field has the keyboard, never behind a dialog, only on the Browse page of a connected session.
mod support;

use curlybrace_ui::app::AggregateAction;
use curlybrace_ui::app::state::{AppState, ConnStatus, HostEndpoint, Workspace};
use curlybrace_ui::viewmodel::doc_actions::DeleteConfirm;
use curlybrace_ui::viewmodel::{CollectionTab, FindAction};
use eframe::egui::{self, Key, Modifiers, accesskit::Role};
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
fn a_chord_with_an_extra_modifier_is_not_the_shortcut() {
    let mut page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &["invoices", "notes"]);
    });
    let first = app_state(&mut page).active_tab();
    let shift = Modifiers::COMMAND | Modifiers::SHIFT;
    let alt = Modifiers::COMMAND | Modifiers::ALT;
    // Cmd+Shift+W is not Cmd+W; Cmd+Shift+3 and Cmd+Alt+3 are not Cmd+3; Cmd+Shift+9 is not Cmd+9.
    press(&mut page, shift, Key::W);
    press(&mut page, alt, Key::W);
    press(&mut page, shift, Key::Num3);
    press(&mut page, alt, Key::Num3);
    press(&mut page, shift, Key::Num9);
    assert_eq!(tab_count(&page), 3, "a chord closed a tab");
    assert_eq!(app_state(&mut page).active_tab(), first, "a chord switched");
    // Ctrl+Alt+Tab is not Ctrl+Tab.
    press(&mut page, Modifiers::CTRL | Modifiers::ALT, Key::Tab);
    assert_eq!(app_state(&mut page).active_tab(), first);
    // The real ones still work.
    press(&mut page, Modifiers::COMMAND, Key::Num3);
    assert_ne!(app_state(&mut page).active_tab(), first);
    press(&mut page, Modifiers::COMMAND, Key::W);
    assert_eq!(tab_count(&page), 2);
}

/// One key press as a keyboard layout reports it: the logical key it types, the physical key it
/// is, and the modifiers held.
fn layout_press(page: &mut BrowsePage, logical: Key, physical: Key, mods: Modifiers) {
    let key = |pressed| egui::Event::Key {
        key: logical,
        physical_key: Some(physical),
        pressed,
        repeat: false,
        modifiers: mods,
    };
    page.harness.input_mut().modifiers = mods;
    page.harness.input_mut().events.push(key(true));
    page.harness.input_mut().events.push(key(false));
    page.harness.run_steps(3);
    page.harness.input_mut().modifiers = Modifiers::NONE;
}

#[test]
fn a_digit_that_needs_shift_on_the_layout_still_picks_its_tab() {
    let mut page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &["a", "b", "c", "d"]);
    });
    let order = app_state(&mut page).tabs.order.clone();
    assert_eq!(order.len(), 5);
    // French AZERTY: the 4 key types `'` (Quote), and Cmd+4 reports the physical 4 key.
    layout_press(&mut page, Key::Quote, Key::Num4, Modifiers::COMMAND);
    assert_eq!(app_state(&mut page).active_tab(), Some(order[3]));
    // The digit typed with Shift is not the shortcut: QWERTY Cmd+Shift+3, and the same chord with
    // the digit as its logical key too.
    layout_press(
        &mut page,
        Key::Num3,
        Key::Num3,
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    layout_press(
        &mut page,
        Key::Num2,
        Key::Num2,
        Modifiers::COMMAND | Modifiers::SHIFT,
    );
    assert_eq!(app_state(&mut page).active_tab(), Some(order[3]));
    assert_eq!(tab_count(&page), 5);
}

#[test]
fn a_bracket_chord_is_found_by_its_physical_key_on_other_layouts() {
    let mut page = browse_page(SIZE_1000, |state| {
        browsing_with_tabs(state, &["a", "b"]);
    });
    let order = app_state(&mut page).tabs.order.clone();
    layout_press(&mut page, Key::Num1, Key::Num1, Modifiers::COMMAND);
    assert_eq!(app_state(&mut page).active_tab(), Some(order[0]));
    // A layout whose bracket keys type other characters: the physical key decides (Shift held,
    // no Alt needed), and an extra Alt on it is still refused.
    let next = Modifiers::COMMAND | Modifiers::SHIFT;
    layout_press(&mut page, Key::Semicolon, Key::CloseBracket, next);
    assert_eq!(app_state(&mut page).active_tab(), Some(order[1]));
    layout_press(
        &mut page,
        Key::Semicolon,
        Key::OpenBracket,
        next | Modifiers::ALT,
    );
    assert_eq!(app_state(&mut page).active_tab(), Some(order[1]));
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
