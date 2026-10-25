//! The paste dialog, its progress and its report in the accesskit tree: labelled controls, the
//! keyboard path (Cancel first and focused when the copy is risky, Escape cancels), a disabled
//! Copy with its stated reason, and a report that lists keys and message keys only. Fast,
//! offline, no GPU.
use bson::{Bson, Document};
use curlybrace_core::copy::engine::{CopyFailure, CopyOutcome, CopyProgress, CopyReport};
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::preview::PreviewCounts;
use curlybrace_core::copy::protocol::{CopyItem, CopyRequest, CopyScope};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Namespace, WriteFailure};
use curlybrace_ui::I18n;
use curlybrace_ui::app::copy::CopyInput;
use curlybrace_ui::app::copy_state::{CopyDialog, CopyPhase, CopyRun, RunOutcome};
use curlybrace_ui::app::reducer::{Input, update};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::views::copy_dialog::show_copy_modal;
use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use uuid::Uuid;

const WORD: &str = "copy";

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn item(name: &str) -> CopyItem {
    CopyItem {
        source: Namespace::new("shop", name),
        target: Namespace::new("crm", name),
        filter: Document::new(),
    }
}

fn dialog(policy: ConflictPolicy) -> CopyDialog {
    let request = CopyRequest {
        source: Uuid::from_u128(1),
        target: Uuid::from_u128(2),
        scope: CopyScope::Collections(vec![item("orders"), item("users")]),
        policy,
        with_indexes_and_options: false,
    };
    CopyDialog::opened(request, WORD).with_policy(policy)
}

struct Run {
    app: AppState,
    inputs: Vec<CopyInput>,
}

fn harness(dialog: CopyDialog) -> Harness<'static, Run> {
    let app = AppState {
        copy: Some(dialog),
        ..AppState::default()
    };
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 1100.0))
        .build_state(
            |ctx, run: &mut Run| {
                let Some(input) = show_copy_modal(ctx, &run.app, &I18n::english()) else {
                    return;
                };
                run.inputs.push(input.clone());
                let (app, _) = update(std::mem::take(&mut run.app), Input::Copy(input));
                run.app = app;
            },
            Run {
                app,
                inputs: Vec::new(),
            },
        );
    harness.run_steps(5);
    harness
}

fn disabled(harness: &Harness<'_, Run>, label: &str) -> bool {
    harness
        .get_by_role_and_label(Role::Button, label)
        .accesskit_node()
        .is_disabled()
}

fn counted(dialog: CopyDialog) -> CopyDialog {
    let items = vec![item("orders"), item("users")];
    dialog
        .with_plan(items)
        .with_count(
            0,
            Some(PreviewCounts {
                new: 5,
                changed: 1,
                unchanged: 2,
            }),
        )
        .with_count(1, None)
}

#[test]
fn the_dialog_names_its_controls_and_shows_the_counts() {
    let h = harness(counted(dialog(ConflictPolicy::Skip)));
    h.get_by_label(&t("copy.dialog.title"));
    for key in [
        "copy.dialog.policy.skip",
        "copy.dialog.policy.overwrite",
        "copy.dialog.policy.upsert",
    ] {
        h.get_by_role_and_label(Role::RadioButton, &t(key));
    }
    let i18n = I18n::english();
    for source in ["orders", "users"] {
        h.get_by_label(&i18n.t_with("copy.dialog.name_label", &[("source", source)]));
    }
    assert_eq!(h.get_all_by_role(Role::TextInput).count(), 2);
    h.get_by_label(&i18n.t_with(
        "copy.dialog.counts.counted",
        &[
            ("collection", "orders"),
            ("new", "5"),
            ("changed", "1"),
            ("unchanged", "2"),
        ],
    ));
    // Unavailable counts say so and keep Copy enabled.
    h.get_by_label(&i18n.t_with("copy.dialog.counts.unavailable", &[("collection", "users")]));
    assert!(!disabled(&h, &t("copy.dialog.copy")));
    h.get_by_role_and_label(Role::CheckBox, &t("copy.dialog.indexes_and_options"));
}

#[test]
fn copy_waits_for_the_typed_word_when_overwriting() {
    let mut h = harness(counted(dialog(ConflictPolicy::Overwrite)));
    assert!(disabled(&h, &t("copy.dialog.copy")));
    h.get_by_label(&t("copy.dialog.blocked_typed"));
    h.get_by_label(&t("copy.dialog.confirm_prompt"));
    // Cancel has the initial focus, so Enter cannot confirm.
    let cancel = t("common.cancel");
    assert!(h.get_by_label(&cancel).accesskit_node().is_focused());
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(h.state().inputs, vec![CopyInput::Cancel]);
}

#[test]
fn typing_the_word_enables_copy_and_confirm_sends_it() {
    let mut h = harness(counted(dialog(ConflictPolicy::Overwrite)));
    let prompt = t("copy.dialog.confirm_prompt");
    let word_box = h.get_by_role_and_label(Role::TextInput, &prompt);
    word_box.focus();
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, &prompt)
        .type_text(WORD);
    h.run_steps(4);
    assert!(!disabled(&h, &t("copy.dialog.copy")));
    h.get_by_role_and_label(Role::Button, &t("copy.dialog.copy"))
        .click();
    h.run_steps(4);
    let inputs = &h.state().inputs;
    assert_eq!(inputs.last(), Some(&CopyInput::Confirm));
    assert!(inputs.contains(&CopyInput::TypeConfirmation(WORD.into())));
}

