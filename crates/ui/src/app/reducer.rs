//! The pure reducer: `update(state, input) -> (state, commands)`. No I/O, no egui.
use super::actions;
use super::compare::{self, CompareAction};
use super::connection_test;
use super::persistence::PersistError;
use super::session_actions;
use super::state::{AppState, Banner, ConnStatus, Pending};
use crate::viewmodel::{
    ConnectionForm, DocAction, EditorAction, EditorState, FindAction, SidebarAction, page_key,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, Event};
use uuid::Uuid;

/// Next state plus the commands the shell must send to the runtime.
pub type Out = (AppState, Vec<Command>);

/// Everything that can change the state.
#[derive(Debug)]
pub enum Input {
    Event(Event),
    Sidebar(SidebarAction),
    Find(FindAction),
    Doc(DocAction),
    Editor(EditorAction),
    Connect(Uuid),
    /// Close the named session (its root disappears from the tree).
    Disconnect(Uuid),
    /// Make the named open session the one the workspace shows.
    Activate(Uuid),
    /// Show the connection manager next to the open sessions.
    ShowConnections,
    TestConnection(Box<ConnectionForm>),
    Compare(CompareAction),
    DismissBanner,
    ForgetRequested,
    ForgetCancelled,
    ForgetFinished(Result<(), PersistError>),
}

