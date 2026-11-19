//! A press that lands on a dialog or a popup lying over the other pane is not a touch of that
//! pane (#159, slice 3 review): the active tab stays, the dialog works. A press on the other
//! pane itself still activates it. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use curlybrace_ui::viewmodel::{EditorState, FindAction};
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::{NodeT, Queryable};
use support::browse_page::{BrowsePage, browse_page};
use support::tabs_shots::split_two;

const STANDARD: (f32, f32) = (1000.0, 700.0);
const WIDE: (f32, f32) = (1400.0, 900.0);

fn t(key: &str) -> &'static str {
    Box::leak(support::browse_page::t(key).into_boxed_str())
}

fn state(page: &BrowsePage) -> &AppState {
    page.harness.state().state()
}

fn stacked() -> BrowsePage {
    browse_page(STANDARD, |s| split_two(s, Orientation::Stacked))
}

/// The lower pane's top edge: the rectangle of the group named "Other pane".
fn other_top(page: &BrowsePage) -> f32 {
    page.harness
        .query_all_by_role(Role::Group)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Other pane"))
        })
        .map(|n| n.rect().top())
        .expect("the other pane")
}

#[test]
fn clicking_save_and_cancel_over_the_other_pane_keeps_the_active_tab() {
    let mut page = stacked();
    let active = state(&page).active_tab();
    page.harness.state_mut().state_mut().editor = Some(EditorState::for_new());
    page.harness.run_steps(3);
    let top = other_top(&page);
    for key in ["editor.save", "editor.cancel"] {
        let button = page.harness.get_by_role_and_label(Role::Button, t(key));
        assert!(
            button.rect().bottom() > top,
            "{key} lies over the other pane"
        );
    }
    page.harness
        .get_by_role_and_label(Role::Button, t("editor.cancel"))
        .click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), active, "the active tab stayed");
    assert!(state(&page).editor.is_none(), "Cancel closed the editor");
}

#[test]
fn a_dialog_over_the_other_pane_side_by_side_keeps_the_active_tab() {
    let mut page = browse_page(WIDE, |s| split_two(s, Orientation::SideBySide));
    let active = state(&page).active_tab();
    page.harness.state_mut().state_mut().editor = Some(EditorState::for_new());
    page.harness.run_steps(3);
    page.harness
        .get_by_role_and_label(Role::MultilineTextInput, t("editor.title_new"))
        .click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), active);
    assert!(state(&page).editor.is_some());
}

#[test]
fn a_copy_as_code_dialog_over_the_other_pane_does_not_switch_tabs() {
    let mut page = stacked();
    let active = state(&page).active_tab();
    page.harness
        .state_mut()
        .dispatch(Input::Find(FindAction::CopyAsCode));
    page.harness.run_steps(4);
    let close = page
        .harness
        .query_all_by_role(Role::Button)
        .find(|n| n.rect().top() > other_top(&page) && n.rect().width() > 0.0)
        .map(|n| n.rect().center());
    assert!(close.is_some(), "a dialog button lies over the other pane");
    // Any press on the dialog, wherever it lies.
    let button = page
        .harness
        .query_all_by_role(Role::Button)
        .find(|n| n.rect().top() > other_top(&page))
        .expect("a button");
    button.click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), active);
}

#[test]
fn a_context_menu_over_the_other_pane_does_not_switch_tabs() {
    let mut page = stacked();
    let active = state(&page).active_tab();
    // The strip's tab menu hangs over the upper pane and reaches the lower one.
    let tab = page
        .harness
        .query_all_by_role(Role::Tab)
        .next()
        .expect("a tab");
    tab.click_secondary();
    page.harness.run_steps(4);
    let entry = page
        .harness
        .query_all_by_role(Role::Button)
        .find(|n| n.accesskit_node().label().as_deref() == Some(t("tabs.move_right")))
        .expect("a menu entry");
    entry.click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), active);
}

#[test]
fn clicking_the_other_pane_itself_still_activates_it() {
    let mut page = stacked();
    let split = state(&page).tabs.split.clone().expect("a split");
    let group = page
        .harness
        .query_all_by_role(Role::Group)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Other pane"))
        })
        .expect("the other pane");
    group.click();
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), Some(split.secondary));
}
