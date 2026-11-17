//! What the pane and the run dialog say about the options: the status lines under the preview,
//! the block that keeps the last result, the timeout under a limit the person set, and what a
//! run lists.
use super::tests::{base, open, t};
use crate::I18n;
use crate::theme;
use crate::viewmodel::ConnectionBadge;
use crate::viewmodel::aggregate::{AggregateState, Preview, PreviewError, PreviewState};
use crate::viewmodel::aggregate_explain::{ExplainState, ExplainUi, ResultTab};
use crate::viewmodel::aggregate_options::OptionsEdit;
use crate::viewmodel::aggregate_run::{CountState, RunDialog};
use crate::views::aggregate_run::show_run_dialog;
use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::pipeline::{PipelineOptions, ReadPreferenceMode, RunTarget};
use curlybrace_core::runtime::COMMAND_TIMEOUT_MESSAGE;
use curlybrace_core::store::Namespace;
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

const SIZE: (f32, f32) = (1000.0, 700.0);

fn applied(draft: AggregateState, edit: OptionsEdit) -> AggregateState {
    draft.options_edited(edit).options_applied().0
}

fn shown_with(options: Vec<OptionsEdit>) -> AggregateState {
    let mut draft = options.into_iter().fold(base(), applied);
    draft.preview = PreviewState::Shown(Preview {
        docs: vec![doc! { "a": 1 }],
        not_run: Vec::new(),
    });
    draft
}

#[test]
fn the_status_line_names_the_read_preference_and_the_hint() {
    let draft = shown_with(vec![
        OptionsEdit::ReadPreference(Some(ReadPreferenceMode::SecondaryPreferred)),
        OptionsEdit::HintText("status_1".into()),
    ]);
    let harness = open(draft, SIZE);
    harness.get_by_label("Read from secondaryPreferred");
    harness.get_by_label("hint: status_1");
    let keys = shown_with(vec![OptionsEdit::HintText(r#"{"a": 1, "b": -1}"#.into())]);
    let harness = open(keys, SIZE);
    harness.get_by_label(r#"hint: {"a":1,"b":-1}"#);
}

#[test]
fn no_option_no_status_line() {
    let harness = open(shown_with(Vec::new()), SIZE);
    assert!(harness.query_by_label_contains("Read from").is_none());
    assert!(harness.query_by_label_contains("hint:").is_none());
}

#[test]
fn a_blocked_preview_says_why_and_keeps_the_last_result() {
    let blocked = shown_with(Vec::new())
        .options_edited(OptionsEdit::Comment("   ".into()))
        .preview_blocked();
    assert!(matches!(
        blocked.preview,
        PreviewState::Failed {
            error: PreviewError::Options,
            ..
        }
    ));
    let harness = open(blocked, SIZE);
    // The same sentence heads the buttons' reasons and the pane: it is on screen.
    assert!(
        harness
            .query_all_by_label(&t("aggregate.options.blocked"))
            .count()
            >= 1
    );
    harness.get_by_label(&t("aggregate.preview.kept"));
}

fn timed_out(draft: AggregateState) -> AggregateState {
    let mut draft = draft;
    draft.preview = PreviewState::Failed {
        error: PreviewError::Core(CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into())),
        last: None,
    };
    draft
}

#[test]
fn a_timeout_under_a_lowered_limit_says_you_set_it() {
    let lowered = timed_out(applied(base(), OptionsEdit::MaxTime("500".into())));
    let harness = open(lowered, SIZE);
    harness.get_by_label(&t("aggregate.options.timeout_set"));
    let unchanged = timed_out(applied(base(), OptionsEdit::MaxTime("10000".into())));
    let harness = open(unchanged, SIZE);
    harness.get_by_label(&t("aggregate.error.timeout"));
    let default = open(timed_out(base()), SIZE);
    default.get_by_label(&t("aggregate.error.timeout"));
}

#[test]
fn a_timeout_of_the_explain_says_so_too() {
    let plan_tab = |draft: AggregateState| {
        let mut draft = draft;
        draft.explain = ExplainUi {
            state: ExplainState::Failed {
                reason: CoreError::Database(COMMAND_TIMEOUT_MESSAGE.into()),
            },
            tab: ResultTab::Plan,
            ..ExplainUi::default()
        };
        draft
    };
    let lowered = plan_tab(applied(base(), OptionsEdit::MaxTime("500".into())));
    let harness = open(lowered, SIZE);
    harness.get_by_label(&t("aggregate.options.timeout_set"));
    let harness = open(plan_tab(base()), SIZE);
    harness.get_by_label(&t("aggregate.explain.error.timeout"));
}

struct Dialog {
    dialog: RunDialog,
    options: PipelineOptions,
}

fn run_dialog(options: PipelineOptions) -> Harness<'static, Dialog> {
    let dialog = RunDialog {
        target: RunTarget {
            db: None,
            collection: "dest".into(),
        },
        stage: "$out".into(),
        source: Namespace::new("app", "people"),
        typed: String::new(),
        count: CountState::Loading,
    };
    let mut installed = false;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            move |ui, state: &mut Dialog| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let _ = show_run_dialog(
                    ui.ctx(),
                    Some(&state.dialog),
                    &state.options,
                    &ConnectionBadge::default(),
                    &I18n::english(),
                );
            },
            Dialog { dialog, options },
        );
    harness.run_steps(4);
    harness
}

#[test]
fn the_run_dialog_lists_the_options_it_sends() {
    let options = PipelineOptions {
        comment: Some("c".into()),
        allow_disk_use: Some(true),
        read_preference: Some(ReadPreferenceMode::Nearest),
        ..PipelineOptions::default()
    };
    let harness = run_dialog(options);
    harness.get_by_label_contains("This run sends these options: ");
    harness.get_by_label_contains(&t("aggregate.options.comment.label"));
    let none = run_dialog(PipelineOptions::default());
    assert!(
        none.query_by_label_contains("sends these options")
            .is_none()
    );
}

#[test]
fn the_run_dialog_states_the_primary_rule_only_for_a_non_primary_read_preference() {
    let nearest = run_dialog(PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Nearest),
        ..PipelineOptions::default()
    });
    nearest.get_by_label(&t("aggregate.options.run.primary"));
    let primary = run_dialog(PipelineOptions {
        read_preference: Some(ReadPreferenceMode::Primary),
        ..PipelineOptions::default()
    });
    assert!(
        primary
            .query_by_label(&t("aggregate.options.run.primary"))
            .is_none()
    );
    let unset = run_dialog(PipelineOptions::default());
    assert!(
        unset
            .query_by_label(&t("aggregate.options.run.primary"))
            .is_none()
    );
}
