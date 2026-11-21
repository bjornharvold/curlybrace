//! Query history inside the app: what is recorded and when (only where a find goes out), the
//! "Remember queries" switch, the file written from the dispatch loop, and a connection that is
//! deleted or duplicated. Every file lives in a tempdir.
use crate::app::explain_tests::{act, browsing, ns};
use crate::app::guard_tests::{checked, guarded, scan_report};
use crate::app::persistence::{PersistGuard, Recovery};
use crate::app::pipelines_wiring_tests::app_in;
use crate::app::query_history_store::{self, HistorySaveError};
use crate::app::reducer::{Input, update};
use crate::app::state::{AppState, Banner};
use crate::app::tools_state::ToolsAction;
use crate::app::*;
use crate::viewmodel::{FindAction, submit_find};
use curlybrace_core::pipeline_options::PipelineOptions;
use curlybrace_core::query_history::{FindText, QueryHistory};
use curlybrace_core::runtime::EMBEDDED_SESSION;
use uuid::Uuid;

fn typed(state: &mut AppState, filter: &str) -> curlybrace_core::store::Query {
    state.find.text.filter = filter.into();
    submit_find(&state.find).expect("the test text parses")
}

fn recorded(state: &AppState) -> Vec<FindText> {
    state
        .query_history
        .store
        .history(EMBEDDED_SESSION, "app", "people")
        .iter()
        .map(|e| e.text.clone())
        .collect()
}

fn run_typed(state: AppState, filter: &str) -> AppState {
    let mut state = state;
    let query = typed(&mut state, filter);
    act(state, FindAction::Run(query)).0
}

fn remember(state: AppState, on: bool) -> AppState {
    update(state, Input::Tools(ToolsAction::SetRememberQueries(on))).0
}

#[test]
fn a_run_records_what_was_typed_with_its_options() {
    let mut state = browsing();
    state.find.text.limit = "5".into();
    state.find.options =
        crate::viewmodel::aggregate_options::OptionsForm::from_options(&PipelineOptions {
            comment: Some("why".into()),
            ..PipelineOptions::default()
        });
    let state = run_typed(state, "{ \"age\": 1 }");
    let entries = recorded(&state);
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].filter, "{ \"age\": 1 }",
        "as typed, not re-formatted"
    );
    assert_eq!(entries[0].limit, "5");
    assert_eq!(entries[0].options.comment.as_deref(), Some("why"));
}

#[test]
fn running_the_same_query_again_keeps_one_entry() {
    let state = run_typed(run_typed(browsing(), "{\"a\":1}"), "{\"a\":1}");
    assert_eq!(recorded(&state).len(), 1);
}

#[test]
fn a_blank_bar_is_not_recorded() {
    let state = act(browsing(), FindAction::Run(Default::default())).0;
    assert!(recorded(&state).is_empty());
}

/// A run is on record and a page after the first is showing, so Previous and Next are allowed.
fn paged() -> AppState {
    let mut state = run_typed(browsing(), "{\"a\":1}");
    state.page = Some(curlybrace_core::store::Page {
        docs: Vec::new(),
        skip: 100,
        has_more: true,
    });
    state.page_query = curlybrace_core::store::Query {
        skip: 100,
        ..Default::default()
    };
    state.find.text.filter = "{\"changed\":1}".into();
    state
}

#[test]
fn previous_next_and_clear_do_not_record() {
    for action in [FindAction::Next, FindAction::Prev, FindAction::Clear] {
        let (next, commands) = act(paged(), action);
        assert!(!commands.is_empty(), "the action sent a find");
        assert_eq!(recorded(&next).len(), 1, "still the one run");
    }
}

#[test]
fn a_run_with_no_connection_is_not_recorded() {
    let mut state = browsing();
    state.status = crate::app::state::ConnStatus::Disconnected;
    let state = run_typed(state, "{\"a\":1}");
    assert!(recorded(&state).is_empty(), "nothing went out");
}