#[test]
fn a_bad_name_shows_its_reason_and_disables_copy() {
    let d = counted(dialog(ConflictPolicy::Skip)).with_name(0, "a$b".into());
    let h = harness(d);
    h.get_by_label(&t("copy.name_problem.forbidden_char"));
    h.get_by_label(&t("copy.dialog.blocked_names"));
    assert!(disabled(&h, &t("copy.dialog.copy")));
}

#[test]
fn escape_cancels_and_copy_confirms_when_nothing_else_is_needed() {
    let mut h = harness(counted(dialog(ConflictPolicy::Skip)));
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert_eq!(h.state().inputs, vec![CopyInput::Cancel]);
    let mut h = harness(counted(dialog(ConflictPolicy::Skip)));
    h.get_by_role_and_label(Role::Button, &t("copy.dialog.copy"))
        .click();
    h.run_steps(4);
    assert_eq!(h.state().inputs, vec![CopyInput::Confirm]);
}

#[test]
fn counting_says_copy_is_available_and_how_far_it_is() {
    let d = dialog(ConflictPolicy::Skip).with_plan(vec![item("orders"), item("users")]);
    let d = d.counting_run(1).with_scanned(0, 1500);
    let h = harness(d);
    h.get_by_label(&t("copy.dialog.counting_note"));
    h.get_by_label(&I18n::english().t_with(
        "copy.dialog.counts.counting",
        &[("collection", "orders"), ("scanned", "1500")],
    ));
    h.get_by_label(
        &I18n::english().t_with("copy.dialog.counts.waiting", &[("collection", "users")]),
    );
    assert!(!disabled(&h, &t("copy.dialog.copy")));
}

fn running(outcome: Option<RunOutcome>) -> CopyDialog {
    let progress = CopyProgress {
        scanned: 12_300,
        written: 12_300,
        skipped: 0,
        failed: 0,
    };
    let run = CopyRun::new(3, false)
        .with_plan(&[item("orders"), item("users")])
        .with_item_done(
            0,
            CopyReport {
                progress,
                failures: Vec::new(),
                outcome: CopyOutcome::Cancelled,
            },
        );
    let run = match outcome {
        Some(outcome) => run.ended(outcome),
        None => run,
    };
    dialog(ConflictPolicy::Skip).with_phase(CopyPhase::Running(run))
}

#[test]
fn a_running_copy_offers_cancel_once() {
    let mut h = harness(running(None));
    h.get_by_label(&t("copy.progress.title"));
    h.get_by_label(
        &I18n::english().t_with("copy.progress.row_waiting", &[("collection", "users")]),
    );
    h.get_by_role_and_label(Role::Button, &t("copy.progress.cancel"))
        .click();
    h.run_steps(4);
    assert_eq!(h.state().inputs, vec![CopyInput::Cancel]);
    assert!(
        disabled(&h, &t("copy.progress.cancel")),
        "a second cancel is off"
    );
    h.get_by_label(&t("copy.progress.cancelling"));
}

#[test]
fn a_cancelled_copy_states_exactly_what_was_written() {
    let mut h = harness(running(Some(RunOutcome::Cancelled)));
    let i18n = I18n::english();
    h.get_by_label(&t("copy.report.title_cancelled"));
    h.get_by_label(&i18n.t_with(
        "copy.report.written",
        &[("collection", "orders"), ("count", "12300")],
    ));
    h.get_by_label(&i18n.t_with("copy.report.not_started", &[("collection", "users")]));
    h.get_by_label(&t("copy.report.nothing_else"));
    // The report owns the focus, so Enter closes it.
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(h.state().inputs, vec![CopyInput::Close]);
}

#[test]
fn a_lost_connection_and_a_read_only_target_use_their_own_words() {
    let lost = harness(running(Some(RunOutcome::Failed(CoreError::Superseded))));
    lost.get_by_label(&t("copy.report.session_lost"));
    let locked = harness(running(Some(RunOutcome::Failed(CoreError::ReadOnly))));
    let reason = I18n::english().t_with("copy.report.error", &[("reason", &t("errors.read_only"))]);
    locked.get_by_label(&reason);
}

#[test]
fn the_failure_report_lists_keys_and_message_keys_only() {
    let failures = vec![CopyFailure {
        key: Bson::Int32(42),
        reason: WriteFailure::Rejected(121),
    }];
    let report = CopyReport {
        progress: CopyProgress {
            scanned: 3,
            written: 2,
            skipped: 0,
            failed: 1,
        },
        failures,
        outcome: CopyOutcome::Completed,
    };
    let run = CopyRun::new(3, true)
        .with_plan(&[item("orders")])
        .with_item_done(0, report)
        .ended(RunOutcome::Completed);
    let h = harness(dialog(ConflictPolicy::Skip).with_phase(CopyPhase::Running(run)));
    h.get_by_label(&t("copy.report.failures_title"));
    h.get_by_label("42");
    h.get_by_label(&t("copy.failure.rejected"));
    h.get_by_label(&t("copy.report.counts_incomplete"));
    assert!(h.query_by_label_contains("121").is_none(), "no server code");
}
