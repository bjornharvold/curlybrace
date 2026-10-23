//! Fixtures for the production connection guard: a coloured production root in the sidebar and
//! the production write warning over the browser.
use super::shots::browsing;
use bson::Bson;
use curlybrace_core::connection::ConnectionColor;
use curlybrace_core::runtime::Command;
use curlybrace_core::test_support::mock_objects::mock_namespace;
use curlybrace_ui::app::state::{AppState, Pending};
use curlybrace_ui::app::write_guard::{ParkedWrite, WriteKind};
use curlybrace_ui::viewmodel::ViewMode;

/// The browser on a session whose saved connection is a red production one.
pub fn production_root(state: &mut AppState) {
    browsing(ViewMode::Tree)(state);
    let active = state.active.expect("browsing opens a session");
    for cfg in state.conn.connections.iter_mut().filter(|c| c.id == active) {
        cfg.production = true;
        cfg.color = Some(ConnectionColor::Red);
    }
}

/// [`production_root`] with a delete parked behind the production write warning.
pub fn write_warning(state: &mut AppState) {
    production_root(state);
    let session = state.active.expect("browsing opens a session");
    let ns = mock_namespace();
    state.write_warning = Some(ParkedWrite {
        command: Command::Delete {
            session,
            ns: ns.clone(),
            id: Bson::Int32(1),
        },
        pending: Pending { ns, editor: None },
        kind: WriteKind::Delete,
        epoch: 1,
    });
}