#[test]
fn a_guard_check_records_nothing_until_run_anyway_and_then_once() {
    let mut state = guarded();
    let query = typed(&mut state, "{\"a\":1}");
    let (state, _) = act(state, FindAction::Run(query.clone()));
    assert!(recorded(&state).is_empty(), "the find is held back");
    let request = state.guard_request;
    let (state, _) = checked(state, request, Ok(scan_report(query)));
    assert!(recorded(&state).is_empty(), "the warning is not a run");
    let (state, commands) = act(state, FindAction::GuardRunAnyway);
    assert!(!commands.is_empty());
    assert_eq!(recorded(&state).len(), 1);
}

#[test]
fn a_cancelled_guard_warning_records_nothing() {
    let mut state = guarded();
    let query = typed(&mut state, "{\"a\":1}");
    let (state, _) = act(state, FindAction::Run(query.clone()));
    let request = state.guard_request;
    let (state, _) = checked(state, request, Ok(scan_report(query)));
    let (state, _) = act(state, FindAction::GuardCancel);
    assert!(recorded(&state).is_empty());
}

#[test]
fn a_clean_guard_check_goes_on_and_records_once() {
    let mut state = guarded();
    let query = typed(&mut state, "{\"a\":1}");
    let (state, _) = act(state, FindAction::Run(query));
    let request = state.guard_request;
    let (state, commands) = checked(
        state,
        request,
        Ok(curlybrace_core::guard::GuardReport {
            findings: Vec::new(),
            unchecked: 0,
        }),
    );
    assert!(!commands.is_empty());
    assert_eq!(recorded(&state).len(), 1);
}

#[test]
fn a_bar_edited_while_the_find_was_held_back_is_not_recorded_under_the_wrong_text() {
    let mut state = guarded();
    let query = typed(&mut state, "{\"a\":1}");
    let (mut state, _) = act(state, FindAction::Run(query.clone()));
    let request = state.guard_request;
    let (s, _) = checked(state, request, Ok(scan_report(query)));
    state = s;
    state.find.text.filter = "{\"b\":2}".into();
    let (state, commands) = act(state, FindAction::GuardRunAnyway);
    assert!(!commands.is_empty(), "the held-back find still goes out");
    assert!(
        recorded(&state).is_empty(),
        "the bar no longer says what was sent"
    );
}

#[test]
fn remembering_off_records_nothing_and_erases_what_was_kept() {
    let state = run_typed(browsing(), "{\"a\":1}");
    let state = remember(state, false);
    assert!(recorded(&state).is_empty(), "erased at once");
    let state = run_typed(state, "{\"b\":1}");
    assert!(recorded(&state).is_empty(), "nothing recorded while off");
    let state = remember(state, true);
    let state = run_typed(state, "{\"c\":1}");
    assert_eq!(recorded(&state).len(), 1, "starts empty, then records");
}

#[test]
fn remembering_off_keeps_the_favourites() {
    let mut state = browsing();
    state.query_history.store = QueryHistory::default()
        .starred(
            EMBEDDED_SESSION,
            "app",
            "people",
            "keep",
            FindText::default(),
            1,
        )
        .unwrap();
    let state = remember(state, false);
    assert_eq!(
        state.query_history.store.favourites(EMBEDDED_SESSION).len(),
        1
    );
}

/// The channel ends the app talks to the runtime through: dropped, a send would fail and raise
/// the "runtime gone" banner these tests look at.
type Keep = (
    tokio::sync::mpsc::UnboundedSender<curlybrace_core::runtime::Event>,
    tokio::sync::mpsc::UnboundedReceiver<curlybrace_core::runtime::Command>,
);

fn history_app(dir: &tempfile::TempDir) -> (CurlybraceApp, Keep) {
    let (app, events, commands) = app_in(dir, Vec::new());
    let app = app.with_query_history_path(Ok(dir.path().join("query_history.json")));
    (app, (events, commands))
}

fn file(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("query_history.json")
}

