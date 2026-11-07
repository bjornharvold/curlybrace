//! The verdict badge in the accesskit tree: title and reasons are text led by an icon (never
//! colour alone), and the copy button has a name. Fast, offline, no GPU.
use curlybrace_core::explain::Reason;
use curlybrace_ui::I18n;
use curlybrace_ui::viewmodel::verdict::{BadgeModel, Tone};
use curlybrace_ui::views::verdict_badge::{VerdictAction, show_verdict_badge};
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn warn_model() -> BadgeModel {
    BadgeModel {
        tone: Tone::Warn,
        icon: "\u{26a0}",
        title_key: "query.verdict.title_collscan",
        reasons: vec![Reason::new("collscan")],
        suggestion: Some("db.orders.createIndex({ \"status\": 1 })".into()),
        plan_only: true,
    }
}

type Frame = (BadgeModel, Vec<VerdictAction>);

fn open(model: BadgeModel) -> Harness<'static, Frame> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 300.0))
        .build_ui_state(
            |ui, (model, seen): &mut Frame| {
                seen.extend(show_verdict_badge(ui, model, &I18n::english()));
            },
            (model, Vec::new()),
        );
    harness.run_steps(4);
    harness
}

#[test]
fn the_title_leads_with_an_icon_and_the_reasons_are_text() {
    let harness = open(warn_model());
    let title = format!("\u{26a0} {}", t("query.verdict.title_collscan"));
    harness.get_by_label(&title);
    harness.get_by_label(&t("query.verdict.reason_collscan"));
    harness.get_by_label(&t("query.verdict.plan_only"));
    harness.get_by_label("db.orders.createIndex({ \"status\": 1 })");
}

#[test]
fn the_copy_button_is_named_and_reports_the_command() {
    let mut harness = open(warn_model());
    harness
        .get_by_role_and_label(Role::Button, &t("query.verdict.copy"))
        .click();
    harness.run_steps(4);
    assert_eq!(
        harness.state().1,
        vec![VerdictAction::CopySuggestion(
            "db.orders.createIndex({ \"status\": 1 })".into()
        )]
    );
}

#[test]
fn a_checking_badge_says_so_in_text_and_has_no_copy_button() {
    let harness = open(BadgeModel {
        tone: Tone::Checking,
        icon: "\u{2026}",
        title_key: "query.verdict.checking",
        reasons: Vec::new(),
        suggestion: None,
        plan_only: false,
    });
    harness.get_by_label(&format!("\u{2026} {}", t("query.verdict.checking")));
    assert_eq!(
        harness.query_all_by_label(&t("query.verdict.copy")).count(),
        0
    );
}
