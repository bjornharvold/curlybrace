//! The History popup's actions in the reducer: loading into the find bar without running, and the
//! favourite dialogs.
use crate::app::explain_tests::{act, browsing};
use crate::app::state::AppState;
use crate::viewmodel::query_history::{Dialog, HistoryAction as H};
use crate::viewmodel::{FindAction, submit_find};
use curlybrace_core::pipeline_options::PipelineOptions;
use curlybrace_core::query_history::FindText;
use curlybrace_core::runtime::EMBEDDED_SESSION;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        sort: "{ \"b\": 1 }".into(),
        projection: "{ \"c\": 1 }".into(),
        skip: "2".into(),
        limit: "7".into(),
        options: PipelineOptions {
            comment: Some("why".into()),
            ..PipelineOptions::default()
        },
    }
}

/// `app.people` with two recent queries (the newer first).
fn with_history() -> AppState {
    let mut state = browsing();
    state.query_history.store = state
        .query_history
        .store
        .recorded(EMBEDDED_SESSION, "app", "people", text("{\"old\":1}"), 1)
        .unwrap()
        .recorded(EMBEDDED_SESSION, "app", "people", text("{\"new\":1}"), 2)
        .unwrap();
    state
}

fn history(state: AppState, action: H) -> (AppState, usize) {
    let (state, commands) = act(state, FindAction::History(action));
    (state, commands.len())
}

fn recent(state: &AppState) -> Vec<String> {
    state
        .query_history
        .store
        .history(EMBEDDED_SESSION, "app", "people")
        .iter()
        .map(|e| e.text.filter.clone())
        .collect()
}

fn favourites(state: &AppState) -> Vec<String> {
    state
        .query_history
        .store
        .favourites(EMBEDDED_SESSION)
        .iter()
        .map(|f| f.name.clone())
        .collect()
}

#[test]
fn loading_a_recent_query_fills_the_whole_bar_and_runs_nothing() {
    let (state, sent) = history(with_history(), H::LoadRecent(1));
    assert_eq!(sent, 0, "loading never runs the query");
    assert_eq!(state.find.text.filter, "{\"old\":1}");
    assert_eq!(state.find.text.sort, "{ \"b\": 1 }");
    assert_eq!(state.find.text.projection, "{ \"c\": 1 }");
    assert_eq!(
        (
            state.find.text.skip.as_str(),
            state.find.text.limit.as_str()
        ),
        ("2", "7")
    );
    assert_eq!(
        state.find.options.parsed().unwrap().comment.as_deref(),
        Some("why")
    );
    assert!(!state.loading_page, "no find is in flight");
    assert_eq!(recent(&state).len(), 2, "loading records nothing");
}

#[test]
fn editing_a_loaded_query_and_running_records_a_new_entry_and_leaves_the_stored_copy() {
    let (state, _) = history(with_history(), H::LoadRecent(1));
    let mut state = state;
    state.find.text.filter = "{\"old\":2}".into();
    let query = submit_find(&state.find).unwrap();
    let (state, commands) = act(state, FindAction::Run(query));
    assert!(!commands.is_empty());
    assert_eq!(
        recent(&state),
        ["{\"old\":2}", "{\"new\":1}", "{\"old\":1}"]
    );
}

#[test]
fn loading_a_favourite_fills_the_bar_and_the_favourite_is_unchanged() {
    let (state, _) = history(with_history(), H::StarAsked(0));
    let (state, _) = history(state, H::NameEdited("mine".into()));
    let (state, _) = history(state, H::Confirmed);
    let id = state.query_history.store.favourites(EMBEDDED_SESSION)[0].id;
    let before = state
        .query_history
        .store
        .favourites(EMBEDDED_SESSION)
        .to_vec();
    let (mut state, sent) = history(state, H::LoadFavourite(id));
    assert_eq!(sent, 0);
    assert_eq!(state.find.text.filter, "{\"new\":1}");
    state.find.text.filter = "{\"edited\":1}".into();
    assert_eq!(
        state.query_history.store.favourites(EMBEDDED_SESSION),
        before
    );
}

#[test]
fn loading_closes_the_search_and_a_missing_entry_changes_nothing() {
    let (state, _) = history(with_history(), H::Search("old".into()));
    assert_eq!(state.query_history.ui.search, "old");
    let (state, _) = history(state, H::LoadRecent(0));
    assert_eq!(state.query_history.ui.search, "", "a fresh popup next time");
    let before = state.find.text.clone();
    let (state, _) = history(state, H::LoadRecent(99));
    assert_eq!(state.find.text, before);
}

