//! Flows of the collection tabs against a real MongoDB (#159, slice 2): the REAL app and runtime,
//! driven by tab actions through the reducer and by the find bar's own controls. Tier 1 (a local
//! server): every test writes to its own throwaway `cbit_*` database and drops it. Run with
//! `scripts/integration-tests.sh`, and ALWAYS point `CURLYBRACE_IT_MONGODB_URI` at a throwaway
//! server (the default is a developer's own MongoDB on 27017).
mod support;

use bson::doc;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::{Namespace, Query};
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::state::ConnStatus;
use curlybrace_ui::app::tab_actions::TabAction;
use curlybrace_ui::app::tabs::Freshness;
use curlybrace_ui::viewmodel::{DocAction, EditorAction, FindAction};
use std::time::{Duration, Instant};
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

macro_rules! fixture {
    () => {{
        let Some(mut f) = Fixture::local(test_name!()) else {
            return;
        };
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
        // The guard and the verdict would hold or add reads: these flows are about tabs.
        let state = f.harness.state_mut().state_mut();
        state.explain.guard = false;
        state.explain.automatic = false;
        f
    }};
}

fn session(f: &Fixture) -> Uuid {
    f.harness.state().state().active.expect("an active session")
}

fn docs_shown(f: &Fixture) -> Option<usize> {
    f.harness
        .state()
        .state()
        .page
        .as_ref()
        .map(|p| p.docs.len())
}

fn tab(f: &mut Fixture, action: TabAction) {
    f.harness.state_mut().dispatch(Input::Tabs(action));
    settle(&mut f.harness);
}

fn open_in_new_tab(f: &mut Fixture, collection: &str) {
    let ns = Namespace::new(f.db.name.as_str(), collection);
    let session = session(f);
    tab(f, TabAction::OpenInNew { session, ns });
}

fn wait_docs(f: &mut Fixture, n: usize) {
    wait_until(&mut f.harness, "the page", SHORT, |h| {
        h.state().state().page.as_ref().map(|p| p.docs.len()) == Some(n)
    });
}