pub fn update(state: AppState, input: Input) -> Out {
    match input {
        Input::Event(ev) => on_event(state, ev),
        Input::Sidebar(a) => actions::sidebar(state, a),
        Input::Find(a) => actions::find(state, a),
        Input::Doc(a) => actions::doc(state, a),
        Input::Editor(a) => actions::editor(state, a),
        Input::Connect(id) => session_actions::connect(state, id),
        Input::Disconnect(id) => session_actions::disconnect(state, id),
        Input::Activate(id) => session_actions::activate(state, id),
        Input::ShowConnections => session_actions::show_connections(state),
        Input::TestConnection(form) => connection_test::start(state, &form),
        Input::Compare(a) => compare::on_action(state, a),
        Input::DismissBanner => (with_banner(state, None), Vec::new()),
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

/// State-only view of [`update`] for callers that do not send commands.
pub fn apply_event(state: AppState, ev: Event) -> AppState {
    on_event(state, ev).0
}

fn with_banner(state: AppState, banner: Option<Banner>) -> AppState {
    AppState { banner, ..state }
}

fn failure(state: AppState, context: &'static str, error: CoreError) -> AppState {
    let banner_session = state.active;
    AppState {
        banner_session,
        forget_target: None,
        ..with_banner(state, Some(Banner::Failure { context, error }))
    }
}

fn on_event(state: AppState, ev: Event) -> Out {
    // Compare sessions and connection tests are independent of the browsing sessions and their
    // epochs.
    let session = match &ev {
        Event::Compare(_) | Event::TestConnection { .. } => None,
        other => other.session(),
    };
    match (ev, session) {
        (Event::Compare(e), _) => compare::on_event(state, e),
        (
            Event::TestConnection {
                request_id,
                outcome,
            },
            _,
        ) => (
            connection_test::on_event(state, request_id, outcome),
            Vec::new(),
        ),
        // A late answer of a session that was closed: it never resurrects the root and never
        // applies to another session.
        (_, Some(session)) if !state.is_open(session) => (state, Vec::new()),
        (ev, Some(session)) => state.with_session(session, |s| on_session_event(s, ev)),
        (_, None) => (state, Vec::new()),
    }
}

/// Applies an event of the session that is active while this runs (see `with_session`).
fn on_session_event(state: AppState, ev: Event) -> Out {
    match ev {
        // The runtime reports every outcome of a replaced session as `Superseded`.
        // Connect/Disconnect already cleared everything tied to that session at dispatch
        // (in-flight markers, pending mutation), so there is nothing left to undo: consuming
        // the answer is enough. Clearing markers here could wrongly clear the NEW session's.
        Event::Failed {
            error: CoreError::Superseded,
            ..
        } => (state, Vec::new()),
        Event::Connected {
            epoch, topology, ..
        } => at_epoch(state, epoch, |s| {
            let s = AppState {
                status: ConnStatus::Connected,
                failure: None,
                topology,
                banner: s.banner.filter(|b| !is_data_failure(b)),
                ..s
            };
            let session = s.session_id();
            (s, vec![Command::ListDatabases { session }])
        }),
        // The session was closed by `disconnect` already; only an answer for a root that is
        // still open (a newer connect of the same id) can get here, and it is not about it.
        Event::Disconnected { .. } => (state, Vec::new()),
        Event::ConnectFailed { epoch, error, .. } => at_epoch(state, epoch, |s| {
            (connection_lost(s, "connect", error), Vec::new())
        }),
        Event::Failed {
            context: "disconnect",
            error,
            ..
        } => (connection_lost(state, "disconnect", error), Vec::new()),
        // Anything else is a late answer from a session that is not connected.
        _ if state.status != ConnStatus::Connected => (state, Vec::new()),
        Event::Databases { names, .. } => {
            let sidebar = state.sidebar.with_databases(names);
            (AppState { sidebar, ..state }, Vec::new())
        }
        Event::Collections { db, names, .. } => {
            let mut loading = state.loading;
            loading.remove(&db);
            let sidebar = state.sidebar.with_collections(&db, names);
            let next = AppState {
                sidebar,
                loading,
                ..state
            };
            (next, Vec::new())
        }
        Event::Results {
            ns, query, page, ..
        } => {
            let current = state.selected_ns() == Some(&ns) && state.issued_query == query;
            (
                if current {
                    results_replaced(state, query, page)
                } else {
                    state
                },
                Vec::new(),
            )
        }
        Event::Mutated { ns, .. } => mutated(state, ns),
        Event::Failed { context, error, .. } => (data_failed(state, context, error), Vec::new()),
        // Routed before; these have no session.
        Event::Compare(_) | Event::TestConnection { .. } => (state, Vec::new()),
    }
}

/// Applies a connect-family outcome unless a newer Connect/Disconnect was requested since.
fn at_epoch(state: AppState, epoch: u64, apply: impl FnOnce(AppState) -> Out) -> Out {
    if epoch < state.latest_requested_epoch {
        return (state, Vec::new());
    }
    apply(AppState {
        latest_requested_epoch: state.latest_requested_epoch.max(epoch),
        ..state
    })
}

/// Banners about data commands of a previous session.
fn is_data_failure(banner: &Banner) -> bool {
    matches!(
        banner,
        Banner::Failure {
            context: "find"
                | "insert"
                | "replace"
                | "delete"
                | "clone"
                | "list_databases"
                | "list_collections",
            ..
        }
    )
}

/// The active session's connection failed or was lost: its root stays (marked failed, showing
/// the error), its view is cleared, and the user is told through the banner.
fn connection_lost(state: AppState, context: &'static str, error: CoreError) -> AppState {
    let banner = Some(Banner::Failure {
        context,
        error: error.clone(),
    });
    // Captured here, while the failing session's view is the active one.
    let forget_target = (context == "connect" && error == CoreError::SshHostKeyChanged)
        .then(|| state.ssh_endpoint.clone())
        .flatten();
    let banner_session = state.active;
    AppState {
        banner,
        banner_session,
        forget_target,
        ..state.failed(error)
    }
}

/// Stores a fresh page, memoising its content key once (not per frame).
fn results_replaced(
    state: AppState,
    query: curlybrace_core::store::Query,
    page: curlybrace_core::store::Page,
) -> AppState {
    AppState {
        page_key: Some(page_key(&page)),
        page: Some(page),
        page_query: query,
        loading_page: false,
        tree: state.tree.results_replaced(),
        ..state
    }
}

/// A mutation finished. Only the one in flight (same namespace) is resolved; the editor it came
/// from is closed, and the page is re-fetched with the query that produced it.
fn mutated(state: AppState, ns: curlybrace_core::store::Namespace) -> Out {
    let matches_pending = state.pending.as_ref().is_some_and(|p| p.ns == ns);
    let editor_done = matches_pending && editor_matches(&state, state.pending.as_ref());
    let state = AppState {
        pending: if matches_pending { None } else { state.pending },
        editor: if editor_done { None } else { state.editor },
        ..state
    };
    if state.selected_ns() != Some(&ns) {
        return (state, Vec::new());
    }
    let query = state.page_query.clone();
    actions::issue_find(state, query)
}

/// True when the open editor still holds what `pending` was saved from.
fn editor_matches(state: &AppState, pending: Option<&Pending>) -> bool {
    match (&state.editor, pending.and_then(|p| p.editor.as_ref())) {
        (Some(editor), Some(key)) => {
            editor.original_id == key.original_id && editor.text == key.text
        }
        _ => false,
    }
}

/// Failed data commands: the previous page stays visible; the user sees a localized banner.
fn data_failed(state: AppState, context: &'static str, error: CoreError) -> AppState {
    match context {
        "list_collections" => failure(
            AppState {
                loading: Default::default(),
                ..state
            },
            context,
            error,
        ),
        "find" => failure(
            AppState {
                loading_page: false,
                ..state
            },
            context,
            error,
        ),
        "insert" | "replace" if editor_matches(&state, state.pending.as_ref()) => {
            let editor = state.editor.map(|e| EditorState {
                error: Some(error),
                ..e
            });
            AppState {
                editor,
                pending: None,
                ..state
            }
        }
        "insert" | "replace" | "delete" | "clone" => failure(
            AppState {
                pending: None,
                ..state
            },
            context,
            error,
        ),
        _ => failure(state, context, error),
    }
}

fn forget_requested(state: AppState) -> AppState {
    let target = state.forgettable_host().cloned();
    match target {
        Some(endpoint) => AppState {
            forget_confirm: Some(endpoint),
            ..state
        },
        None => state,
    }
}

fn forget_finished(state: AppState, result: Result<(), PersistError>) -> AppState {
    let banner = match result {
        Ok(()) => Banner::HostForgotten,
        Err(e) => Banner::Persist(e),
    };
    AppState {
        forget_confirm: None,
        banner: Some(banner),
        ..state
    }
}

#[cfg(test)]
#[path = "reducer_tests.rs"]
mod tests;
