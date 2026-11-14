//! The Copy as code dialog open over the Aggregate tab of `crm.customers`: the three-stage
//! pipeline of the other Aggregate screenshots, shown as Python with the default options.
use super::aggregate_shots::aggregate_tab;
use curlybrace_core::codegen::{CodeRequest, Language};
use curlybrace_core::store::Namespace;
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::CodeExportUi;
use curlybrace_ui::viewmodel::code_export::aggregate_stages;

pub fn code_export_dialog(state: &mut AppState) {
    aggregate_tab(state);
    let stages = aggregate_stages(&state.aggregate).expect("the fixture pipeline is valid");
    let request = CodeRequest::Aggregate {
        ns: Namespace::new("crm", "customers"),
        stages,
    };
    state.code_export = Box::new(
        CodeExportUi::default()
            .opened(request)
            .language_set(Language::Python),
    );
}
