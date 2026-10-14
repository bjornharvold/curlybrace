//! Opening, focusing and closing sessions. Pure: each handler returns the next state plus the
//! runtime commands to send.
//!
//! The runtime numbers every Connect and every Disconnect command from one counter whichever
//! session it names, so `epoch_counter` goes up by exactly one per command sent here.
use super::reducer::Out;
use super::session::SessionView;
use super::state::{AppState, Banner, ConnStatus, HostEndpoint, Workspace};
use curlybrace_core::runtime::Command;
use uuid::Uuid;

/// Connects `id`, reconnects it when its last attempt failed, or just focuses it when it is open.
pub(super) fn connect(state: AppState, id: Uuid) -> Out {
    let Some(cfg) = state.connection(id).cloned() else {
        return (state, Vec::new());
    };
    match state.status_of(id) {
        Some(ConnStatus::Failed) | None => {}
        Some(_) => return (activate_in_browser(state, id), Vec::new()),
    }
    let epoch = state.epoch_counter + 1;
    let view = SessionView {
        status: ConnStatus::Connecting,
        latest_requested_epoch: epoch,
        ssh_endpoint: HostEndpoint::of(&cfg),
        ..SessionView::default()
    };
    let next = AppState {
        epoch_counter: epoch,
        banner: None,
        forget_confirm: None,
        workspace: Workspace::Browse,
        ..state.opened_with(id, view)
    };
    (next, vec![Command::Connect(cfg)])
}

/// Closes `id`: its root and view go, and the neighbour (the next root, else the previous one)
/// takes the focus when `id` was active. A failed root is closed with a Disconnect too, because
/// the runtime keeps an empty entry for it until then.
pub(super) fn disconnect(state: AppState, id: Uuid) -> Out {
    let Some(position) = state.open.iter().position(|open| *open == id) else {
        return (state, Vec::new());
    };
    let neighbour = state
        .open
        .get(position + 1)
        .or_else(|| position.checked_sub(1).and_then(|p| state.open.get(p)))
        .copied();
    let epoch = state.epoch_counter + 1;
    let focused = match (state.active == Some(id), neighbour) {
        (true, Some(next)) => state.activated(next),
        _ => state,
    };
    // A failure the closed session raised goes with it; other sessions' banners stay.
    let own_failure = focused.banner_session == Some(id)
        && matches!(focused.banner, Some(Banner::Failure { .. }));
    let dropped = focused.session_dropped(id);
    let next = if own_failure {
        AppState {
            banner: None,
            banner_session: None,
            forget_target: None,
            ..dropped
        }
    } else {
        dropped
    };
    let next = AppState {
        epoch_counter: epoch,
        ..next
    };
    (next, vec![Command::Disconnect { session: id }])
}

/// Shows the open session `id`; unknown ids change nothing.
pub(super) fn activate(state: AppState, id: Uuid) -> Out {
    (activate_in_browser(state, id), Vec::new())
}

/// Shows the connection manager; the open sessions and their views stay as they are.
pub(super) fn show_connections(state: AppState) -> Out {
    (
        AppState {
            workspace: Workspace::Connections,
            ..state
        },
        Vec::new(),
    )
}

fn activate_in_browser(state: AppState, id: Uuid) -> AppState {
    if !state.is_open(id) {
        return state;
    }
    AppState {
        workspace: Workspace::Browse,
        ..state.activated(id)
    }
}

#[cfg(test)]
#[path = "session_flow_tests.rs"]
mod flow_tests;
