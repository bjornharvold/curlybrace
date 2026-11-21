//! Query history logging audit (#72): whatever goes wrong with the history (a refused write, a
//! file another window changed, a file that cannot be read, a switch turned off), no log line
//! contains the text of a query, the name of a favourite or the namespace it was run in, and no
//! `Debug` print of the state does either. A capturing logger sees every record at every level.
use crate::app::explain_tests::{act, browsing};
use crate::app::persistence::PersistGuard;
use crate::app::pipelines_wiring_tests::app_in;
use crate::app::query_history_app::on_action;
use crate::app::state::AppState;
use crate::app::tools_state::ToolsAction;
use crate::app::*;
use crate::log_capture::{capture, records_mentioning};
use crate::viewmodel::query_history::HistoryAction as H;
use crate::viewmodel::{FindAction, submit_find};
use curlybrace_core::runtime::EMBEDDED_SESSION;

const QUERY: &str = "SENTINEL-QUERY-7f3a";
const NAME: &str = "SENTINEL-NAME-7f3a";
const DB: &str = "sentineldb7f3a";
const COLLECTION: &str = "sentinelcoll7f3a";
const WORDS: [&str; 4] = [QUERY, NAME, DB, COLLECTION];

type Keep = (
    tokio::sync::mpsc::UnboundedSender<curlybrace_core::runtime::Event>,
    tokio::sync::mpsc::UnboundedReceiver<curlybrace_core::runtime::Command>,
);

fn app(dir: &tempfile::TempDir) -> (CurlybraceApp, Keep) {
    let (app, events, commands) = app_in(dir, Vec::new());
    let mut app = app.with_query_history_path(Ok(dir.path().join("query_history.json")));
    let mut state = browsing();
    state.sidebar = state
        .sidebar
        .with_databases(vec![DB.into()])
        .with_collections(DB, vec![COLLECTION.into()])
        .toggled(DB)
        .selecting(curlybrace_core::store::Namespace::new(DB, COLLECTION));
    let current = std::mem::take(app.state_mut());
    state.query_history = current.query_history;
    state.pipelines_path = current.pipelines_path;
    *app.state_mut() = state;
    (app, (events, commands))
}

fn run(app: &mut CurlybraceApp, filter: &str) {
    app.state_mut().find.text.filter = filter.into();
    let query = submit_find(&app.state().find).unwrap();
    app.dispatch(Input::Find(FindAction::Run(query)));
}

fn star(app: &mut CurlybraceApp, name: &str) {
    for action in [H::StarAsked(0), H::NameEdited(name.into()), H::Confirmed] {
        app.dispatch(Input::Find(FindAction::History(action)));
    }
}

fn assert_nothing_leaked(what: &str) {
    for word in WORDS {
        let leaked = records_mentioning(word);
        assert!(
            leaked.is_empty(),
            "{what}: a log line holds {word}: {leaked:?}"
        );
    }
}

fn quoted(filter_key: &str) -> String {
    format!("{{\"{filter_key}\": 1}}")
}

#[test]
fn a_normal_run_a_star_and_a_switch_log_nothing_of_the_query() {
    capture();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app(&dir);
    run(&mut app, &quoted(QUERY));
    star(&mut app, NAME);
    app.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    run(&mut app, &quoted(QUERY));
    assert_nothing_leaked("normal use");
}

#[test]
fn a_refused_write_is_logged_by_kind_only() {
    capture();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app(&dir);
    app.state_mut().query_history.guard = PersistGuard::blocked();
    run(&mut app, &quoted(QUERY));
    star(&mut app, NAME);
    assert!(
        !records_mentioning("could not save the query history").is_empty(),
        "the failure was logged (so this test is not vacuous)"
    );
    assert_nothing_leaked("a blocked file");
}

#[test]
fn a_file_another_window_changed_or_broke_is_logged_without_text() {
    capture();
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = app(&dir);
    let (mut b, _kb) = app(&dir);
    run(&mut a, &quoted(QUERY));
    run(&mut b, &quoted(QUERY));
    // Then the file is damaged under both windows, and one of them backs it up and goes on.
    std::fs::write(dir.path().join("query_history.json"), "{ not json").unwrap();
    run(&mut a, &quoted("again"));
    // A backup that cannot be made blocks saving: the logged reason has no text either.
    std::fs::write(dir.path().join("query_history.json"), "{ still not json").unwrap();
    a.state_mut().query_history.seen = Default::default();
    run(&mut a, &quoted(NAME));
    assert_nothing_leaked("a changed or damaged file");
}

#[test]
fn a_file_that_cannot_be_used_at_startup_is_logged_without_text() {
    capture();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("query_history.json"),
        format!("{{\"version\": 1, \"history\": \"{QUERY}\"}}"),
    )
    .unwrap();
    let (app, _events, _commands) = app_in(&dir, Vec::new());
    let app = app.with_query_history_path(Ok(dir.path().join("query_history.json")));
    assert!(app.state().banner.is_some(), "the notice was raised");
    assert_nothing_leaked("a damaged file");
}

#[test]
fn a_too_large_query_is_logged_by_kind_only() {
    capture();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app(&dir);
    let big = format!("{QUERY}{}", "x".repeat(300 * 1024));
    run(&mut app, &format!("{{\"a\": \"{big}\"}}"));
    assert!(
        !records_mentioning("query not recorded").is_empty(),
        "the refusal was logged (so this test is not vacuous)"
    );
    assert_nothing_leaked("an over-large query");
}

#[test]
fn a_taken_name_and_a_failed_confirm_log_nothing_of_the_name() {
    capture();
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app(&dir);
    run(&mut app, &quoted(QUERY));
    star(&mut app, NAME);
    run(&mut app, &quoted("other"));
    star(&mut app, NAME);
    assert_nothing_leaked("a taken name");
}

#[test]
fn debug_prints_of_the_state_and_actions_hold_no_text() {
    let mut state = browsing();
    state.find.text.filter = quoted(QUERY);
    let query = submit_find(&state.find).unwrap();
    let (state, _) = act(state, FindAction::Run(query));
    let (state, _) = on_action(state, H::StarAsked(0));
    let (state, _) = on_action(state, H::NameEdited(NAME.into()));
    let shown = format!(
        "{:?} {:?} {:?}",
        state.query_history,
        H::NameEdited(NAME.into()),
        H::Search(QUERY.into())
    );
    for word in [QUERY, NAME] {
        assert!(!shown.contains(word), "{word} in {shown}");
    }
    // The store's own text is there to be found by the person, just never printed.
    assert_eq!(
        state
            .query_history
            .store
            .history(EMBEDDED_SESSION, "app", "people")
            .len(),
        1
    );
    let _: &AppState = &state;
}
