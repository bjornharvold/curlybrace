//! The compare apply review behaves like the production write warning when a destination is a
//! production connection: heading with the badge, Cancel first and focused, Apply second.
//! Reviews without a production destination are unchanged.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::compare::CompareAction;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::views::compare::show_compare;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::compare_apply_states::{act, reviewing, with_delete, with_replace};

struct Run {
    app: AppState,
    actions: Vec<CompareAction>,
}

/// The fixtures write to the connection named `two` (the second one).
fn review(production: bool, base: AppState) -> Harness<'static, Run> {
    let mut app = reviewing(base);
    app.compare.connections_production = (false, production);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1600.0))
        .build_ui_state(
            |ui, run: &mut Run| {
                let shown = show_compare(ui, &run.app.compare, &run.app.conn, &I18n::english());
                if let Some(action) = shown {
                    run.actions.push(action.clone());
                    run.app = act(
                        std::mem::replace(&mut run.app, AppState::new(Vec::new())),
                        action,
                    );
                }
            },
            Run {
                app,
                actions: Vec::new(),
            },
        );
    harness.run_steps(5);
    harness
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn left_of(h: &Harness<'_, Run>, key: &str) -> f32 {
    h.get_by_role_and_label(Role::Button, &t(key)).rect().left()
}

fn focused(h: &Harness<'_, Run>, key: &str) -> bool {
    h.get_by_role_and_label(Role::Button, &t(key))
        .accesskit_node()
        .is_focused()
}

const CANCEL: &str = "compare.apply.review.cancel";
const APPLY: &str = "compare.apply.review.apply";

#[test]
fn a_production_review_has_a_heading_with_the_production_badge() {
    let h = review(true, with_replace());
    h.get_by_label(&t("write_warning.title"));
    h.get_by_label(&t("connection.production_badge"));
}

#[test]
fn a_plain_review_has_neither_heading_nor_badge() {
    let h = review(false, with_replace());
    assert!(h.query_by_label(&t("write_warning.title")).is_none());
    assert!(
        h.query_by_label(&t("connection.production_badge"))
            .is_none()
    );
}

#[test]
fn production_puts_cancel_first_and_focused() {
    let h = review(true, with_replace());
    assert!(left_of(&h, CANCEL) < left_of(&h, APPLY), "Cancel first");
    assert!(focused(&h, CANCEL));
    assert!(!focused(&h, APPLY));
}

#[test]
fn enter_on_a_production_review_cancels_it() {
    let mut h = review(true, with_replace());
    h.key_press(egui::Key::Enter);
    h.run_steps(5);
    assert_eq!(h.state().actions, vec![CompareAction::CancelReview]);
}

#[test]
fn escape_on_a_production_review_cancels_it() {
    let mut h = review(true, with_replace());
    h.key_press(egui::Key::Escape);
    h.run_steps(5);
    assert_eq!(h.state().actions, vec![CompareAction::CancelReview]);
}

#[test]
fn apply_is_still_reachable_on_a_production_review() {
    let mut h = review(true, with_replace());
    h.get_by_role_and_label(Role::Button, &t(APPLY)).click();
    h.run_steps(5);
    assert_eq!(h.state().actions, vec![CompareAction::ConfirmApply]);
}

#[test]
fn a_plain_review_keeps_apply_first_and_no_forced_focus() {
    let h = review(false, with_replace());
    assert!(left_of(&h, APPLY) < left_of(&h, CANCEL), "Apply first");
    assert!(!focused(&h, CANCEL));
    assert!(!focused(&h, APPLY));
}

#[test]
fn typing_the_confirmation_on_a_production_review_keeps_the_text_focus() {
    let mut h = review(true, with_delete());
    h.get_by_role(Role::TextInput).focus();
    h.run_steps(5);
    h.get_by_role(Role::TextInput).type_text("users");
    h.run_steps(5);
    assert!(h.get_by_role(Role::TextInput).accesskit_node().is_focused());
    assert!(!focused(&h, CANCEL), "Cancel did not steal the focus");
    assert!(
        !h.get_by_role_and_label(Role::Button, &t(APPLY))
            .accesskit_node()
            .is_disabled()
    );
}
