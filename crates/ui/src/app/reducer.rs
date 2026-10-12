//! The pure reducer: `update(state, input) -> (state, commands)`. No I/O, no egui.
use super::actions;
use super::persistence::PersistError;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{DocAction, EditorAction, FindAction, SidebarAction, page_key};
use mongostudio_core::error::CoreError;
use mongostudio_core::runtime::{Command, Event};
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
    Disconnect,
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
        Input::Connect(id) => actions::connect(state, id),
        Input::Disconnect => actions::disconnect(state),
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
    with_banner(state, Some(Banner::Failure { context, error }))
}

fn on_event(state: AppState, ev: Event) -> Out {
    match ev {
        Event::Connected => epoch_done(state, |s| {
            let s = AppState {
                status: ConnStatus::Connected,
                ..s
            };
            (s, vec![Command::ListDatabases])
        }),
        Event::Disconnected => epoch_done(state, |s| (disconnected(s, None), Vec::new())),
        Event::Failed {
            context: context @ ("connect" | "disconnect"),
            error,
        } => epoch_done(state, move |s| {
            (disconnected(s, Some((context, error))), Vec::new())
        }),
        // Anything else is a late answer from a session that is gone.
        _ if state.status != ConnStatus::Connected => (state, Vec::new()),
        Event::Databases(names) => {
            let sidebar = state.sidebar.with_databases(names);
            (AppState { sidebar, ..state }, Vec::new())
        }
        Event::Collections { db, names } => {
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
        Event::Results { ns, query, page } => {
            let current = state.selected_ns() == Some(&ns) && state.last_query == query;
            (
                if current {
                    results_replaced(state, page)
                } else {
                    state
                },
                Vec::new(),
            )
        }
        Event::Mutated { ns } => mutated(state, ns),
        Event::Failed { context, error } => (data_failed(state, context, error), Vec::new()),
    }
}

/// Connect/Disconnect are ordered by request, so only the outcome of the LAST request is
/// authoritative; earlier (superseded) outcomes are consumed and ignored.
fn epoch_done(state: AppState, apply: impl FnOnce(AppState) -> Out) -> Out {
    let latest = state.epoch_pending <= 1;
    let state = AppState {
        epoch_pending: state.epoch_pending.saturating_sub(1),
        ..state
    };
    if latest {
        apply(state)
    } else {
        (state, Vec::new())
    }
}

/// A disconnected state with the whole session cleared and an optional failure banner.
fn disconnected(state: AppState, failed: Option<(&'static str, CoreError)>) -> AppState {
    let cleared = state.session_cleared();
    let banner = match failed {
        Some((context, error)) => Some(Banner::Failure { context, error }),
        None => cleared.banner.clone(),
    };
    AppState {
        status: ConnStatus::Disconnected,
        banner,
        ..cleared
    }
}

/// Stores a fresh page, memoising its content key once (not per frame).
fn results_replaced(state: AppState, page: mongostudio_core::store::Page) -> AppState {
    AppState {
        page_key: Some(page_key(&page)),
        page: Some(page),
        tree: state.tree.results_replaced(),
        ..state
    }
}

fn mutated(state: AppState, ns: mongostudio_core::store::Namespace) -> Out {
    let state = AppState {
        pending_mutation: false,
        ..state
    };
    if state.selected_ns() != Some(&ns) {
        return (state, Vec::new());
    }
    let query = state.last_query.clone();
    let state = AppState {
        editor: None,
        ..state
    };
    actions::issue_find(state, query)
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
        "insert" | "replace" if state.editor.is_some() => {
            let editor = state.editor.map(|e| crate::viewmodel::EditorState {
                error: Some(error),
                ..e
            });
            AppState {
                editor,
                pending_mutation: false,
                ..state
            }
        }
        "insert" | "replace" | "delete" | "clone" => failure(
            AppState {
                pending_mutation: false,
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
