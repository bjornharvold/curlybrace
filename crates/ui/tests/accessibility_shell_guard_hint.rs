//! The Shell tab's guard states in words, with the REAL English locale: the disabled Run's
//! tooltip says the queries are being checked while they are (not that a script runs), and a
//! check that answered in the background says the run was not started. Fast, offline, no GPU.
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::ConnectionBadge;
use curlybrace_ui::viewmodel::shell::{ShellNotice, ShellState};
use curlybrace_ui::views::shell::{ShellViewContext, show_shell};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn tab(shell: ShellState) -> Harness<'static, ShellState> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            |ui, shell: &mut ShellState| {
                let ctx = ShellViewContext {
                    session: Uuid::from_u128(12),
                    connection: "local",
                    badge: ConnectionBadge::default(),
                    connected: true,
                    now_secs: 0,
                    collections: &[],
                };
                let _ = show_shell(ui, shell, ctx, &I18n::english());
            },
            shell,
        );
    harness.run_steps(4);
    harness
}

fn script() -> ShellState {
    ShellState::default().edited("db.orders.find({ status: 1 })".to_string(), None)
}

/// The Shell tab showing `shell` after the pointer rests on the disabled Run button.
fn hovering_run(shell: ShellState) -> Harness<'static, ShellState> {
    let mut h = tab(shell);
    let label = t("shell.run");
    let run = h.get_by_role_and_label(Role::Button, &label);
    assert!(run.accesskit_node().is_disabled());
    run.hover();
    h.run_steps(8);
    h
}

#[test]
fn while_checking_the_disabled_run_says_the_queries_are_being_checked() {
    let checking = t("shell.run_disabled_checking");
    assert_eq!(checking, "Checking the queries in the script\u{2026}");
    let before = tab(script().checking(1, None, 0));
    let shown = before.query_all_by_label(&checking).count();

    let h = hovering_run(script().checking(1, None, 0));

    // The checking line shows the same words, so the tooltip makes them appear once more.
    assert!(h.query_all_by_label(&checking).count() > shown);
    assert!(h.query_by_label(&t("shell.run_disabled_running")).is_none());
}

#[test]
fn while_running_the_disabled_run_says_a_script_is_running() {
    let (running, _) = script().started("db.orders.find({ status: 1 })");

    let h = hovering_run(running);

    h.get_by_label(&t("shell.run_disabled_running"));
    assert!(
        h.query_by_label(&t("shell.run_disabled_checking"))
            .is_none()
    );
}

#[test]
fn a_run_cleared_in_the_background_says_it_was_not_started() {
    let shell = ShellState {
        notice: Some(ShellNotice::GuardNotStartedBackground),
        ..script()
    };
    let h = tab(shell);
    h.get_by_label(
        "The check finished while you were in another connection, so the script was not \
         started. Press Run again here.",
    );
}
