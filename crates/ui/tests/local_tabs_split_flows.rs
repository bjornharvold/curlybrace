//! Split view against a real MongoDB (#159, slice 3): two panes on two saved connections to the
//! same server keep their own pages, and a pane's input is applied in its own connection. Tier 1
//! (a local server): every test writes to its own throwaway `cbit_*` database and drops it. Run
//! with `scripts/integration-tests.sh`, and ALWAYS point `CURLYBRACE_IT_MONGODB_URI` at a
//! throwaway server (the default is a developer's own MongoDB on 27017).
mod support;

use bson::doc;
use curlybrace_core::store::{Namespace, Query};
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::ConnStatus;
use curlybrace_ui::app::tab_actions::TabAction;
use curlybrace_ui::viewmodel::FindAction;
use egui_kittest::kittest::{NodeT, Queryable};
use support::actions::*;
use support::fixture::Fixture;
use support::wait::{SHORT, settle, wait_until};
use uuid::Uuid;

const WORDS: &str = "words";
const THINGS: &str = "things";

macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

fn session(f: &Fixture) -> Uuid {
    f.harness.state().state().active.expect("an active session")
}

fn tab(f: &mut Fixture, action: TabAction) {
    f.harness.state_mut().dispatch(Input::Tabs(action));
    settle(&mut f.harness);
}

fn wait_docs(f: &mut Fixture, n: usize) {
    wait_until(&mut f.harness, "the page", SHORT, |h| {
        h.state().state().page.as_ref().map(|p| p.docs.len()) == Some(n)
    });
}

fn active(f: &Fixture) -> curlybrace_ui::app::tabs::TabId {
    f.harness.state().state().active_tab().expect("a tab")
}

/// The fixture with two saved connections to the same server under different names: the second
/// one is what a pane on another connection is.
fn two_connection_fixture(test: &str) -> Option<Fixture> {
    if !curlybrace_testkit::local_enabled(test) {
        return None;
    }
    let server = curlybrace_testkit::local_server();
    let mut setup = support::fixture::Setup::saved_connection(&server, true);
    let uri = server.uri();
    setup
        .saved
        .push(curlybrace_core::connection::ConnectionConfig::new(
            "second", &uri,
        ));
    let mut f = Fixture::start(server, setup);
    f.db.insert(
        WORDS,
        vec![
            doc! {"_id": 1_i32, "name": "abc"},
            doc! {"_id": 2_i32, "name": "ABC"},
            doc! {"_id": 3_i32, "name": "xyz"},
        ],
    );
    f.db.insert(
        THINGS,
        vec![
            doc! {"_id": 1_i32, "k": 1_i32},
            doc! {"_id": 2_i32, "k": 2_i32},
        ],
    );
    let state = f.harness.state_mut().state_mut();
    state.explain.guard = false;
    state.explain.automatic = false;
    Some(f)
}

/// The number of documents the tab `tab` of `session` shows, wherever the tab lives.
fn docs_of(f: &Fixture, session: Uuid, tab: curlybrace_ui::app::tabs::TabId) -> Option<usize> {
    let s = f.harness.state().state();
    let page = match (s.active == Some(session), s.parked_tabs.get(&tab)) {
        (_, Some(view)) => view.page.as_ref(),
        (true, None) => s.page.as_ref(),
        (false, None) => s.parked.get(&session).and_then(|v| v.page.as_ref()),
    };
    page.map(|p| p.docs.len())
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn split_view_with_two_connections_keeps_both_pages() {
    let Some(mut f) = two_connection_fixture(test_name!()) else {
        return;
    };
    // Connection one browses `words` (3 documents).
    browse(&mut f, WORDS);
    let a = session(&f);
    let a_tab = active(&f);
    // Connection two (the same server, another name) browses `things` (2 documents).
    let b = f
        .harness
        .state()
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.name == "second")
        .map(|c| c.id)
        .expect("the second saved connection");
    f.harness.state_mut().dispatch(Input::Connect(b));
    wait_until(&mut f.harness, "the second connection", SHORT, |h| {
        h.state().state().status_of(b) == Some(ConnStatus::Connected)
    });
    let ns = Namespace::new(f.db.name.as_str(), THINGS);
    f.harness.state_mut().dispatch(Input::Sidebar(
        curlybrace_ui::viewmodel::SidebarAction::Open(ns),
    ));
    wait_docs(&mut f, 2);
    let b_tab = active(&f);
    assert_eq!(session(&f), b);
    // Split: connection two's tab beside connection one's.
    tab(&mut f, TabAction::OpenInOtherPane(a_tab));
    let split = f
        .harness
        .state()
        .state()
        .tabs
        .split
        .clone()
        .expect("a split");
    assert_eq!((split.primary, split.secondary), (b_tab, a_tab));
    settle(&mut f.harness);
    // Both panes are drawn from their own connection, and keep their pages.
    assert_eq!(docs_of(&f, b, b_tab), Some(2));
    assert_eq!(
        docs_of(&f, a, a_tab),
        Some(3),
        "the other pane did not lose its page"
    );
    // Each pane's group names its own connection.
    let names: Vec<String> = f
        .harness
        .query_all_by_role(eframe::egui::accesskit::Role::Group)
        .filter_map(|n| n.accesskit_node().label())
        .map(|l| l.to_string())
        .collect();
    assert!(names.iter().any(|n| n.contains("second")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("local")), "{names:?}");
    // The other pane's own find (an input of that pane) runs on connection one, makes it the
    // active tab, and leaves connection two's page where it was.
    let skip_one = Query {
        skip: 1,
        ..Query::default()
    };
    f.harness.state_mut().dispatch(Input::InTab {
        tab: a_tab,
        input: Box::new(Input::Find(FindAction::Run(skip_one))),
    });
    wait_until(&mut f.harness, "the other pane's find", SHORT, |h| {
        let s = h.state().state();
        s.active == Some(a) && !s.loading_page && s.page.as_ref().map(|p| p.docs.len()) == Some(2)
    });
    assert_eq!(
        docs_of(&f, a, a_tab),
        Some(2),
        "connection one's pane ran its find"
    );
    assert_eq!(
        docs_of(&f, b, b_tab),
        Some(2),
        "connection two's pane kept its page"
    );
    let split = f
        .harness
        .state()
        .state()
        .tabs
        .split
        .clone()
        .expect("still split");
    assert_eq!(
        (split.primary, split.secondary),
        (a_tab, b_tab),
        "focus followed the input"
    );
    assert_eq!(f.harness.state().state().status, ConnStatus::Connected);
    assert!(f.harness.state().state().banner.is_none());
}
