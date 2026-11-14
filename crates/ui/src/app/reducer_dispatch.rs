//! The reducer's dispatch of everything the person does (everything but a server event).
use super::actions;
use super::aggregate_actions;
use super::bulk_actions;
use super::compare;
use super::connection_test;
use super::copy;
use super::index_actions;
use super::reducer::{Input, Out, forget_finished, forget_requested, on_event};
use super::saved_copies;
use super::session_actions;
use super::shell_actions;
use super::state::AppState;
use super::tools_actions;
use super::write_guard;

/// What the person does, then the Copy as code dialog closed if the view moved away from it.
#[inline(never)]
pub(super) fn update_view(state: AppState, input: Input) -> Out {
    let mut out = dispatch(state, input);
    super::code_export_actions::close_if_stale(&mut out.0);
    out
}

fn dispatch(state: AppState, input: Input) -> Out {
    match input {
        Input::Event(ev) => on_event(state, ev),
        Input::Sidebar(a) => actions::sidebar(state, a),
        Input::Find(a) => actions::find(state, a),
        Input::Doc(a) => actions::doc(state, a),
        Input::Editor(a) => actions::editor(state, a),
        Input::Indexes(a) => index_actions::on_action(state, a),
        Input::Tab(tab) => index_actions::switch_tab(state, tab),
        Input::WriteWarning(a) => write_guard::on_action(state, a),
        Input::Connect(id) => session_actions::connect(state, id),
        Input::Disconnect(id) => session_actions::disconnect(state, id),
        Input::Activate(id) => session_actions::activate(state, id),
        Input::ShowConnections => session_actions::show_connections(state),
        Input::TestConnection(form) => connection_test::start(state, &form),
        Input::Compare(a) => compare::on_action(state, a),
        Input::Copy(a) => copy::on_input(state, a),
        Input::Tools(a) => tools_actions::on_action(state, a),
        Input::Bulk(a) => bulk_actions::on_action(state, a),
        Input::Copies(a) => saved_copies::on_action(state, a),
        Input::Shell(a) => shell_actions::on_action(state, a),
        Input::Aggregate(a) => aggregate_actions::on_action(state, a),
        Input::DismissBanner => (state.without_banner(), Vec::new()),
        Input::ForgetRequested => (forget_requested(state), Vec::new()),
        Input::ForgetCancelled => (
            AppState {
                forget_confirm: None,
                ..state
            },
            Vec::new(),
        ),
        Input::ForgetFinished(result) => (forget_finished(state, result), Vec::new()),
    }
}
