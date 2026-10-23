//! Every reason a Review button is closed stays visible: a read-only destination never hides the
//! busy, running or checking reason that applies at the same time. Fast, offline, no GPU.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::compare::CompareState;
use curlybrace_ui::views::compare::show_compare;
use curlybrace_ui::views::connections::ConnectionsState;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use support::compare_apply_states::{applying, check_out, run_active, with_replace};

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn panel(state: AppState) -> Harness<'static, (CompareState, ConnectionsState)> {
    let mut compare = state.compare;
    compare.connections_read_only = (false, true);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(
            |ui, (s, c): &mut (CompareState, ConnectionsState)| {
                show_compare(ui, s, c, &I18n::english());
            },
            (compare, state.conn),
        );
    harness.run_steps(5);
    harness
}

fn count(harness: &Harness<'static, (CompareState, ConnectionsState)>, key: &str) -> usize {
    harness.query_all_by_label(&t(key)).count()
}

#[test]
fn a_read_only_target_and_a_running_compare_are_both_stated() {
    let harness = panel(run_active(with_replace()));
    let label = t("compare.apply.pending.review");
    let review = harness.get_by_role_and_label(Role::Button, &label);
    assert!(review.accesskit_node().is_disabled());
    assert!(count(&harness, "compare.apply.reason.read_only_target") >= 1);
    assert_eq!(count(&harness, "compare.apply.reason.running"), 1);
}

#[test]
fn a_read_only_target_and_an_apply_in_progress_are_both_stated() {
    let harness = panel(applying(with_replace()));
    assert!(count(&harness, "compare.apply.reason.read_only_target") >= 1);
    assert_eq!(count(&harness, "compare.apply.reason.busy"), 1);
}

#[test]
fn a_read_only_target_and_a_check_in_flight_are_both_stated() {
    let harness = panel(check_out(with_replace()));
    assert!(count(&harness, "compare.apply.reason.read_only_target") >= 1);
    assert_eq!(count(&harness, "compare.apply.reason.checking"), 1);
}

#[test]
fn a_read_only_target_alone_shows_only_its_own_reason() {
    let harness = panel(with_replace());
    assert!(count(&harness, "compare.apply.reason.read_only_target") >= 1);
    for key in ["running", "busy", "checking", "not_completed"] {
        assert_eq!(count(&harness, &format!("compare.apply.reason.{key}")), 0);
    }
}

#[test]
fn a_running_compare_alone_shows_only_its_own_reason() {
    let mut state = run_active(with_replace());
    state.compare.connections_read_only = (false, false);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(
            |ui, (s, c): &mut (CompareState, ConnectionsState)| {
                show_compare(ui, s, c, &I18n::english());
            },
            (state.compare, state.conn),
        );
    harness.run_steps(5);
    assert_eq!(count(&harness, "compare.apply.reason.running"), 1);
    assert_eq!(count(&harness, "compare.apply.reason.read_only_target"), 0);
}
