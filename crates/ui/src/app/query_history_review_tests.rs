//! Lasting failures, an unreadable file and the "Remember queries" switch, dialogs that outlive
//! their collection, and changes made again after another window wrote the file.
use super::tests::{Keep, browsing_app, dispatch_run, file, history_app};
use super::{Op, stale_dialog};
use crate::app::explain_tests::{act, browsing};
use crate::app::persistence::{PersistGuard, Recovery};
use crate::app::query_history_store::{self, HistorySaveError};
use crate::app::reducer::Input;
use crate::app::state::Banner;
use crate::app::tools_state::ToolsAction;
use crate::app::*;
use crate::viewmodel::FindAction;
use crate::viewmodel::query_history::{Dialog, HistoryAction as H};
use curlybrace_core::query_history::FindText;
use curlybrace_core::runtime::EMBEDDED_SESSION;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

fn star_op(id: Uuid, name: &str) -> Op {
    Op::Star {
        id,
        connection: EMBEDDED_SESSION,
        db: "app".into(),
        collection: "people".into(),
        name: name.into(),
        text: Box::new(text("{}")),
        now: 1,
    }
}

fn on_disk(dir: &tempfile::TempDir) -> curlybrace_core::query_history::QueryHistory {
    query_history_store::load(&file(dir), &persistence::back_up).settings
}

const EMPTY: &str = "{\"version\": 1, \"history\": [], \"favourites\": []}";

fn app_with_blocked_file(dir: &tempfile::TempDir) -> (CurlybraceApp, Keep) {
    std::fs::write(file(dir), EMPTY).unwrap();
    let (mut app, keep) = browsing_app(dir);
    app.state_mut().query_history.guard = PersistGuard::blocked();
    app.state_mut().banner = None;
    (app, keep)
}

// A lasting failure is not retried on every step.

#[test]
fn a_lasting_failure_is_not_retried_by_steps_that_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app_with_blocked_file(&dir);
    dispatch_run(&mut app, "{\"a\":1}");
    // The cause is gone, but nothing about the history or the file changed: no retry.
    app.state_mut().query_history.guard = PersistGuard::default();
    for _ in 0..5 {
        app.dispatch(Input::DismissBanner);
    }
    assert_eq!(
        std::fs::read_to_string(file(&dir)).unwrap(),
        EMPTY,
        "no retry"
    );
    // A change of the history is a reason to try again.
    dispatch_run(&mut app, "{\"b\":1}");
    assert_eq!(on_disk(&dir).history_len(), 2, "tried again and written");
}

#[test]
fn a_change_of_the_file_is_a_reason_to_try_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app_with_blocked_file(&dir);
    dispatch_run(&mut app, "{\"a\":1}");
    app.state_mut().query_history.guard = PersistGuard::default();
    app.dispatch(Input::DismissBanner);
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), EMPTY);
    // Another program replaces the file (a longer text, so its size differs).
    std::fs::write(file(&dir), format!("{EMPTY}\n\n")).unwrap();
    app.dispatch(Input::DismissBanner);
    assert_eq!(on_disk(&dir).history_len(), 1, "the next step wrote it");
}

#[test]
fn the_queue_of_unwritten_changes_is_bounded_and_keeps_favourite_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = app_with_blocked_file(&dir);
    for i in 0..3 {
        let state = app.state_mut();
        state
            .query_history_apply(star_op(Uuid::from_u128(i + 1), &format!("fav{i}")))
            .unwrap();
    }
    for i in 0..100 {
        dispatch_run(&mut app, &format!("{{\"n\":{i}}}"));
    }
    let qh = &app.state().query_history;
    assert!(qh.pending_records() <= 32, "{}", qh.pending_records());
    assert_eq!(
        qh.pending_len() - qh.pending_records(),
        3,
        "favourites stay"
    );
    assert_eq!(
        qh.store.history_len(),
        100,
        "the history itself is all there"
    );
}

// An unreadable file does not turn recording back on.

#[test]
fn a_newer_file_is_left_alone_and_a_favourite_change_says_it_was_not_saved() {
    let dir = tempfile::tempdir().unwrap();
    let newer = r#"{"version": 99, "history": [], "favourites": []}"#;
    std::fs::write(file(&dir), newer).unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    app.state_mut().banner = None;
    app.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "mine"))
        .unwrap();
    app.dispatch(Input::DismissBanner);
    assert_eq!(std::fs::read_to_string(file(&dir)).unwrap(), newer);
    assert_eq!(
        app.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::NewerVersion))
    );
}

#[test]
fn a_corrupt_file_pauses_recording_until_the_switch_is_turned_on() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{ nope").unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    assert!(!app.state().query_history.recording());
    dispatch_run(&mut app, "{\"a\":1}");
    assert_eq!(
        app.state().query_history.store.history_len(),
        0,
        "not recorded"
    );
    assert_eq!(
        std::fs::read_to_string(file(&dir)).unwrap(),
        "{ nope",
        "not rewritten"
    );
    // Turning the switch on is the person's decision to start again.
    app.dispatch(Input::Tools(ToolsAction::SetRememberQueries(true)));
    assert!(app.state().query_history.recording());
    dispatch_run(&mut app, "{\"b\":1}");
    assert_eq!(on_disk(&dir).history_len(), 1);
}

#[test]
fn a_newer_file_pauses_recording_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(file(&dir), "{\"version\": 99}").unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    assert!(!app.state().query_history.recording());
    dispatch_run(&mut app, "{\"a\":1}");
    assert_eq!(app.state().query_history.store.history_len(), 0);
}

