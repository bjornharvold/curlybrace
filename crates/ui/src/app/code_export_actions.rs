//! The Copy as code dialog's handlers. Pure and offline: opening it reads the draft or the query
//! bar of the open collection, and nothing is ever sent to the server, so it works without a
//! connection. Copy itself is the view's (the clipboard is egui's); the reducer only keeps the
//! dialog open.
use super::reducer::Out;
use super::state::{AppState, Workspace};
use crate::viewmodel::NameField;
use crate::viewmodel::code_export::aggregate_stages;
use curlybrace_core::codegen::{CodeRequest, Language};
use curlybrace_core::query::parse_export_query;

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
    /// Whether the code carries the aggregate's options.
    SetIncludeOptions(bool),
    /// The view put the previewed text on the clipboard; the dialog stays open.
    Copy,
    Close,
}

/// The request `source` stands for in `state`, if it can be built (the button is off otherwise).
fn request(state: &AppState, source: CodeSource) -> Option<CodeRequest> {
    let ns = state.selected_ns()?.clone();
    match source {
        CodeSource::Aggregate => {
            aggregate_stages(&state.aggregate)
                .ok()
                .map(|stages| CodeRequest::Aggregate {
                    ns,
                    stages,
                    options: state.aggregate.options.clone(),
                })
        }
        CodeSource::Find => find_request(state, ns),
    }
}

/// The find the query bar would send, as code: its text and its options. Nothing while either
/// does not parse (the button is off).
fn find_request(state: &AppState, ns: curlybrace_core::store::Namespace) -> Option<CodeRequest> {
    let options = state
        .find
        .options
        .parsed()
        .ok()
        .filter(|_| !state.find.options_blocked())?;
    let query = parse_export_query(&state.find.text).ok()?;
    Some(CodeRequest::Find {
        ns,
        query: curlybrace_core::store::Query {
            options: options.clone(),
            ..query
        },
    })
}

/// `state` with the dialog closed when it no longer belongs to what is in view: another session,
/// another collection (none, after a lost connection) or the connection manager. Run after every
/// input, so no path that changes the view can leave it behind.
pub(super) fn close_if_stale(state: &mut AppState) {
    if state.code_export.dialog.is_none() {
        return;
    }
    let browsing = state.workspace == Workspace::Browse;
    let stale = state
        .code_export
        .is_stale(state.session_id(), state.selected_ns());
    if !browsing || stale {
        let ui = std::mem::take(&mut *state.code_export);
        *state.code_export = ui.closed();
    }
}

// Never inlined: `AppState` moves by value through the reducer, and its frames are near the
// debug-build stack limit (see `reducer::on_event`).
#[inline(never)]
pub(super) fn on_action(mut state: AppState, action: CodeAction) -> Out {
    // Moved out of the state (no copy of the dialog's text and request), and put back changed.
    let ui = std::mem::take(&mut *state.code_export);
    let next = match action {
        CodeAction::Open(source) => match request(&state, source) {
            Some(request) => ui.opened_in(request, state.session_id()),
            None => ui,
        },
        CodeAction::SetLanguage(language) => ui.language_set(language),
        CodeAction::SetSetup(setup) => ui.setup_set(setup),
        CodeAction::SetName(field, name) => ui.name_set(field, name),
        CodeAction::SetWriteStage(keep) => ui.write_stage_set(keep),
        CodeAction::SetIncludeOptions(include) => ui.include_options_set(include),
        CodeAction::Copy => ui,
        CodeAction::Close => ui.closed(),
    };
    *state.code_export = next;
    (state, Vec::new())
}

#[cfg(test)]
#[path = "code_export_actions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "code_export_scope_tests.rs"]
mod scope_tests;

#[cfg(test)]
#[path = "code_export_options_tests.rs"]
mod options_tests;