#[test]
fn nothing_loads_without_a_selected_collection() {
    let mut state = with_history();
    state.sidebar = Default::default();
    let (state, _) = history(state, H::LoadRecent(0));
    assert_eq!(state.find.text.filter, "");
}

#[test]
fn starring_asks_for_a_name_then_saves_a_favourite() {
    let (state, _) = history(with_history(), H::StarAsked(1));
    assert!(matches!(
        state.query_history.ui.dialog,
        Some(Dialog::Star { .. })
    ));
    let (state, _) = history(state, H::NameEdited("Old ones".into()));
    let (state, _) = history(state, H::Confirmed);
    assert_eq!(favourites(&state), ["Old ones"]);
    assert_eq!(state.query_history.ui.dialog, None);
    let fav = &state.query_history.store.favourites(EMBEDDED_SESSION)[0];
    assert_eq!(fav.text.filter, "{\"old\":1}", "the entry that was picked");
    assert_eq!(recent(&state).len(), 2, "history is kept");
}

#[test]
fn a_refused_name_stays_in_the_dialog_with_its_message() {
    let (state, _) = history(with_history(), H::StarAsked(0));
    let (state, _) = history(state, H::NameEdited("  ".into()));
    let (state, _) = history(state, H::Confirmed);
    assert_eq!(
        state.query_history.ui.error,
        Some("history.error.empty_name")
    );
    assert!(state.query_history.ui.dialog.is_some());
    assert!(favourites(&state).is_empty());

    let (state, _) = history(state, H::NameEdited("dup".into()));
    assert_eq!(state.query_history.ui.error, None, "editing clears it");
    let (state, _) = history(state, H::Confirmed);
    let (state, _) = history(state, H::StarAsked(1));
    let (state, _) = history(state, H::NameEdited("DUP".into()));
    let (state, _) = history(state, H::Confirmed);
    assert_eq!(
        state.query_history.ui.error,
        Some("history.error.name_taken")
    );
    assert_eq!(favourites(&state), ["dup"]);
}

#[test]
fn cancel_closes_the_dialog_without_a_change() {
    let (state, _) = history(with_history(), H::StarAsked(0));
    let (state, _) = history(state, H::DialogCancelled);
    assert_eq!(state.query_history.ui.dialog, None);
    assert!(favourites(&state).is_empty());
}

fn with_favourite() -> AppState {
    let (state, _) = history(with_history(), H::StarAsked(0));
    let (state, _) = history(state, H::NameEdited("first".into()));
    history(state, H::Confirmed).0
}

#[test]
fn rename_starts_with_the_current_name_and_changes_only_the_name() {
    let state = with_favourite();
    let id = state.query_history.store.favourites(EMBEDDED_SESSION)[0].id;
    let (state, _) = history(state, H::RenameAsked(id));
    match &state.query_history.ui.dialog {
        Some(Dialog::Rename { name, .. }) => assert_eq!(name, "first"),
        _ => panic!("a rename dialog"),
    }
    let (state, _) = history(state, H::NameEdited("second".into()));
    let (state, _) = history(state, H::Confirmed);
    assert_eq!(favourites(&state), ["second"]);
    assert_eq!(
        state.query_history.store.favourites(EMBEDDED_SESSION)[0]
            .text
            .filter,
        "{\"new\":1}"
    );
}

#[test]
fn delete_asks_then_removes_only_the_favourite() {
    let state = with_favourite();
    let id = state.query_history.store.favourites(EMBEDDED_SESSION)[0].id;
    let (state, _) = history(state, H::DeleteAsked(id));
    assert_eq!(favourites(&state), ["first"], "asking changes nothing");
    let (state, _) = history(state, H::Confirmed);
    assert!(favourites(&state).is_empty());
    assert_eq!(recent(&state).len(), 2);
}

#[test]
fn clear_history_asks_then_clears_this_collection_and_keeps_favourites() {
    let state = with_favourite();
    let (state, _) = history(state, H::ClearAsked);
    assert_eq!(recent(&state).len(), 2, "asking changes nothing");
    let (state, _) = history(state, H::Confirmed);
    assert!(recent(&state).is_empty());
    assert_eq!(favourites(&state), ["first"]);
}

#[test]
fn a_confirm_with_no_dialog_changes_nothing() {
    let state = with_history();
    let (state, _) = history(state, H::Confirmed);
    assert_eq!(recent(&state).len(), 2);
}