#[test]
fn the_recovery_notice_says_recording_is_paused() {
    let i18n = I18n::english();
    for key in [
        Recovery::QueryHistory { backed_up: true }.i18n_key(),
        Recovery::QueryHistory { backed_up: false }.i18n_key(),
        Recovery::QueryHistoryNewer.i18n_key(),
    ] {
        let text = i18n.t(key);
        assert!(text.contains("Remember queries"), "{key}: {text}");
    }
    assert!(
        i18n.t("persist.query_history_backed_up")
            .contains("deleted"),
        "the notice says the backup is removed when the switch is turned off"
    );
}

#[test]
fn a_file_deleted_while_running_keeps_the_switch_that_was_set() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, _keep) = browsing_app(&dir);
    app.dispatch(Input::Tools(ToolsAction::SetRememberQueries(false)));
    assert!(
        std::fs::read_to_string(file(&dir))
            .unwrap()
            .contains("\"remember\": false")
    );
    std::fs::remove_file(file(&dir)).unwrap();
    // A change that must be written on top of what the file now holds (nothing).
    app.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(5), "keep"))
        .unwrap();
    app.dispatch(Input::DismissBanner);
    assert!(!app.state().query_history.store.remembering());
    let written = on_disk(&dir);
    assert!(!written.remembering(), "the file still says off");
    assert_eq!(written.favourites(EMBEDDED_SESSION).len(), 1);
}

// Dialogs belong to the collection they were opened for.

fn open_clear_on_people() -> crate::app::state::AppState {
    let mut state = browsing();
    state.query_history.store = state
        .query_history
        .store
        .recorded(EMBEDDED_SESSION, "app", "people", text("{\"a\":1}"), 1)
        .unwrap()
        .recorded(EMBEDDED_SESSION, "app", "pets", text("{\"b\":1}"), 2)
        .unwrap();
    act(state, FindAction::History(H::ClearAsked)).0
}

fn history_len(state: &crate::app::state::AppState, coll: &str) -> usize {
    state
        .query_history
        .store
        .history(EMBEDDED_SESSION, "app", coll)
        .len()
}

#[test]
fn a_dialog_confirms_against_the_collection_it_was_opened_for_or_not_at_all() {
    let state = open_clear_on_people();
    assert!(!stale_dialog(&state));
    let mut moved = state;
    moved.sidebar = moved.sidebar.selecting(Namespace::new("app", "pets"));
    assert!(
        stale_dialog(&moved),
        "the collection changed under the dialog"
    );
    let (after, _) = act(moved, FindAction::History(H::Confirmed));
    assert_eq!(
        history_len(&after, "pets"),
        1,
        "the other collection is not cleared"
    );
    assert_eq!(
        history_len(&after, "people"),
        1,
        "and nothing was cleared at all"
    );
    assert!(
        after.query_history.ui.dialog.is_none(),
        "the dialog is closed"
    );
}

#[test]
fn a_dialog_whose_collection_is_gone_is_stale_too() {
    let mut state = open_clear_on_people();
    state.sidebar = Default::default();
    assert!(stale_dialog(&state));
    let (after, _) = act(state, FindAction::History(H::Confirmed));
    assert!(after.query_history.ui.dialog.is_none());
}

#[test]
fn a_dialog_on_its_own_collection_still_confirms() {
    let state = open_clear_on_people();
    let (after, _) = act(state, FindAction::History(H::Confirmed));
    assert_eq!(history_len(&after, "people"), 0);
    assert_eq!(history_len(&after, "pets"), 1);
}

#[test]
fn an_open_history_dialog_counts_as_a_dialog_for_the_tab_shortcuts() {
    let mut state = browsing();
    assert!(!state.any_dialog_open());
    state.query_history.ui.dialog = Some(Dialog::Clear);
    assert!(state.any_dialog_open());
}

// Changes made again on top of another window's file.

#[test]
fn a_favourite_keeps_its_id_when_it_is_made_again_on_top_of_another_windows_file() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    dispatch_run(&mut b, "{\"from\":\"b\"}");
    let id = Uuid::from_u128(4242);
    a.state_mut()
        .query_history_apply(star_op(id, "mine"))
        .unwrap();
    a.dispatch(Input::DismissBanner);
    let written = on_disk(&dir);
    assert_eq!(written.favourites(EMBEDDED_SESSION)[0].id, id);
    assert_eq!(
        written.history_len(),
        1,
        "the other window's run is there too"
    );
}

#[test]
fn a_change_that_cannot_be_made_again_is_shown_not_only_logged() {
    let dir = tempfile::tempdir().unwrap();
    let (mut a, _ka) = browsing_app(&dir);
    let (mut b, _kb) = browsing_app(&dir);
    b.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(1), "same name"))
        .unwrap();
    b.dispatch(Input::DismissBanner);
    // The first window does not know and saves the same name: it succeeded there.
    a.state_mut()
        .query_history_apply(star_op(Uuid::from_u128(2), "Same Name"))
        .unwrap();
    a.state_mut().banner = None;
    a.dispatch(Input::DismissBanner);
    assert_eq!(
        a.state().banner,
        Some(Banner::QueryHistoryPersist(HistorySaveError::ReplayDropped))
    );
    assert_eq!(on_disk(&dir).favourites(EMBEDDED_SESSION).len(), 1);
}

#[test]
fn history_app_helper_is_usable() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _keep) = history_app(&dir);
    assert!(app.state().query_history.recording());
}
