//! The run controls by accessible name and role, on real drafts: Run hidden or off with its
//! reason, the confirmation dialog's texts and controls, and the keyboard (focus in the name
//! field, Enter confirms only a matching name, Escape cancels).
use super::*;
use crate::theme;
use crate::viewmodel::aggregate::AggregateState;
use crate::viewmodel::aggregate_run::{RunEnv, RunUi};
use curlybrace_core::connection::ConnectionColor;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{PipelineError, RunTarget};
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

const BIG: (f32, f32) = (1000.0, 700.0);

struct Frame {
    availability: RunAvailability,
    run: RunUi,
    badge: ConnectionBadge,
    seen: Vec<AggregateAction>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn dialog(typed: &str) -> RunDialog {
    RunDialog {
        target: RunTarget {
            db: None,
            collection: "city_totals".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("crm", "customers"),
        typed: typed.into(),
        target_count: Some(42),
    }
}

fn harness(frame: Frame, size: (f32, f32)) -> Harness<'static, Frame> {
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(size.0, size.1))
        .build_ui_state(
            move |ui, f: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let i18n = I18n::english();
                let button = ui
                    .horizontal(|ui| show_run_button(ui, &f.availability, &i18n))
                    .inner;
                show_run_status(ui, &f.run.status, &i18n);
                let dialogs = show_run_dialog(ui.ctx(), f.run.dialog.as_ref(), &f.badge, &i18n);
                for action in button.into_iter().chain(dialogs) {
                    if let AggregateAction::RunTyped(text) = &action {
                        f.run = std::mem::take(&mut f.run).typed(text.clone());
                    }
                    f.seen.push(action);
                }
            },
            frame,
        );
    h.run_steps(4);
    h
}

fn with(availability: RunAvailability, run: RunUi) -> Frame {
    Frame {
        availability,
        run,
        badge: ConnectionBadge::default(),
        seen: Vec::new(),
    }
}

fn open_dialog(typed: &str) -> Harness<'static, Frame> {
    let run = RunUi::default().opened(dialog(typed));
    harness(with(RunAvailability::Ready, run), BIG)
}

fn name_label() -> String {
    I18n::english().t_with("aggregate.run.type_accessible", &[("name", "city_totals")])
}

fn focused(h: &Harness<'static, Frame>) -> Option<String> {
    h.root()
        .children_recursive()
        .find(|n| n.accesskit_node().is_focused())
        .and_then(|n| n.accesskit_node().label())
}

#[test]
fn run_is_not_there_for_a_pipeline_that_does_not_write() {
    let h = harness(with(RunAvailability::Hidden, RunUi::default()), BIG);
    let label = t("aggregate.run.button");
    assert!(h.query_by_role_and_label(Role::Button, &label).is_none());
}

#[test]
fn run_is_on_when_ready_and_asks_to_open_the_dialog() {
    let mut h = harness(with(RunAvailability::Ready, RunUi::default()), BIG);
    let label = t("aggregate.run.button");
    let run = h.get_by_role_and_label(Role::Button, &label);
    assert!(!run.accesskit_node().is_disabled());
    run.click();
    h.run_steps(2);
    assert_eq!(h.state().seen, [AggregateAction::RunClicked]);
}

#[test]
fn run_is_off_with_the_reason_of_each_refusal() {
    let cases = [
        (RunDisabled::ReadOnly, "aggregate.run.disabled.read_only"),
        (RunDisabled::Busy, "aggregate.run.disabled.busy"),
        (
            RunDisabled::TextNotApplied,
            "aggregate.run.disabled.text_not_applied",
        ),
        (
            RunDisabled::NoConnection,
            "aggregate.run.disabled.no_connection",
        ),
        (
            RunDisabled::NoCollection,
            "aggregate.run.disabled.no_collection",
        ),
    ];
    for (reason, key) in cases {
        let frame = with(RunAvailability::Disabled(reason), RunUi::default());
        let h = harness(frame, BIG);
        let label = t("aggregate.run.button");
        let run = h.get_by_role_and_label(Role::Button, &label);
        assert!(run.accesskit_node().is_disabled(), "{key}");
        assert_eq!(run.accesskit_node().description(), Some(t(key)), "{key}");
    }
}

#[test]
fn an_unusable_pipeline_names_its_problem_as_the_reason() {
    let reason = RunDisabled::Invalid(PipelineError::NoWriteStage);
    let frame = with(RunAvailability::Disabled(reason), RunUi::default());
    let h = harness(frame, BIG);
    let label = t("aggregate.run.button");
    let run = h.get_by_role_and_label(Role::Button, &label);
    assert_eq!(
        run.accesskit_node().description(),
        Some(t("aggregate.error.no_write_stage"))
    );
}

#[test]
fn the_dialog_states_what_will_happen() {
    let h = open_dialog("");
    let i18n = I18n::english();
    h.get_by_label(&t("aggregate.run.title"));
    h.get_by_label(&i18n.t_with("aggregate.run.source", &[("source", "crm.customers")]));
    let target = i18n.t_with("aggregate.run.target", &[("target", "crm.city_totals")]);
    h.get_by_label(&target);
    h.get_by_label(&t("aggregate.run.stage_out"));
    h.get_by_label(&i18n.t_with("aggregate.run.count", &[("count", "42")]));
    h.get_by_label(&t("aggregate.run.irreversible"));
}

