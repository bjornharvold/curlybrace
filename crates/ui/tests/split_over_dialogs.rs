//! A press that lands on a dialog or a popup lying over the other pane is not a touch of that
//! pane (#159, slice 3 review): the active tab stays, the dialog works. A press on the other
//! pane itself still activates it. Fast, offline, no GPU.
mod support;

use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::EditorState;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use eframe::egui::{self, accesskit::Role};
use egui_kittest::kittest::{NodeT, Queryable};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The other (lower) pane's rectangle.
fn other_rect(page: &BrowsePage) -> egui::Rect {
    page.harness
        .query_all_by_role(Role::Group)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Other pane"))
        })
        .map(|n| n.rect())
        .expect("the other pane")
}

#[test]
fn a_copy_as_code_dialog_over_the_other_pane_does_not_switch_tabs() {
    // The dialog (centred, 770 wide) lies over the lower pane; a press on its code is not a touch
    // of that pane.
    let mut page = browse_page(STANDARD, |s| {
        support::code_export_shots::code_export_dialog_options(s);
        let session = s.active.expect("a session");
        *s = std::mem::take(s).tab_opened(session).0;
        let partner = support::tabs_shots::parked_tab(s, session, "crm", "invoices");
        support::tabs_shots::split_with(s, partner, Orientation::Stacked);
    });
    let active = state(&page).active_tab();
    assert!(
        state(&page).code_export.dialog.is_some(),
        "the dialog is open"
    );
    let lower = other_rect(&page);
    // A press on the dialog's code box (it has no node of its own), well inside the lower pane.
    let at = egui::pos2(600.0, 450.0);
    assert!(lower.contains(at), "the point lies over the other pane");
    page.harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(at));
    page.harness
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
    page.harness
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    page.harness.run_steps(4);
    assert_eq!(state(&page).active_tab(), active, "the active tab stayed");
    assert!(
        state(&page).code_export.dialog.is_some(),
        "and the dialog is still open"
    );
}

/// The stacked split with a popup of egui's own (a non-modal, non-interactable foreground area, as
/// egui's menus and combo lists are) hanging over the lower, other pane. No popup of the app
/// reaches over the other pane at 1000x700, so one is drawn after each frame.
#[test]
fn a_popup_over_the_other_pane_does_not_switch_tabs() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("tokio runtime");
    let enter = rt.enter();
    let (mut app, _dir) = support::shots::build_app(vec![], None);
    split_two(app.state_mut(), Orientation::Stacked);
    let clicked = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&clicked);
    let mut installed = false;
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(STANDARD.0, STANDARD.1))
        .with_pixels_per_point(1.0)
        .build_state(
            move |ctx, app: &mut curlybrace_ui::app::CurlybraceApp| {
                if !installed {
                    curlybrace_ui::theme::install(ctx);
                    installed = true;
                }
                app.run_frame(ctx);
                egui::Area::new(egui::Id::new("test_popup"))
                    .order(egui::Order::Foreground)
                    .interactable(false)
                    .fixed_pos(egui::pos2(420.0, 480.0))
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            if ui.button("Popup entry").clicked() {
                                flag.store(true, Ordering::SeqCst);
                            }
                        });
                    });
            },
            app,
        );
    harness.run_steps(6);
    drop(enter);
    let active = harness.state().state().active_tab();
    let lower = harness
        .query_all_by_role(Role::Group)
        .find(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.starts_with("Other pane"))
        })
        .map(|n| n.rect())
        .expect("the other pane");
    let entry = harness.get_by_label("Popup entry");
    assert!(
        lower.contains_rect(entry.rect()),
        "the popup lies over the other pane"
    );
    entry.click();
    harness.run_steps(4);
    assert!(clicked.load(Ordering::SeqCst), "the popup took the click");
    assert_eq!(
        harness.state().state().active_tab(),
        active,
        "the active tab stayed"
    );
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