fn active(f: &Fixture) -> curlybrace_ui::app::tabs::TabId {
    f.harness.state().state().active_tab().expect("a tab")
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn two_tabs_run_different_filters_and_keep_their_pages() {
    let mut f = fixture!();
    browse(&mut f, WORDS);
    let words = active(&f);
    let filter = f.labels.t("findbar.filter");
    fill(&mut f, &filter, r#"{"name": "abc"}"#);
    click_key(&mut f, "findbar.run");
    wait_docs(&mut f, 1);
    open_in_new_tab(&mut f, THINGS);
    wait_docs(&mut f, 2);
    let things = active(&f);
    assert_ne!(words, things);
    // Back to the first tab: its own filter and its own page, with no read of the server.
    tab(&mut f, TabAction::Activate(words));
    assert_eq!(docs_shown(&f), Some(1), "the filtered page is kept");
    assert_eq!(
        f.harness.state().state().find.text.filter,
        r#"{"name": "abc"}"#
    );
    assert_eq!(
        f.harness
            .state()
            .state()
            .selected_ns()
            .map(|n| n.collection.as_str()),
        Some(WORDS)
    );
    tab(&mut f, TabAction::Activate(things));
    assert_eq!(docs_shown(&f), Some(2), "the other tab keeps its page");
    assert_eq!(f.harness.state().state().find.text.filter, "");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_write_in_one_tab_refreshes_the_other_on_show() {
    let mut f = fixture!();
    browse(&mut f, WORDS);
    let first = active(&f);
    tab(&mut f, TabAction::Duplicate(first));
    wait_docs(&mut f, 3);
    let copy = active(&f);
    assert_ne!(first, copy);
    // Insert a document from the copy.
    let state = f.harness.state_mut();
    state.dispatch(Input::Doc(DocAction::Insert));
    let text = r#"{"_id": 10, "name": "added"}"#.to_string();
    f.harness
        .state_mut()
        .dispatch(Input::Editor(EditorAction::Save {
            original_id: None,
            text,
        }));
    wait_until(
        &mut f.harness,
        "the write and the copy's re-read",
        SHORT,
        |h| {
            h.state().state().pending.is_none()
                && h.state().state().page.as_ref().map(|p| p.docs.len()) == Some(4)
        },
    );
    // The first tab, parked, still shows what it read, and says it is stale.
    let stale = f.harness.state().state().freshness_of(first);
    assert_eq!(stale, Freshness::Stale);
    // Showing it reads again: the new document is there.
    tab(&mut f, TabAction::Activate(first));
    wait_docs(&mut f, 4);
    assert_eq!(
        f.harness.state().state().freshness_of(first),
        Freshness::Fresh
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn closing_a_tab_drops_its_late_answer() {
    let mut f = fixture!();
    browse(&mut f, WORDS);
    let words = active(&f);
    open_in_new_tab(&mut f, THINGS);
    wait_docs(&mut f, 2);
    let things = active(&f);
    // A find that takes about two seconds on the server, sent from the second tab.
    let slow = Query {
        filter: doc! {"$where": "sleep(2000) || true"},
        ..Query::default()
    };
    f.harness
        .state_mut()
        .dispatch(Input::Find(FindAction::Run(slow)));
    settle(&mut f.harness);
    assert!(f.harness.state().state().loading_page || docs_shown(&f).is_some());
    // Leave it and close it while the server is still busy.
    tab(&mut f, TabAction::Activate(words));
    tab(&mut f, TabAction::Close(things));
    assert!(!f.harness.state().state().tabs.owner.contains_key(&things));
    let until = Instant::now() + Duration::from_millis(3500);
    while Instant::now() < until {
        f.harness.step();
        std::thread::sleep(Duration::from_millis(20));
    }
    let s = f.harness.state().state();
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.banner.is_none(), "a late answer raised {:?}", s.banner);
    assert_eq!(
        s.page.as_ref().map(|p| p.docs.len()),
        Some(3),
        "the open tab is untouched"
    );
    assert_eq!(s.selected_ns().map(|n| n.collection.as_str()), Some(WORDS));
    assert_eq!(s.tabs_of(session(&f)).len(), 1);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn lost_and_reconnected_tabs_stay_dormant_until_run() {
    let mut f = fixture!();
    browse(&mut f, WORDS);
    let words = active(&f);
    open_in_new_tab(&mut f, THINGS);
    wait_docs(&mut f, 2);
    let things = active(&f);
    let id = session(&f);
    // The connection is lost: the session fails, the tabs stay.
    f.harness.state_mut().dispatch(Input::Event(Event::Failed {
        session: id,
        context: "find",
        error: CoreError::Unreachable,
    }));
    settle(&mut f.harness);
    assert_eq!(f.harness.state().state().status, ConnStatus::Failed);
    assert_eq!(
        f.harness.state().state().tabs_of(id).len(),
        2,
        "no tab was lost"
    );
    // Reconnect: connected again, and nothing was read for the tabs.
    f.harness.state_mut().dispatch(Input::Connect(id));
    wait_until(&mut f.harness, "the reconnect", SHORT, |h| {
        h.state().state().status == ConnStatus::Connected
    });
    settle(&mut f.harness);
    let s = f.harness.state().state();
    assert!(s.page.is_none() && !s.loading_page, "no find was sent");
    assert_eq!(s.freshness_of(things), Freshness::Dormant);
    assert_eq!(s.freshness_of(words), Freshness::Dormant);
    // Run wakes one tab; the other stays dormant until its own Run.
    tab(&mut f, TabAction::Run(things));
    wait_docs(&mut f, 2);
    let s = f.harness.state().state();
    assert_eq!(s.freshness_of(things), Freshness::Fresh);
    assert_eq!(s.freshness_of(words), Freshness::Dormant);
}
