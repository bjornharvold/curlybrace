use super::*;
use crate::viewmodel::code_export::CodeExportUi;
use bson::doc;
use curlybrace_core::codegen::CodeRequest;
use curlybrace_core::pipeline_options::{
    CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
};
use curlybrace_core::store::{Namespace, Query};

fn ns() -> Namespace {
    Namespace::new("crm", "customers")
}

fn aggregate(stages: Vec<bson::Document>, options: PipelineOptions) -> CodeRequest {
    CodeRequest::Aggregate {
        ns: ns(),
        stages,
        options,
    }
}

fn read_stages() -> Vec<bson::Document> {
    vec![doc! { "$match": { "status": "active" } }]
}

fn write_stages() -> Vec<bson::Document> {
    vec![
        doc! { "$match": { "status": "active" } },
        doc! { "$out": "totals" },
    ]
}

fn every_option() -> PipelineOptions {
    PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: None,
            case_level: None,
            case_first: None,
            numeric_ordering: None,
            alternate: None,
        }),
        hint: Some(HintOption::Name("status_1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some("nightly".into()),
        read_preference: Some(ReadPreferenceMode::Secondary),
    }
}

fn dialog(request: CodeRequest) -> CodeDialog {
    CodeExportUi::default()
        .opened(request)
        .dialog
        .expect("open")
}

#[test]
fn no_options_no_view() {
    let d = dialog(aggregate(read_stages(), PipelineOptions::default()));
    assert_eq!(code_options_view(&d), None);
}

#[test]
fn find_code_has_no_options_view() {
    let find = CodeRequest::Find {
        ns: ns(),
        query: Query::default(),
    };
    assert_eq!(code_options_view(&dialog(find)), None);
}

#[test]
fn the_labels_follow_what_is_set_in_order() {
    let d = dialog(aggregate(read_stages(), every_option()));
    let view = code_options_view(&d).expect("a view");
    assert!(view.include);
    assert_eq!(
        view.labels,
        vec![
            "aggregate.options.collation.enabled",
            "aggregate.options.hint.label",
            "aggregate.options.allow_disk_use.label",
            "aggregate.options.max_time.label",
            "aggregate.options.comment.label",
            "aggregate.options.read_preference.label",
        ]
    );
    assert!(!view.read_preference_left_out);
    let some = PipelineOptions {
        comment: Some("x".into()),
        max_time_ms: Some(10),
        ..PipelineOptions::default()
    };
    let view = code_options_view(&dialog(aggregate(read_stages(), some))).expect("a view");
    assert_eq!(
        view.labels,
        vec![
            "aggregate.options.max_time.label",
            "aggregate.options.comment.label"
        ]
    );
}

#[test]
fn include_off_lists_nothing() {
    let ui = CodeExportUi::default()
        .opened(aggregate(read_stages(), every_option()))
        .include_options_set(false);
    let view = code_options_view(ui.dialog.as_ref().unwrap()).expect("the box stays");
    assert!(!view.include);
    assert!(view.labels.is_empty());
    assert!(!view.read_preference_left_out);
    assert!(!ui.clipboard_text().unwrap().contains("nightly"));
}

#[test]
fn a_kept_write_stage_leaves_the_read_preference_out_of_the_labels_and_says_so() {
    let ui = CodeExportUi::default().opened(aggregate(write_stages(), every_option()));
    let view = code_options_view(ui.dialog.as_ref().unwrap()).expect("a view");
    assert!(view.read_preference_left_out);
    assert!(
        !view
            .labels
            .contains(&"aggregate.options.read_preference.label")
    );
    assert_eq!(view.labels.len(), 5);
    // Dropping the write stage keeps the read preference again.
    let ui = ui.write_stage_set(false);
    let view = code_options_view(ui.dialog.as_ref().unwrap()).expect("a view");
    assert!(!view.read_preference_left_out);
    assert!(
        view.labels
            .contains(&"aggregate.options.read_preference.label")
    );
}

#[test]
fn include_options_is_remembered_for_the_next_dialog() {
    let ui = CodeExportUi::default()
        .opened(aggregate(read_stages(), every_option()))
        .include_options_set(false)
        .closed();
    assert!(!ui.last.1.include_options);
    let again = ui.opened(aggregate(read_stages(), every_option()));
    assert!(!again.dialog.as_ref().unwrap().options.include_options);
    // An invalid name edit does not lose the choice.
    let edited = again.name_set(crate::viewmodel::NameField::Collection, "class".into());
    assert!(!edited.last.1.include_options);
    assert!(!edited.dialog.as_ref().unwrap().options.include_options);
}
