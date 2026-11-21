//! The Browse page with query history on, for the screenshots and the layout tests: the history
//! file lives in a temp folder that is kept for the rest of the test run (never the developer's
//! real config directory), and the collection has recent queries and favourites.
use super::shots::{SHOT_NOW, browsing};
use curlybrace_core::query_history::{FindText, QueryHistory};
use curlybrace_ui::app::state::AppState;
use curlybrace_ui::viewmodel::ViewMode;

fn text(filter: &str) -> FindText {
    FindText {
        filter: filter.into(),
        ..FindText::default()
    }
}

/// A history file path in a folder that outlives the app (a test process is short-lived).
pub fn history_file() -> std::path::PathBuf {
    let dir = tempfile::tempdir().expect("temp dir").keep();
    dir.join("query_history.json")
}

/// The browser of `browsing` with the History button on: three recent queries and two favourites
/// for `crm.customers`.
pub fn history_page(state: &mut AppState) {
    browsing(ViewMode::Table)(state);
    *state = std::mem::take(state).with_query_history_path(Ok(history_file()));
    let connection = state.active.expect("an open session");
    let store = QueryHistory::default()
        .recorded(
            connection,
            "crm",
            "customers",
            text("{ \"status\": \"open\" }"),
            SHOT_NOW - 7_200,
        )
        .and_then(|s| {
            s.recorded(
                connection,
                "crm",
                "customers",
                text("{ \"city\": \"Oslo\", \"age\": { \"$gt\": 30 } }"),
                SHOT_NOW - 600,
            )
        })
        .and_then(|s| {
            s.recorded(
                connection,
                "crm",
                "customers",
                text("{ \"email\": { \"$regex\": \"@example.com$\" } }"),
                SHOT_NOW - 20,
            )
        })
        .and_then(|s| {
            s.starred(
                connection,
                "crm",
                "customers",
                "Open customers",
                text("{ \"status\": \"open\" }"),
                SHOT_NOW - 86_400,
            )
        })
        .and_then(|s| {
            s.starred(
                connection,
                "crm",
                "customers",
                "Oslo over thirty",
                text("{ \"city\": \"Oslo\", \"age\": { \"$gt\": 30 } }"),
                SHOT_NOW - 3_600,
            )
        })
        .expect("a usable sample");
    state.query_history.store = store;
}
