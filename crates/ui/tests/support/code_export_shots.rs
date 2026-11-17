//! The Copy as code dialog open over the Aggregate tab of `crm.customers`: the three-stage
//! pipeline of the other Aggregate screenshots, shown as Python with the default options.
use super::aggregate_shots::aggregate_tab;
use curlybrace_core::codegen::{CodeRequest, Language};
use curlybrace_core::pipeline_options::{
    Alternate, CaseFirst, CollationOption, HintOption, PipelineOptions, ReadPreferenceMode,
    Strength,
};
use curlybrace_core::store::Namespace;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::CodeExportUi;
use curlybrace_ui::viewmodel::code_export::aggregate_stages;

/// The same dialog as Node.js code for a pipeline that sets every option (the nightly report of
/// the code goldens, without the bidi character).
pub fn code_export_dialog_options(state: &mut AppState) {
    aggregate_tab(state);
    let stages = aggregate_stages(&state.aggregate).expect("the fixture pipeline is valid");
    let options = PipelineOptions {
        collation: Some(CollationOption {
            locale: "en".into(),
            strength: Some(Strength::Tertiary),
            case_level: Some(false),
            case_first: Some(CaseFirst::Upper),
            numeric_ordering: Some(true),
            alternate: Some(Alternate::Shifted),
        }),
        hint: Some(HintOption::Name("status_1_total_-1".into())),
        allow_disk_use: Some(true),
        max_time_ms: Some(5000),
        comment: Some("nightly report".into()),
        read_preference: Some(ReadPreferenceMode::SecondaryPreferred),
    };
    let request = CodeRequest::Aggregate {
        ns: Namespace::new("crm", "customers"),
        stages,
        options,
    };
    state.code_export = Box::new(
        CodeExportUi::default()
            .opened(request)
            .language_set(Language::Node),
    );
}

pub fn code_export_dialog(state: &mut AppState) {
    aggregate_tab(state);
    let stages = aggregate_stages(&state.aggregate).expect("the fixture pipeline is valid");
    let request = CodeRequest::Aggregate {
        ns: Namespace::new("crm", "customers"),
        stages,
        options: PipelineOptions::default(),
    };
    state.code_export = Box::new(
        CodeExportUi::default()
            .opened(request)
            .language_set(Language::Python),
    );
}