fn browsing_app(dir: &tempfile::TempDir) -> (CurlybraceApp, Keep) {
    let (mut app, keep) = history_app(dir);
    let state = std::mem::take(app.state_mut());
    let browse = browsing();
    *app.state_mut() = AppState {
        query_history: state.query_history,
        pipelines_path: state.pipelines_path,
        ..browse
    };
    (app, keep)
}

fn dispatch_run(app: &mut CurlybraceApp, filter: &str) {
    app.state_mut().find.text.filter = filter.into();
    let query = submit_find(&app.state().find).unwrap();
    app.dispatch(Input::Find(FindAction::Run(query)));
}

#[test]
fn a_run_reaches_a_private_file_before_the_next_launch() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    dispatch_run(&mut app, "{\"age\": 7}");
    let on_disk = query_history_store::load(&file(&dir), &persistence::back_up).settings;
    assert_eq!(
        on_disk.history(EMBEDDED_SESSION, "app", "people")[0]
            .text
            .filter,
        "{\"age\": 7}"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(file(&dir)).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // A new window loads it.
    let (again, _k) = history_app(&dir);
    assert_eq!(again.state().query_history.store.history_len(), 1);
}

#[test]
fn nothing_is_written_when_nothing_was_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    app.dispatch(Input::DismissBanner);
    app.dispatch(Input::Find(FindAction::Clear));
    assert!(
        !file(&dir).exists(),
        "an idle or non-recording step writes no file"
    );
}

#[test]
fn off_writes_no_query_text_and_erases_it_from_the_file_and_its_backups() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    dispatch_run(&mut app, "{\"secret\": \"SENTINEL\"}");
    assert!(
        std::fs::read_to_string(file(&dir))
            .unwrap()
            .contains("SENTINEL")
    );
    // A backup copy of the old file also holds the text.
    std::fs::write(
        dir.path().join("query_history.json.corrupt-1"),
        "{\"secret\":\"SENTINEL\"}",
    )
    .unwrap();
    app.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    dispatch_run(&mut app, "{\"secret\": \"SENTINEL\"}");
    let bytes = std::fs::read_to_string(file(&dir)).unwrap();
    assert!(
        !bytes.contains("SENTINEL"),
        "off: no query text on disk: {bytes}"
    );
    assert!(bytes.contains("\"remember\": false"));
    assert!(!dir.path().join("query_history.json.corrupt-1").exists());
    // The setting survives a restart.
    let (again, _k) = history_app(&dir);
    assert!(!again.state().query_history.store.remembering());
}

#[test]
fn a_corrupt_file_is_backed_up_with_a_recovery_banner() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (app, _k) = history_app(&dir);
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::QueryHistory { backed_up: true }))
    );
    assert_eq!(app.state().query_history.store, QueryHistory::default());
}

#[test]
fn a_newer_file_is_left_alone_and_a_run_says_it_was_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let text = r#"{"version": 99, "history": [], "favourites": []}"#;
    std::fs::write(file(&dir), text).unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    app.state_mut().banner = None;
    dispatch_run(&mut app, "{\"a\":1}");
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), text);
    assert_eq!(
        app.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::NewerVersion))
    );
}

#[test]
fn a_blocked_guard_keeps_the_original_and_shows_one_banner() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "[]").unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    app.state_mut().query_history.guard = PersistGuard::blocked();
    app.state_mut().banner = None;
    dispatch_run(&mut app, "{\"a\":1}");
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), "[]");
    assert_eq!(
        app.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::Blocked))
    );
    app.state_mut().banner = None;
    dispatch_run(&mut app, "{\"b\":1}");
    assert_eq!(app.state().banner, None, "the streak was announced once");
}

#[test]
fn another_window_s_change_is_merged_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    dispatch_run(&mut a, "{\"from\":\"a\"}");
    dispatch_run(&mut b, "{\"from\":\"b\"}");
    let on_disk = query_history_store::load(&file(&dir), &persistence::back_up).settings;
    let filters: Vec<String> = on_disk
        .history(EMBEDDED_SESSION, "app", "people")
        .iter()
        .map(|e| e.text.filter.clone())
        .collect();
    assert_eq!(filters, ["{\"from\":\"b\"}", "{\"from\":\"a\"}"]);
    assert_eq!(b.state().query_history.store, on_disk);
}

