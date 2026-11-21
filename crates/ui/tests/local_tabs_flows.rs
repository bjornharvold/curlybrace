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

/// Server-side evidence, read through a client of the test's own (never through the app).
struct Server {
    handle: tokio::runtime::Handle,
    client: mongodb::Client,
    db: String,
}

impl Server {
    fn of(f: &Fixture) -> Self {
        Self {
            handle: f.runtime().handle().clone(),
            client: f.db.client.clone(),
            db: f.db.name.clone(),
        }
    }

    /// The server logs every operation on the test's database from now on (level 2).
    fn profile_everything(&self) {
        let db = self.client.database(&self.db);
        self.handle
            .block_on(async { db.run_command(doc! {"profile": 2_i32}).await })
            .expect("profiling level 2");
    }

    /// How many finds on `collection` the server has run since profiling began.
    fn finds_on(&self, collection: &str) -> u64 {
        let profile = self
            .client
            .database(&self.db)
            .collection::<bson::Document>("system.profile");
        let filter = doc! {"op": "query", "ns": format!("{}.{collection}", self.db)};
        self.handle
            .block_on(async { profile.count_documents(filter).await })
            .expect("count the profile")
    }

    /// Whether the slow `$where` find on `things` of the test's database is still running.
    fn slow_find_running(&self) -> bool {
        let ns = format!("{}.{THINGS}", self.db);
        let admin = self.client.database("admin");
        let reply = self
            .handle
            .block_on(async {
                admin
                    .run_command(doc! {"currentOp": 1_i32, "active": true})
                    .await
            })
            .expect("currentOp");
        reply.get_array("inprog").is_ok_and(|ops| {
            ops.iter().filter_map(|op| op.as_document()).any(|op| {
                op.get_str("ns") == Ok(ns.as_str())
                    && op
                        .get_document("command")
                        .ok()
                        .and_then(|c| c.get_document("filter").ok())
                        .is_some_and(|filter| filter.contains_key("$where"))
            })
        })
    }
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
    let server = Server::of(&f);
    f.harness
        .state_mut()
        .dispatch(Input::Find(FindAction::Run(slow)));
    let slow_request = f.harness.state().state().issued_request;
    // The server has started it...
    wait_until(&mut f.harness, "the slow find on the server", SHORT, |_| {
        server.slow_find_running()
    });
    // ... and the tab is left and closed while it is busy.
    tab(&mut f, TabAction::Activate(words));
    tab(&mut f, TabAction::Close(things));
    assert!(!f.harness.state().state().tabs.owner.contains_key(&things));
    // The server finishes the find and its answer is queued for the app...
    wait_until(
        &mut f.harness,
        "the server to finish the find",
        SHORT,
        |_| !server.slow_find_running(),
    );
    // ... and the answers come in the order the runtime sent them: a find sent now from the open
    // tab is answered after the closed tab's, so its answer is the proof that the late one was
    // read (and dropped) by the app, with no fixed wait.
    let marker = Query {
        skip: 1,
        ..Query::default()
    };
    f.harness
        .state_mut()
        .dispatch(Input::Find(FindAction::Run(marker)));
    let marker_request = f.harness.state().state().issued_request;
    assert_ne!(marker_request, slow_request);
    wait_until(&mut f.harness, "the marker find's answer", SHORT, |h| {
        let s = h.state().state();
        !s.loading_page && s.page.as_ref().map(|p| p.docs.len()) == Some(2)
    });
    let s = f.harness.state().state();
    // What the drop leaves behind: the closed tab's request number is in no state.
    assert!(s.issued_request != slow_request && s.verdict_request != slow_request);
    assert!(
        s.parked_tabs
            .values()
            .all(|view| view.issued_request != slow_request),
        "a parked tab still awaits the closed tab's find"
    );
    assert_eq!(s.status, ConnStatus::Connected);
    assert!(s.banner.is_none(), "a late answer raised {:?}", s.banner);
    assert_eq!(
        s.page.as_ref().map(|p| p.docs.len()),
        Some(2),
        "the open tab shows its own marker find (3 documents, one skipped)"
    );
    assert_eq!(s.find.text.filter, "", "its find bar is untouched");
    assert_eq!(s.selected_ns().map(|n| n.collection.as_str()), Some(WORDS));
    assert_eq!(s.tabs_of(session(&f)).len(), 1);
    assert!(!s.loading_page, "no read is left waiting in the open tab");
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
    // From here the server logs every operation, to prove which finds it was asked for.
    let server = Server::of(&f);
    server.profile_everything();
    let (words_before, things_before) = (server.finds_on(WORDS), server.finds_on(THINGS));
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
    // The server agrees: neither tab's collection was read, the shown tab's or the parked one's.
    assert_eq!(server.finds_on(WORDS), words_before, "the parked tab read");
    assert_eq!(server.finds_on(THINGS), things_before, "the shown tab read");
    // Run wakes one tab; the other stays dormant until its own Run.
    tab(&mut f, TabAction::Run(things));
    wait_docs(&mut f, 2);
    let s = f.harness.state().state();
    assert_eq!(s.freshness_of(things), Freshness::Fresh);
    assert_eq!(s.freshness_of(words), Freshness::Dormant);
    assert_eq!(server.finds_on(THINGS), things_before + 1, "Run read once");
    assert_eq!(server.finds_on(WORDS), words_before, "and only its own tab");
}

