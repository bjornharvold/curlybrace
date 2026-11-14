//! The Copy as code dialog's handlers. Pure and offline: opening it reads the draft or the query
//! bar of the open collection, and nothing is ever sent to the server, so it works without a
//! connection. Copy itself is the view's (the clipboard is egui's); the reducer only keeps the
//! dialog open.
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::NameField;
use crate::viewmodel::code_export::aggregate_stages;
use curlybrace_core::codegen::{CodeRequest, Language};
use curlybrace_core::query::parse_query;

/// Where the code comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSource {
    /// The enabled stages of the Aggregate tab's draft.
    Aggregate,
    /// The query the Find button would send.
    Find,
}

/// What the person does with the Copy as code dialog.
#[derive(Clone, Debug, PartialEq)]
pub enum CodeAction {
    Open(CodeSource),
    SetLanguage(Language),
    SetSetup(bool),
    SetName(NameField, String),
    SetWriteStage(bool),
    /// The view put the previewed text on the clipboard; the dialog stays open.
    Copy,
    Close,
}

/// The request `source` stands for in `state`, if it can be built (the button is off otherwise).
fn request(state: &AppState, source: CodeSource) -> Option<CodeRequest> {
    let ns = state.selected_ns()?.clone();
    match source {
        CodeSource::Aggregate => aggregate_stages(&state.aggregate)
            .ok()
            .map(|stages| CodeRequest::Aggregate { ns, stages }),
        CodeSource::Find => parse_query(&state.find.text)
            .ok()
            .map(|query| CodeRequest::Find { ns, query }),
    }
}

// Never inlined: `AppState` moves by value through the reducer, and its frames are near the
// debug-build stack limit (see `reducer::on_event`).
#[inline(never)]
pub(super) fn on_action(state: AppState, action: CodeAction) -> Out {
    let ui = (*state.code_export).clone();
    let next = match action {
        CodeAction::Open(source) => match request(&state, source) {
            Some(request) => ui.opened(request),
            None => ui,
        },
        CodeAction::SetLanguage(language) => ui.language_set(language),
        CodeAction::SetSetup(setup) => ui.setup_set(setup),
        CodeAction::SetName(field, name) => ui.name_set(field, name),
        CodeAction::SetWriteStage(keep) => ui.write_stage_set(keep),
        CodeAction::Copy => ui,
        CodeAction::Close => ui.closed(),
    };
    let state = AppState {
        code_export: Box::new(next),
        ..state
    };
    (state, Vec::new())
}

#[cfg(test)]
#[path = "code_export_actions_tests.rs"]
mod tests;
