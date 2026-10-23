//! The compare apply review names a production destination, driven through the real entry
//! point (`show_compare`), so removing the wiring from the connections list to the dialog fails.
mod support;

use curlybrace_ui::I18n;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::views::compare::show_compare;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use support::compare_apply_states::{reviewing, with_insert};

/// The insert fixture writes to the connection named `two` (the second one).
fn review_with_two_production(production: bool) -> AppState {
    let mut state = reviewing(with_insert());
    state.conn.connections[1].production = production;
    state.compare.connections_production = (false, production);
    state
}

fn notice_shown(state: AppState) -> bool {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100.0, 1600.0))
        .build_ui_state(
            |ui, state: &mut AppState| {
                let _ = show_compare(ui, &state.compare, &state.conn, &I18n::english());
            },
            state,
        );
    harness.run_steps(5);
    let line = I18n::english().t_with(
        "compare.apply.review.production_line",
        &[("connection", "two")],
    );
    harness.query_by_label(&line).is_some()
}

#[test]
fn the_review_names_a_production_destination() {
    assert!(notice_shown(review_with_two_production(true)));
}

#[test]
fn the_review_has_no_production_notice_for_a_plain_destination() {
    assert!(!notice_shown(review_with_two_production(false)));
}

#[test]
fn a_production_connection_that_is_only_read_gets_no_notice() {
    let mut state = reviewing(with_insert());
    state.conn.connections[0].production = true;
    state.compare.connections_production = (true, false);
    assert!(!notice_shown(state));
}

#[test]
fn editing_the_saved_flag_after_opening_changes_nothing() {
    let mut state = review_with_two_production(true);
    state.conn.connections[1].production = false;
    assert!(notice_shown(state), "the session opened on production");
    let mut state = review_with_two_production(false);
    state.conn.connections[1].production = true;
    assert!(!notice_shown(state), "the session opened on a plain one");
}