/// The collection names of the `tab`s of the session, in strip order.
fn collections(f: &Fixture) -> Vec<String> {
    let s = f.harness.state().state();
    s.tabs_of(session(f))
        .into_iter()
        .filter_map(|tab| s.tab_ns(tab))
        .map(|ns| ns.collection)
        .collect()
}

/// A workspace file in its own folder, and the folder that keeps it alive.
fn workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("workspace.json");
    (dir, path)
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn restore_after_relaunch_reopens_tabs_and_text() {
    let mut f = fixture!();
    let (_keep, path) = workspace();
    f.use_workspace(&path);
    browse(&mut f, WORDS);
    let words = active(&f);
    let filter = f.labels.t("findbar.filter");
    fill(&mut f, &filter, r#"{"name": "abc"}"#);
    click_key(&mut f, "findbar.run");
    wait_docs(&mut f, 1);
    open_in_new_tab(&mut f, THINGS);
    wait_docs(&mut f, 2);
    let _ = words;
    f.relaunch(&path);
    // Before the person connects: nothing open, nothing in the strip, tabs waiting.
    {
        let s = f.harness.state().state();
        assert!(s.open.is_empty() && s.visible_tabs().is_empty());
        assert_eq!(s.restore.dormant.len(), 1);
    }
    connect(&mut f);
    assert_eq!(collections(&f), [WORDS, THINGS]);
    let s = f.harness.state().state();
    assert_eq!(
        s.selected_ns().map(|n| n.collection.as_str()),
        Some(THINGS),
        "the tab that was in front is in front again"
    );
    let restored_words = s.tabs_of(session(&f))[0];
    for tab in s.tabs_of(session(&f)) {
        assert_eq!(s.freshness_of(tab), Freshness::Dormant);
    }
    tab(&mut f, TabAction::Activate(restored_words));
    let s = f.harness.state().state();
    assert_eq!(s.find.text.filter, r#"{"name": "abc"}"#, "its text is back");
    assert!(s.page.is_none(), "and nothing was read for it");
    tab(&mut f, TabAction::Run(restored_words));
    wait_docs(&mut f, 1);
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn restore_sends_no_find_before_run() {
    let mut f = fixture!();
    let (_keep, path) = workspace();
    f.use_workspace(&path);
    browse(&mut f, WORDS);
    open_in_new_tab(&mut f, THINGS);
    wait_docs(&mut f, 2);
    // From here the server logs every operation, to prove which finds it was asked for.
    let server = Server::of(&f);
    server.profile_everything();
    let (words_before, things_before) = (server.finds_on(WORDS), server.finds_on(THINGS));
    f.relaunch(&path);
    connect(&mut f);
    // Show every restored tab: a dormant tab sends nothing when it is shown.
    for tab_id in f.harness.state().state().tabs_of(session(&f)) {
        tab(&mut f, TabAction::Activate(tab_id));
    }
    settle(&mut f.harness);
    assert_eq!(server.finds_on(WORDS), words_before, "a restored tab read");
    assert_eq!(
        server.finds_on(THINGS),
        things_before,
        "a restored tab read"
    );
    // Run is the one thing that reads, and it reads once.
    let words_tab = f.harness.state().state().tabs_of(session(&f))[0];
    tab(&mut f, TabAction::Run(words_tab));
    wait_docs(&mut f, 3);
    assert_eq!(server.finds_on(WORDS), words_before + 1, "Run read once");
    assert_eq!(
        server.finds_on(THINGS),
        things_before,
        "and only its own tab"
    );
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn restore_never_runs_a_write() {
    use curlybrace_ui::app::AggregateAction;
    use curlybrace_ui::viewmodel::CollectionTab;
    let mut f = fixture!();
    let (_keep, path) = workspace();
    f.use_workspace(&path);
    browse(&mut f, WORDS);
    let target = "cb_restore_target";
    let state = f.harness.state_mut();
    state.dispatch(Input::Tab(CollectionTab::Aggregate));
    state.dispatch(Input::Aggregate(AggregateAction::AddStage("$match".into())));
    state.dispatch(Input::Aggregate(AggregateAction::AddStage("$out".into())));
    let out = format!(r#"{{"$out": "{target}"}}"#);
    state.dispatch(Input::Aggregate(AggregateAction::Edit(1, out)));
    settle(&mut f.harness);
    let server = Server::of(&f);
    let exists = |server: &Server| {
        let db = server.client.database(&server.db);
        let names = server
            .handle
            .block_on(async { db.list_collection_names().await })
            .expect("list the collections");
        names.iter().any(|n| n == target)
    };
    assert!(!exists(&server), "typing the draft wrote nothing");
    f.relaunch(&path);
    connect(&mut f);
    settle(&mut f.harness);
    let s = f.harness.state().state();
    assert_eq!(s.aggregate.stages.len(), 2, "the draft is back");
    assert!(
        s.aggregate.stages[1].text.contains(target),
        "with its $out stage"
    );
    assert!(!exists(&server), "connecting and showing it wrote nothing");
}
