//! Fixed in-memory states with several tabs, for the tab tests and snapshots. No database, no
//! network: the tabs are put together through the model API (`tab_opened`) and their fields set
//! directly, the way the reducer leaves them.
use super::db::sample_people;
use super::shots::{browsing, install_page};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::store::{Namespace, Page};
use curlybrace_ui::app::session::SessionView;
use curlybrace_ui::app::state::{AppState, ConnStatus};
use curlybrace_ui::app::tab_strip::Split;
use curlybrace_ui::app::tabs::TabId;
use curlybrace_ui::viewmodel::split_layout::Orientation;
use curlybrace_ui::viewmodel::{SidebarState, ViewMode, page_key};
use uuid::Uuid;

fn people_page() -> Page {
    Page {
        docs: sample_people(),
        skip: 0,
        has_more: false,
    }
}

/// The browsing fixture (`Production` session on `crm.customers`, showing people) with one more
/// parked tab per collection of `others`, each holding the same page. Returns the new tabs.
pub fn browsing_with_tabs(state: &mut AppState, others: &[&str]) -> Vec<TabId> {
    browsing(ViewMode::Table)(state);
    let session = state.active.expect("an active session");
    // The session's first tab is the current one (the fields `browsing` just set).
    *state = std::mem::take(state).tab_opened(session).0;
    others
        .iter()
        .map(|collection| parked_tab(state, session, "crm", collection))
        .collect()
}

/// A parked tab of `session` on `db.collection` holding a page.
pub fn parked_tab(state: &mut AppState, session: Uuid, db: &str, collection: &str) -> TabId {
    let (next, tab) = std::mem::take(state).tab_opened(session);
    *state = next;
    let tab = tab.expect("the session is open");
    let page = people_page();
    if let Some(view) = state.parked_tabs.get_mut(&tab) {
        view.ns = Some(Namespace::new(db, collection));
        view.page_key = Some(page_key(&page));
        view.page = Some(page);
        view.view_mode = ViewMode::Table;
    }
    tab
}

/// Another open session named `name` whose one tab shows `crm.<collection>`, with the given
/// production and read-only flags; it stays in the background.
pub fn background_session(
    state: &mut AppState,
    name: &str,
    collection: &str,
    (production, read_only): (bool, bool),
) -> Uuid {
    let mut cfg = ConnectionConfig::new(name, "mongodb://db.example.com:27017");
    cfg.production = production;
    cfg.read_only = read_only;
    let id = cfg.id;
    state.conn.connections.push(cfg);
    state.open.push(id);
    state.parked.insert(
        id,
        SessionView {
            status: ConnStatus::Connected,
            production,
            read_only,
            sidebar: SidebarState::default().selecting(Namespace::new("crm", collection)),
            ..SessionView::default()
        },
    );
    *state = std::mem::take(state).tab_opened(id).0;
    id
}

/// The six-tab strip of the layout audit and the small snapshot: three plain tabs of the active
/// session and a production one, a read-only one and a production read-only one of other sessions.
pub fn six_tabs(state: &mut AppState) {
    browsing_with_tabs(state, &["invoices", "notes"]);
    background_session(state, "Staging", "orders", (true, false));
    background_session(state, "Reports", "sessions", (false, true));
    background_session(state, "Billing", "ledger", (true, true));
}

/// Splits the view: the tab being looked at beside `partner`, side by side or stacked.
pub fn split_with(state: &mut AppState, partner: TabId, orientation: Orientation) {
    let primary = state.active_tab().expect("a tab being looked at");
    state.tabs = std::mem::take(&mut state.tabs).with_split(Some(Split {
        primary,
        secondary: partner,
        ratio: 0.5,
        orientation,
        secondary_first: false,
    }));
}

/// What a real connection is called: long enough that the Browse header (the connection's name,
/// badge and sub-tabs, which does not wrap) is as wide as it is in use. The layout rules are
/// measured with it.
pub const REAL_NAME: &str = "Production replica";

/// Renames the active connection, as the layout tests do before they measure.
pub fn renamed(state: &mut AppState, name: &str) {
    let active = state.active;
    for connection in state
        .conn
        .connections
        .iter_mut()
        .filter(|c| Some(c.id) == active)
    {
        connection.name = name.to_string();
    }
}

/// On a page that already shows the fixture (`browse_page` installs it): the shown tab beside a
/// parked one of the same connection, with a real connection name, and no second session.
pub fn split_here(state: &mut AppState, orientation: Orientation) {
    renamed(state, REAL_NAME);
    let session = state.active.expect("an active session");
    *state = std::mem::take(state).tab_opened(session).0;
    let partner = parked_tab(state, session, "crm", "invoices");
    split_with(state, partner, orientation);
}

/// The fixture's tab of `crm.customers` beside a parked tab of the same connection.
pub fn split_two(state: &mut AppState, orientation: Orientation) {
    let tabs = browsing_with_tabs(state, &["invoices"]);
    renamed(state, REAL_NAME);
    split_with(state, tabs[0], orientation);
}

/// The fixture's tab beside the one tab of another connection (`Staging`, a production one).
pub fn split_two_connections(state: &mut AppState, orientation: Orientation) {
    browsing_with_tabs(state, &[]);
    renamed(state, REAL_NAME);
    let other = background_session(state, "Staging", "orders", (true, false));
    // The other connection's tab shows a page, as a tab that has run its query does.
    if let Some(view) = state.parked.get_mut(&other) {
        let page = people_page();
        view.page_key = Some(page_key(&page));
        view.page = Some(page);
        view.view_mode = ViewMode::Table;
    }
    let partner = state
        .current_tab_of(other)
        .expect("the other connection's tab");
    split_with(state, partner, orientation);
}

/// Marks the active session's given tab dormant: its page is gone and it waits for Run.
pub fn make_dormant(state: &mut AppState, tab: TabId) {
    state.tabs = std::mem::take(&mut state.tabs).made_dormant([tab]);
}

/// Puts the fixture's page on the current tab again (after a change dropped it).
pub fn current_page(state: &mut AppState) {
    install_page(state, sample_people());
}
