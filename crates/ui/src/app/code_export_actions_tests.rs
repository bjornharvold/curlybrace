//! The Copy as code dialog in the reducer: opened from the Aggregate tab and from the query bar
//! (with no connection at all), the code regenerated on every change, an unusable name shown in
//! place of the code, Copy offering exactly the previewed text, and Close.
use super::{CodeAction, CodeSource};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, update};
use crate::app::state::AppState;
use crate::viewmodel::{FindAction, NameField};
use bson::doc;
use curlybrace_core::codegen::{CodeOptions, CodeRequest, CodegenError, Language, generate};
use curlybrace_core::query::parse_query;
use curlybrace_core::runtime::Command;
use curlybrace_core::store::Namespace;

fn ns() -> Namespace {
    Namespace::new("app", "people")
}

/// `app.people` selected and no connection: the dialog never needs one.
fn offline() -> AppState {
    let mut s = AppState::new(vec![]);
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["people".into()])
        .toggled("app")
        .selecting(ns());
    s
}

fn with_pipeline(s: AppState) -> AppState {
    let draft = (*s.aggregate)
        .clone()
        .add_stage("$match")
        .edit(0, r#"{ "$match": { "status": "active" } }"#);
    AppState {
        aggregate: Box::new(draft),
        ..s
    }
}

fn with_filter(mut s: AppState, filter: &str) -> AppState {
    s.find.text.filter = filter.into();
    s
}

fn act(s: AppState, action: CodeAction) -> AppState {
    let (next, commands) = update(s, Input::Aggregate(AggregateAction::Code(action)));
    assert!(commands.is_empty(), "the dialog never talks to the server");
    next
}

fn opened_aggregate() -> AppState {
    act(
        with_pipeline(offline()),
        CodeAction::Open(CodeSource::Aggregate),
    )
}

fn opened_find() -> AppState {
    let s = with_filter(offline(), r#"{ "age": { "$gt": 30 } }"#);
    act(s, CodeAction::Open(CodeSource::Find))
}

#[test]
fn it_opens_from_the_aggregate_tab_with_the_enabled_stages() {
    let s = opened_aggregate();
    let dialog = s.code_export.dialog.as_ref().expect("open");
    assert_eq!(
        dialog.request,
        CodeRequest::Aggregate {
            ns: ns(),
            stages: vec![doc! { "$match": { "status": "active" } }],
        }
    );
    assert_eq!(dialog.language, Language::Mongosh);
    assert!(dialog.text.as_ref().unwrap().contains("status"));
}

#[test]
fn it_opens_from_the_query_bar_with_the_query_find_would_send() {
    let s = opened_find();
    let dialog = s.code_export.dialog.as_ref().expect("open");
    let sent = parse_query(&s.find.text).unwrap();
    assert_eq!(
        dialog.request,
        CodeRequest::Find {
            ns: ns(),
            query: sent
        }
    );
}

#[test]
fn the_entry_actions_of_both_tabs_open_it_without_a_command() {
    let (s, commands) = update(
        with_pipeline(offline()),
        Input::Aggregate(AggregateAction::Code(CodeAction::Open(
            CodeSource::Aggregate,
        ))),
    );
    assert!(commands.is_empty());
    assert!(s.code_export.dialog.is_some());
    let (s, commands) = update(
        with_filter(offline(), "{}"),
        Input::Find(FindAction::CopyAsCode),
    );
    assert!(commands.is_empty());
    assert!(s.code_export.dialog.is_some());
}

#[test]
fn the_entry_action_works_inside_a_frame_batch() {
    let (s, commands) = update(
        with_pipeline(offline()),
        Input::Aggregate(AggregateAction::Batch(vec![AggregateAction::Code(
            CodeAction::Open(CodeSource::Aggregate),
        )])),
    );
    assert!(s.code_export.dialog.is_some());
    assert!(!commands.iter().any(|c| matches!(c, Command::Find { .. })));
}

#[test]
fn it_stays_closed_when_there_is_nothing_valid_to_show() {
    let no_stage = act(offline(), CodeAction::Open(CodeSource::Aggregate));
    assert_eq!(no_stage.code_export.dialog, None);
    let bad_query = act(
        with_filter(offline(), "{ not json"),
        CodeAction::Open(CodeSource::Find),
    );
    assert_eq!(bad_query.code_export.dialog, None);
    let no_collection = act(
        with_filter(AppState::new(vec![]), "{}"),
        CodeAction::Open(CodeSource::Find),
    );
    assert_eq!(no_collection.code_export.dialog, None);
}

#[test]
fn switching_the_language_regenerates_the_code() {
    let s = opened_aggregate();
    let before = s.code_export.clipboard_text().unwrap().to_string();
    let s = act(s, CodeAction::SetLanguage(Language::Python));
    let after = s.code_export.clipboard_text().unwrap();
    assert_ne!(before, after);
    let request = s.code_export.dialog.as_ref().unwrap().request.clone();
    let expected = generate(Language::Python, &request, &CodeOptions::default()).unwrap();
    assert_eq!(after, expected);
}

#[test]
fn the_setup_option_and_the_variable_names_change_the_code() {
    let s = act(
        opened_find(),
        CodeAction::SetName(NameField::Collection, "people".into()),
    );
    assert!(s.code_export.clipboard_text().unwrap().contains("people"));
    let s = act(s, CodeAction::SetSetup(false));
    let bare = s.code_export.clipboard_text().unwrap();
    assert!(!bare.contains("MongoClient"), "{bare}");
}

#[test]
fn the_write_stage_option_applies_to_an_aggregate() {
    let draft = (*offline().aggregate)
        .clone()
        .add_stage("$match")
        .add_stage("$out")
        .edit(1, r#"{ "$out": "totals" }"#);
    let s = AppState {
        aggregate: Box::new(draft),
        ..offline()
    };
    let s = act(s, CodeAction::Open(CodeSource::Aggregate));
    assert!(s.code_export.clipboard_text().unwrap().contains("$out"));
    let s = act(s, CodeAction::SetWriteStage(false));
    assert!(!s.code_export.clipboard_text().unwrap().contains("$out"));
}

#[test]
fn an_invalid_name_shows_its_error_and_leaves_nothing_to_copy() {
    let s = act(
        opened_find(),
        CodeAction::SetName(NameField::Client, "not valid".into()),
    );
    let dialog = s.code_export.dialog.as_ref().unwrap();
    assert_eq!(dialog.text, Err(CodegenError::InvalidName("client")));
    assert_eq!(s.code_export.clipboard_text(), None);
    let fixed = act(s, CodeAction::SetName(NameField::Client, "conn".into()));
    assert!(fixed.code_export.clipboard_text().is_some());
}

#[test]
fn copy_offers_exactly_the_previewed_text_and_keeps_the_dialog_open() {
    let s = act(opened_aggregate(), CodeAction::SetLanguage(Language::Go));
    let previewed = s.code_export.dialog.as_ref().unwrap().text.clone().unwrap();
    assert_eq!(s.code_export.clipboard_text(), Some(previewed.as_str()));
    let copied = act(s, CodeAction::Copy);
    assert_eq!(
        copied.code_export.clipboard_text(),
        Some(previewed.as_str())
    );
    assert!(copied.code_export.dialog.is_some());
}

#[test]
fn close_ends_the_dialog_and_the_language_is_remembered_for_the_next_one() {
    let s = act(opened_find(), CodeAction::SetLanguage(Language::Rust));
    let s = act(s, CodeAction::SetSetup(false));
    let closed = act(s, CodeAction::Close);
    assert_eq!(closed.code_export.dialog, None);
    let again = act(closed, CodeAction::Open(CodeSource::Find));
    let dialog = again.code_export.dialog.unwrap();
    assert_eq!(dialog.language, Language::Rust);
    assert!(!dialog.options.setup);
}
