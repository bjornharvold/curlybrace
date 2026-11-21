//! The reducer arm for the tool paths: user choices and the answers of the version checks.
use super::reducer::Out;
use super::state::AppState;
use super::tools_state::ToolsAction;

/// Applies the action to the tool paths. Never sends a runtime command: the checks it asks for
/// wait in the state's queue for the app layer to start.
pub(super) fn on_action(mut state: AppState, action: ToolsAction) -> Out {
    if let ToolsAction::SetRestore(on) = action {
        let state = super::workspace_settings_actions::restore_set(state, on);
        return (state, Vec::new());
    }
    if let ToolsAction::SetRememberQueries(on) = action {
        return (
            super::query_history_app::remember_set(state, on),
            Vec::new(),
        );
    }
    let tools = std::mem::take(&mut state.tools);
    let next = match action {
        // Taken above: the switch belongs to the saved session, not to the tool paths.
        ToolsAction::SetRestore(_) | ToolsAction::SetRememberQueries(_) => tools,
        ToolsAction::ChooseGlobal { kind, choice } => tools.choose_global(kind, choice),
        ToolsAction::ChooseOverride {
            connection,
            kind,
            choice,
        } => tools.choose_override(connection, kind, choice),
        ToolsAction::UseGlobal { connection, kind } => tools.use_global(connection, kind),
        ToolsAction::Recheck { scope, kind } => tools.recheck(scope, kind),
        ToolsAction::OpenSettings { suggestions } => tools.open_settings(suggestions),
        ToolsAction::CloseSettings => tools.close_settings(),
        ToolsAction::Browse { scope, kind } => tools.begin_browse(scope, kind),
        ToolsAction::Browsed { path } => tools.browsed(path),
        ToolsAction::Probed {
            scope,
            kind,
            request,
            result,
        } => tools.probed(scope, kind, request, result),
    };
    (
        AppState {
            tools: next,
            ..state
        },
        Vec::new(),
    )
}
