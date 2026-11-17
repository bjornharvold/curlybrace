//! The "Include options" check box and the summary of the options in the Copy as code dialog.
use crate::I18n;
use crate::app::CodeAction;
use crate::theme;
use crate::viewmodel::code_export::CodeExportUi;
use crate::views::code_export::show_code_dialog;
use curlybrace_core::codegen::CodeRequest;
use curlybrace_core::pipeline_options::{PipelineOptions, ReadPreferenceMode};
use curlybrace_core::store::Namespace;
use eframe::egui::{self, accesskit::Role};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

struct Frame {
    code: CodeExportUi,
    seen: Vec<CodeAction>,
    copied: Vec<String>,
}

fn t(key: &str) -> String {
    I18n::english().t(key)
}

fn request(stages: Vec<bson::Document>, options: PipelineOptions) -> CodeRequest {
    CodeRequest::Aggregate {
        ns: Namespace::new("crm", "customers"),
        stages,
        options,
    }
}

fn reads() -> Vec<bson::Document> {
    vec![bson::doc! { "$match": { "status": "active" } }]
}

fn writes() -> Vec<bson::Document> {
    vec![
        bson::doc! { "$match": { "status": "active" } },
        bson::doc! { "$out": "totals" },
    ]
}

fn options() -> PipelineOptions {
    PipelineOptions {
        max_time_ms: Some(5000),
        comment: Some("nightly".into()),
        read_preference: Some(ReadPreferenceMode::Secondary),
        ..PipelineOptions::default()
    }
}

fn harness(request: CodeRequest) -> Harness<'static, Frame> {
    let frame = Frame {
        code: CodeExportUi::default().opened(request),
        seen: Vec::new(),
        copied: Vec::new(),
    };
    let mut installed = false;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_ui_state(
            move |ui, f: &mut Frame| {
                if !installed {
                    theme::install(ui.ctx());
                    installed = true;
                }
                let actions = show_code_dialog(ui.ctx(), &f.code, &I18n::english());
                let copied = ui.ctx().output(|o| {
                    o.commands
                        .iter()
                        .filter_map(|c| match c {
                            egui::OutputCommand::CopyText(text) => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                });
                f.copied.extend(copied);
                for action in actions {
                    if let CodeAction::SetIncludeOptions(include) = action {
                        f.code = f.code.clone().include_options_set(include);
                    }
                    f.seen.push(action);
                }
            },
            frame,
        );
    h.run_steps(4);
    h
}

#[test]
fn the_include_options_box_shows_only_when_options_are_set() {
    let h = harness(request(reads(), options()));
    h.get_by_role_and_label(Role::CheckBox, &t("codegen.options.include_options"));
    let none = harness(request(reads(), PipelineOptions::default()));
    assert!(
        none.query_by_role_and_label(Role::CheckBox, &t("codegen.options.include_options"))
            .is_none()
    );
}

#[test]
fn toggling_it_emits_set_include_options() {
    let mut h = harness(request(reads(), options()));
    h.get_by_role_and_label(Role::CheckBox, &t("codegen.options.include_options"))
        .click();
    h.run_steps(4);
    assert_eq!(h.state().seen, [CodeAction::SetIncludeOptions(false)]);
}

#[test]
fn enter_on_the_box_does_not_copy() {
    let mut h = harness(request(reads(), options()));
    h.get_by_role_and_label(Role::CheckBox, &t("codegen.options.include_options"))
        .focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().copied.is_empty(), "{:?}", h.state().copied);
}

#[test]
fn the_summary_names_the_options_in_words() {
    let h = harness(request(reads(), options()));
    let names = [
        t("aggregate.options.max_time.label"),
        t("aggregate.options.comment.label"),
        t("aggregate.options.read_preference.label"),
    ]
    .join(&t("codegen.options.separator"));
    let listed = I18n::english().t_with("codegen.options.listed", &[("options", &names)]);
    h.get_by_label(&listed);
}

#[test]
fn the_left_out_read_preference_is_explained() {
    let h = harness(request(writes(), options()));
    h.get_by_label(&t("codegen.options.read_preference_left_out"));
    let names = [
        t("aggregate.options.max_time.label"),
        t("aggregate.options.comment.label"),
    ]
    .join(&t("codegen.options.separator"));
    let listed = I18n::english().t_with("codegen.options.listed", &[("options", &names)]);
    h.get_by_label(&listed);
    let reads_only = harness(request(reads(), options()));
    assert!(
        reads_only
            .query_by_label(&t("codegen.options.read_preference_left_out"))
            .is_none()
    );
}
