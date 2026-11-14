use super::*;
use bson::doc;
use curlybrace_core::query::{QueryText, parse_query};
use curlybrace_core::store::Namespace;

const MATCH: &str = r#"{ "$match": { "status": "active" } }"#;
const GROUP: &str = r#"{ "$group": { "_id": "$city", "n": { "$sum": 1 } } }"#;
const OUT: &str = r#"{ "$out": "city_totals" }"#;

fn draft(stages: &[(&str, &str)]) -> AggregateState {
    stages
        .iter()
        .fold(AggregateState::default(), |draft, (name, text)| {
            let at = draft.stages.len();
            draft.add_stage(name).edit(at, text)
        })
}

fn ns() -> Namespace {
    Namespace::new("crm", "customers")
}

fn aggregate_request() -> CodeRequest {
    CodeRequest::Aggregate {
        ns: ns(),
        stages: aggregate_stages(&draft(&[("$match", MATCH), ("$group", GROUP)])).unwrap(),
    }
}

fn find_request() -> CodeRequest {
    CodeRequest::Find {
        ns: ns(),
        query: parse_query(&QueryText {
            filter: r#"{ "a": 1 }"#.into(),
            ..QueryText::default()
        })
        .unwrap(),
    }
}

#[test]
fn the_stages_are_the_enabled_ones_without_the_preview_limit() {
    let stages = aggregate_stages(&draft(&[("$match", MATCH), ("$group", GROUP)])).unwrap();
    assert_eq!(
        stages,
        vec![
            doc! { "$match": { "status": "active" } },
            doc! { "$group": { "_id": "$city", "n": { "$sum": 1 } } },
        ]
    );
}

#[test]
fn a_disabled_stage_is_left_out() {
    let toggled = draft(&[("$match", MATCH), ("$group", GROUP)]).toggle(0);
    let stages = aggregate_stages(&toggled).unwrap();
    assert_eq!(stages.len(), 1);
    assert!(stages[0].contains_key("$group"));
}

#[test]
fn a_trailing_write_stage_is_kept_as_the_run_plan_sends_it() {
    let stages = aggregate_stages(&draft(&[("$match", MATCH), ("$out", OUT)])).unwrap();
    assert_eq!(stages.last(), Some(&doc! { "$out": "city_totals" }));
    assert_eq!(stages.len(), 2);
}

#[test]
fn a_stage_that_does_not_parse_blocks_with_its_position() {
    let broken = draft(&[("$match", MATCH), ("$group", "{ not json")]);
    assert_eq!(
        aggregate_stages(&broken),
        Err(CodeBlock::Pipeline(PipelineError::NotAStage { index: 1 }))
    );
}

#[test]
fn no_enabled_stage_blocks_as_empty() {
    assert_eq!(
        aggregate_stages(&AggregateState::default()),
        Err(CodeBlock::Pipeline(PipelineError::Empty))
    );
    let all_off = draft(&[("$match", MATCH)]).toggle(0);
    assert_eq!(
        aggregate_stages(&all_off),
        Err(CodeBlock::Pipeline(PipelineError::Empty))
    );
}

#[test]
fn a_write_stage_that_is_not_last_blocks() {
    let misplaced = draft(&[("$out", OUT), ("$match", MATCH)]);
    assert!(matches!(
        aggregate_stages(&misplaced),
        Err(CodeBlock::Pipeline(PipelineError::WriteStageNotLast { .. }))
    ));
}

#[test]
fn edited_text_that_is_not_applied_blocks() {
    let text_mode = draft(&[("$match", MATCH)]).to_text();
    assert!(
        aggregate_stages(&text_mode).is_ok(),
        "untouched text is the stages"
    );
    let edited = text_mode.with_text("[ { \"$match\": {} } ]");
    assert_eq!(aggregate_stages(&edited), Err(CodeBlock::TextNotApplied));
}

#[test]
fn opening_uses_the_remembered_language_and_options() {
    let ui = CodeExportUi::default().opened(find_request());
    assert_eq!(ui.dialog.as_ref().unwrap().language, Language::Mongosh);
    let ui = ui.language_set(Language::Python).setup_set(false).closed();
    let again = ui.opened(find_request());
    let dialog = again.dialog.unwrap();
    assert_eq!(dialog.language, Language::Python);
    assert!(!dialog.options.setup);
}

#[test]
fn switching_the_language_regenerates_the_text() {
    let ui = CodeExportUi::default().opened(aggregate_request());
    let before = ui.clipboard_text().unwrap().to_string();
    let ui = ui.language_set(Language::Java);
    let after = ui.clipboard_text().unwrap();
    assert_ne!(before, after);
    let expected = generate(
        Language::Java,
        &aggregate_request(),
        &CodeOptions::default(),
    )
    .unwrap();
    assert_eq!(after, expected);
}

#[test]
fn an_unusable_name_replaces_the_code_with_its_error() {
    let ui = CodeExportUi::default()
        .opened(find_request())
        .name_set(NameField::Db, "1bad name".into());
    let dialog = ui.dialog.as_ref().unwrap();
    assert_eq!(dialog.text, Err(CodegenError::InvalidName("db")));
    assert_eq!(ui.clipboard_text(), None);
}

#[test]
fn names_that_are_not_usable_are_not_remembered_but_other_options_are() {
    let ui = CodeExportUi::default()
        .opened(find_request())
        .name_set(NameField::Client, "class".into())
        .setup_set(false);
    assert_eq!(ui.last.1.client_var, "client");
    assert!(!ui.last.1.setup);
    let fixed = ui.name_set(NameField::Client, "mongo".into());
    assert_eq!(fixed.last.1.client_var, "mongo");
}

#[test]
fn the_write_stage_option_changes_the_aggregate_text_only() {
    let request = CodeRequest::Aggregate {
        ns: ns(),
        stages: aggregate_stages(&draft(&[("$match", MATCH), ("$out", OUT)])).unwrap(),
    };
    let on = CodeExportUi::default().opened(request);
    assert!(on.clipboard_text().unwrap().contains("$out"));
    let off = on.write_stage_set(false);
    assert!(!off.clipboard_text().unwrap().contains("$out"));
    assert!(off.dialog.unwrap().is_aggregate());
    assert!(
        !CodeExportUi::default()
            .opened(find_request())
            .dialog
            .unwrap()
            .is_aggregate()
    );
}

#[test]
fn edits_without_a_dialog_change_nothing() {
    let ui = CodeExportUi::default();
    assert_eq!(ui.clone().language_set(Language::Go), ui);
    assert_eq!(ui.clone().setup_set(false), ui);
}

#[test]
fn debug_never_prints_the_request_or_the_code() {
    let ui = CodeExportUi::default().opened(find_request());
    let shown = format!("{ui:?}");
    assert!(!shown.contains("crm"), "{shown}");
    assert!(!shown.contains("find("), "{shown}");
}
