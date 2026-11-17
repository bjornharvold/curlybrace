//! The draft's options in the Copy as code dialog: carried into the request, rendered in the
//! code, switched off by the "Include options" check, and refused while the form has a problem.
use super::{CodeAction, CodeSource};
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::app::{AggregateAction, OptionsAction};
use crate::viewmodel::aggregate_options::{OptionsEdit, Tri};
use curlybrace_core::codegen::{CodeRequest, Language};
use curlybrace_core::pipeline_options::PipelineOptions;
use curlybrace_core::store::Namespace;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

fn offline() -> AppState {
    let mut s = AppState::new(vec![]);
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(ns());
    let draft = (*s.aggregate)
        .clone()
        .add_stage("$match")
        .edit(0, r#"{ "$match": { "status": "active" } }"#);
    AppState {
        aggregate: Box::new(draft),
        ..s
    }
}

/// Runs the action; the commands (a preview with the new options) are not what is tested here.
fn aggregate(s: AppState, action: AggregateAction) -> AppState {
    update(s, Input::Aggregate(action)).0
}

fn edit(s: AppState, edit: OptionsEdit) -> AppState {
    aggregate(s, AggregateAction::Options(OptionsAction::Edit(edit)))
}

/// A draft with a time limit and a disk-use flag applied.
fn with_options() -> AppState {
    let s = edit(offline(), OptionsEdit::AllowDiskUse(Tri::On));
    let s = edit(s, OptionsEdit::MaxTime("5000".into()));
    aggregate(s, AggregateAction::Options(OptionsAction::Apply))
}

fn code(s: AppState, action: CodeAction) -> AppState {
    aggregate(s, AggregateAction::Code(action))
}

fn open(s: AppState) -> AppState {
    code(s, CodeAction::Open(CodeSource::Aggregate))
}

#[test]
fn opening_from_the_aggregate_tab_carries_the_draft_options() {
    let s = open(with_options());
    let expected = PipelineOptions {
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        ..PipelineOptions::default()
    };
    let dialog = s.code_export.dialog.as_ref().expect("open");
    let CodeRequest::Aggregate { options, .. } = &dialog.request else {
        panic!("an aggregate request");
    };
    assert_eq!(*options, expected);
}

#[test]
fn the_code_contains_the_options() {
    let s = code(
        open(with_options()),
        CodeAction::SetLanguage(Language::Node),
    );
    let text = s.code_export.clipboard_text().expect("code");
    assert!(text.contains("maxTimeMS: 5000"), "{text}");
    assert!(text.contains("allowDiskUse: true"), "{text}");
}

#[test]
fn unchecking_include_options_regenerates_without_them() {
    let s = code(
        open(with_options()),
        CodeAction::SetLanguage(Language::Node),
    );
    let s = code(s, CodeAction::SetIncludeOptions(false));
    let text = s.code_export.clipboard_text().expect("code");
    assert!(!text.contains("maxTimeMS"), "{text}");
    let s = code(s, CodeAction::SetIncludeOptions(true));
    assert!(
        s.code_export
            .clipboard_text()
            .unwrap()
            .contains("maxTimeMS")
    );
}

#[test]
fn copy_puts_the_previewed_text_with_options_on_the_clipboard_output() {
    let s = code(
        open(with_options()),
        CodeAction::SetLanguage(Language::Python),
    );
    let before = s.code_export.clipboard_text().map(str::to_string);
    let s = code(s, CodeAction::Copy);
    let dialog = s.code_export.dialog.as_ref().expect("still open");
    assert_eq!(dialog.clipboard_text().map(str::to_string), before);
    assert!(before.unwrap().contains("maxTimeMS=5000"));
}

#[test]
fn a_blocked_form_opens_no_dialog() {
    let s = edit(offline(), OptionsEdit::MaxTime("0".into()));
    assert!(s.aggregate.options_blocked());
    let s = open(s);
    assert!(s.code_export.dialog.is_none());
}