#[test]
fn an_app_without_a_path_keeps_the_history_in_memory_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _e, _c) = app_in(&dir, Vec::new());
    *app.state_mut() = browsing();
    dispatch_run(&mut app, "{\"a\":1}");
    assert_eq!(app.state().query_history.store.history_len(), 1);
    assert_eq!(app.state().banner, None);
    assert!(!file(&dir).exists());
}

fn two_connections() -> (ConnectionConfig, ConnectionConfig) {
    (
        ConnectionConfig::new("local", "mongodb://localhost:27017"),
        ConnectionConfig::new("other", "mongodb://localhost:27018"),
    )
}

fn seeded(id: Uuid, other: Uuid) -> QueryHistory {
    let text = |f: &str| FindText {
        filter: f.into(),
        ..FindText::default()
    };
    QueryHistory::default()
        .recorded(id, "d", "c", text("mine"), 1)
        .unwrap()
        .starred(id, "d", "c", "fav", text("mine"), 2)
        .unwrap()
        .recorded(other, "d", "c", text("theirs"), 3)
        .unwrap()
}

#[test]
fn deleting_a_connection_drops_its_history_and_favourites_only() {
    let dir = tempfile::tempdir().unwrap();
    let (cfg, other) = two_connections();
    let (id, other_id) = (cfg.id, other.id);
    let (app, _e, _c) = app_in(&dir, vec![cfg, other]);
    let mut app = app.with_query_history_path(Ok(file(&dir)));
    app.state_mut().query_history.store = seeded(id, other_id);
    app.dispatch_connection(ConnectionAction::Delete(id));
    let store = &app.state().query_history.store;
    assert!(store.history(id, "d", "c").is_empty());
    assert!(store.favourites(id).is_empty());
    assert_eq!(store.history(other_id, "d", "c").len(), 1);
    let on_disk = query_history_store::load(&file(&dir), &persistence::back_up).settings;
    assert_eq!(&on_disk, store);
}

#[test]
fn a_delete_that_was_not_written_keeps_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (app, _e, _c) = app_in(&dir, vec![cfg]);
    let mut app = app.with_query_history_path(Ok(file(&dir)));
    app.state_mut().persist = PersistGuard::blocked();
    app.state_mut().query_history.store = seeded(id, Uuid::new_v4());
    app.dispatch_connection(ConnectionAction::Delete(id));
    assert!(
        app.state().connection(id).is_some(),
        "the delete was refused"
    );
    assert_eq!(app.state().query_history.store.favourites(id).len(), 1);
}

#[test]
fn a_duplicate_copies_favourites_but_not_history() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let (app, _e, _c) = app_in(&dir, vec![cfg]);
    let mut app = app.with_query_history_path(Ok(file(&dir)));
    app.state_mut().query_history.store = seeded(id, Uuid::new_v4());
    app.dispatch_connection(ConnectionAction::Duplicate(id));
    let copy = app
        .state()
        .conn
        .connections
        .iter()
        .find(|c| c.id != id)
        .expect("the copy")
        .id;
    let store = &app.state().query_history.store;
    assert_eq!(store.favourites(copy)[0].name, "fav");
    assert!(store.history(copy, "d", "c").is_empty());
    assert_eq!(store.history(id, "d", "c").len(), 1);
    let on_disk = query_history_store::load(&file(&dir), &persistence::back_up).settings;
    assert_eq!(&on_disk, store);
}

#[test]
fn the_bar_text_round_trips_through_the_stored_form() {
    let mut state = browsing();
    state.find.text.sort = "{ \"a\": 1 }".into();
    state.find.text.skip = "3".into();
    let text = crate::app::query_history_app::find_text_of(&state.find);
    let bar = crate::app::query_history_app::bar_of(&text);
    assert_eq!(bar.text, state.find.text);
    assert_eq!(bar.options, state.find.options);
    let _ = ns();
}