#[test]
fn a_merge_dialog_says_what_merge_does_and_an_unknown_count_is_not_available() {
    let merge = RunDialog {
        stage: "$merge".into(),
        target_count: None,
        target: RunTarget {
            db: Some("reports".into()),
            collection: "city_totals".into(),
        },
        ..dialog("")
    };
    let run = RunUi::default().opened(merge);
    let h = harness(with(RunAvailability::Ready, run), BIG);
    h.get_by_label(&t("aggregate.run.stage_merge"));
    h.get_by_label(&t("aggregate.run.count_unavailable"));
    let target =
        I18n::english().t_with("aggregate.run.target", &[("target", "reports.city_totals")]);
    h.get_by_label(&target);
}

#[test]
fn a_production_connection_shows_its_badge_in_the_dialog() {
    let mut frame = with(RunAvailability::Ready, RunUi::default().opened(dialog("")));
    frame.badge = ConnectionBadge::new(Some(ConnectionColor::Red), true);
    let h = harness(frame, BIG);
    h.get_by_label(&t(crate::viewmodel::connection_tag::PRODUCTION_BADGE_KEY));
}

#[test]
fn the_name_field_is_named_focused_and_run_waits_for_a_matching_name() {
    let h = open_dialog("");
    h.get_by_role_and_label(Role::TextInput, &name_label());
    assert_eq!(focused(&h), Some(name_label()));
    let label = t("aggregate.run.confirm");
    let run = h.get_by_role_and_label(Role::Button, &label);
    assert!(run.accesskit_node().is_disabled());
    assert_eq!(
        run.accesskit_node().description(),
        Some(t("aggregate.run.confirm_blocked"))
    );
    h.get_by_role_and_label(Role::Button, &t("common.cancel"));
}

#[test]
fn a_mismatch_keeps_run_off_and_a_match_turns_it_on() {
    let cases = [
        ("city_total", false),
        ("City_Totals", false),
        (" city_totals ", true),
    ];
    for (typed, on) in cases {
        let h = open_dialog(typed);
        let label = t("aggregate.run.confirm");
        let run = h.get_by_role_and_label(Role::Button, &label);
        assert_eq!(!run.accesskit_node().is_disabled(), on, "{typed:?}");
    }
}

#[test]
fn typing_reports_the_text_and_the_matching_name_enables_run() {
    let mut h = open_dialog("");
    h.get_by_role_and_label(Role::TextInput, &name_label())
        .type_text("city_totals");
    h.run_steps(4);
    let typed = AggregateAction::RunTyped("city_totals".into());
    assert!(h.state().seen.contains(&typed));
    let label = t("aggregate.run.confirm");
    let run = h.get_by_role_and_label(Role::Button, &label);
    assert!(!run.accesskit_node().is_disabled());
    run.click();
    h.run_steps(2);
    assert_eq!(h.state().seen.last(), Some(&AggregateAction::RunConfirmed));
}

#[test]
fn enter_confirms_only_a_matching_name() {
    let mut mismatch = open_dialog("city");
    mismatch.key_press(egui::Key::Enter);
    mismatch.run_steps(4);
    assert!(
        !mismatch
            .state()
            .seen
            .contains(&AggregateAction::RunConfirmed)
    );

    let mut matching = open_dialog("city_totals");
    matching.key_press(egui::Key::Enter);
    matching.run_steps(4);
    assert!(
        matching
            .state()
            .seen
            .contains(&AggregateAction::RunConfirmed)
    );
}

#[test]
fn escape_and_cancel_dismiss_the_dialog() {
    let mut h = open_dialog("city_totals");
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert_eq!(h.state().seen, [AggregateAction::RunDismissed]);
    let mut h = open_dialog("");
    h.get_by_role_and_label(Role::Button, &t("common.cancel"))
        .click();
    h.run_steps(2);
    assert_eq!(h.state().seen, [AggregateAction::RunDismissed]);
}

#[test]
fn each_status_has_its_own_line() {
    let i18n = I18n::english();
    let done = |count| RunStatus::Done {
        target: Namespace::new("crm", "city_totals"),
        count,
    };
    let failed = i18n.error(&CoreError::NotFound);
    let cases = [
        (RunStatus::Running, t("aggregate.run.status.running")),
        (
            done(Some(7)),
            i18n.t_with(
                "aggregate.run.status.done",
                &[("target", "crm.city_totals"), ("count", "7")],
            ),
        ),
        (
            done(None),
            i18n.t_with(
                "aggregate.run.status.done_no_count",
                &[("target", "crm.city_totals")],
            ),
        ),
        (
            RunStatus::Failed(CoreError::NotFound),
            i18n.t_with("aggregate.run.status.failed", &[("error", &failed)]),
        ),
        (RunStatus::Unknown, t("aggregate.run.status.unknown")),
    ];
    for (status, text) in cases {
        let run = RunUi {
            status,
            ..RunUi::default()
        };
        let h = harness(with(RunAvailability::Ready, run), BIG);
        h.get_by_label(&text);
    }
}

#[test]
fn a_draft_that_writes_offers_run_and_a_read_draft_does_not() {
    let env = RunEnv {
        connected: true,
        has_collection: true,
        read_only: true,
    };
    let read = AggregateState::default().add_stage("$match");
    assert_eq!(read.can_run(env), RunAvailability::Hidden);
    let write = AggregateState::default()
        .add_stage("$match")
        .add_stage("$out")
        .edit(1, "{ \"$out\": \"city_totals\" }");
    assert_eq!(
        write.can_run(env),
        RunAvailability::Disabled(RunDisabled::ReadOnly)
    );
}
