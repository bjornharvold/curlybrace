//! The query history across a restart: the REAL app and runtime, driven through the accessibility
//! tree by `egui_kittest`, against a real MongoDB. Tier 1 (a local server): every test writes to
//! its own throwaway `cbit_*` database and drops it, and keeps its history file in its own temp
//! folder. Run with `scripts/integration-tests.sh`, and ALWAYS point `CURLYBRACE_IT_MONGODB_URI`
//! at a throwaway server (the default is a developer's own MongoDB on 27017).
mod support;

use bson::doc;
use curlybrace_ui::app::reducer::Input;
use curlybrace_ui::app::tools_state::ToolsAction;
use std::path::Path;
use support::actions::*;
use support::fixture::Fixture;
use support::wait::{SHORT, settle, wait_until};

const WORDS: &str = "words";
const ABC: &str = r#"{"name": "abc"}"#;
const XYZ: &str = r#"{"name": "xyz"}"#;

/// The name of the enclosing test function (for the printed skip reason).
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
        // The guard and the verdict would hold or add reads: these flows are about the history.
        let state = f.harness.state_mut().state_mut();
        state.explain.guard = false;
        state.explain.automatic = false;
        f
    }};
}

/// Types `filter` in the bar and presses Find; waits for the page of that find.
fn run_filter(f: &mut Fixture, filter: &str, first_name: &str) {
    let label = f.labels.t("findbar.filter");
    fill(f, &label, filter);
    click_key(f, "findbar.run");
    let want = first_name.to_string();
    wait_until(&mut f.harness, "the page of the find", SHORT, move |h| {
        h.state()
            .state()
            .page
            .as_ref()
            .is_some_and(|p| p.docs.first().and_then(|d| d.get_str("name").ok()) == Some(&want))
    });
}

/// The filters in the history file for `words`, newest first.
fn filters_on_disk(path: &Path) -> Vec<String> {
    let bytes = std::fs::read(path).expect("the history file was written");
    let file: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON");
    file["history"][0]["entries"]
        .as_array()
        .expect("one namespace with entries")
        .iter()
        .map(|e| e["text"]["filter"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn a_query_run_before_a_restart_is_loaded_after_it_and_runs_only_on_run() {
    let mut f = fixture!();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("query_history.json");
    f.use_query_history(&path);
    browse(&mut f, WORDS);
    run_filter(&mut f, ABC, "abc");
    run_filter(&mut f, XYZ, "xyz");
    assert_eq!(
        filters_on_disk(&path),
        [XYZ, ABC],
        "both runs, newest first"
    );

    // Quit and start again on the same files.
    f.relaunch_with_history(&path);
    browse(&mut f, WORDS);
    let filter_label = f.labels.t("findbar.filter");
    assert_eq!(
        text_of(&f.harness, &filter_label),
        "",
        "a fresh bar: nothing was restored from the history"
    );
    let shown_before = f
        .harness
        .state()
        .state()
        .page
        .as_ref()
        .map(|p| p.docs.len());
    assert_eq!(shown_before, Some(3), "the whole collection is on screen");

    click_key(&mut f, "history.button");
    let row = f.labels.t_with("history.row.load", &[("text", ABC)]);
    f.click_text(&row);
    settle(&mut f.harness);
    let state = f.harness.state().state();
    assert_eq!(
        state.find.text.filter, ABC,
        "the bar holds the loaded query"
    );
    assert!(!state.loading_page, "loading did not send a find");
    assert_eq!(
        state.page.as_ref().map(|p| p.docs.len()),
        Some(3),
        "and the page is still the whole collection"
    );

    click_key(&mut f, "findbar.run");
    wait_until(&mut f.harness, "the page of the loaded query", SHORT, |h| {
        h.state()
            .state()
            .page
            .as_ref()
            .is_some_and(|p| p.docs.len() == 1)
    });
    let page = f.harness.state().state().page.clone().expect("a page");
    assert_eq!(page.docs[0].get_str("name").unwrap(), "abc");
    assert_eq!(
        filters_on_disk(&path),
        [ABC, XYZ],
        "running it moved it to the front, without a second copy"
    );
    assert_eq!(f.db.count(WORDS), 3, "a find never writes");
}

#[test]
#[ignore = "needs a local MongoDB: set CURLYBRACE_IT=1"]
fn switching_remember_queries_off_erases_the_file_and_stops_recording() {
    let mut f = fixture!();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("query_history.json");
    f.use_query_history(&path);
    browse(&mut f, WORDS);
    run_filter(&mut f, ABC, "abc");
    assert!(std::fs::read_to_string(&path).unwrap().contains("abc"));

    f.harness
        .state_mut()
        .dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    run_filter(&mut f, XYZ, "xyz");
    let bytes = std::fs::read_to_string(&path).unwrap();
    assert!(!bytes.contains("abc") && !bytes.contains("xyz"), "{bytes}");

    // It stays off after a restart, and the History popup says nothing is recorded.
    f.relaunch_with_history(&path);
    browse(&mut f, WORDS);
    click_key(&mut f, "history.button");
    let off = f.labels.t("history.empty.recent_off");
    wait_shows(&mut f, &off);
    let setting = std::fs::read_to_string(&path).unwrap();
    assert!(setting.contains("\"remember\": false"), "{setting}");
}
