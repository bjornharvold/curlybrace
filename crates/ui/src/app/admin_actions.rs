//! The answers of the administration commands (drop, rename, view, duplicate, statistics).

use curlybrace_core::runtime::Command;

use super::reducer::Out;
use super::state::AppState;

/// An administration write succeeded: the databases are listed again, and so are the collections
/// of `db` when the sidebar holds them (an expanded node that is not loaded is left alone, it
/// fetches on its own expansion).
pub(super) fn changed(state: AppState, db: String) -> Out {
    let state = super::admin_dialog_actions::answered(state, &db);
    let session = state.session_id();
    let mut commands = vec![Command::ListDatabases { session }];
    let loaded = state
        .sidebar
        .databases
        .iter()
        .any(|d| d.name == db && d.collections.is_some());
    let mut loading = state.loading.clone();
    if loaded && loading.insert(db.clone()) {
        commands.push(Command::ListCollections { session, db });
    }
    (AppState { loading, ..state }, commands)
}
