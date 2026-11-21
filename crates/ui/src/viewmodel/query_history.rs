//! The History popup's model (#72): what it lists for a search, the dialogs it opens and the
//! actions it emits. Pure: nothing here touches a file or logs. Queries can hold personal
//! values, so no `Debug` here prints a query, a name or a namespace.
use crate::I18n;
use crate::viewmodel::index_detail::Age;
use curlybrace_core::query_history::{Entry, Favourite, FindText, QueryHistory, SavedError};
use curlybrace_core::saved_pipeline::{same_name, validate_name};
use uuid::Uuid;

/// What the person did in the popup or its dialogs.
#[derive(Clone, PartialEq)]
pub enum HistoryAction {
    /// The search box was edited; the value is the new text.
    Search(String),
    /// Load the recent query at this position of the collection's list into the find bar.
    LoadRecent(usize),
    LoadFavourite(Uuid),
    /// Save the recent query at this position as a favourite (asks for a name).
    StarAsked(usize),
    RenameAsked(Uuid),
    DeleteAsked(Uuid),
    /// Clear this collection's recent queries (asks first).
    ClearAsked,
    /// The name field of the open dialog was edited.
    NameEdited(String),
    /// The open dialog was confirmed.
    Confirmed,
    DialogCancelled,
}

/// Only the kind, and the position of a row: the text and the names are the person's.
impl std::fmt::Debug for HistoryAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoadRecent(i) => write!(f, "HistoryAction::LoadRecent({i})"),
            Self::StarAsked(i) => write!(f, "HistoryAction::StarAsked({i})"),
            Self::Search(_) => f.write_str("HistoryAction::Search"),
            Self::LoadFavourite(_) => f.write_str("HistoryAction::LoadFavourite"),
            Self::RenameAsked(_) => f.write_str("HistoryAction::RenameAsked"),
            Self::DeleteAsked(_) => f.write_str("HistoryAction::DeleteAsked"),
            Self::ClearAsked => f.write_str("HistoryAction::ClearAsked"),
            Self::NameEdited(_) => f.write_str("HistoryAction::NameEdited"),
            Self::Confirmed => f.write_str("HistoryAction::Confirmed"),
            Self::DialogCancelled => f.write_str("HistoryAction::DialogCancelled"),
        }
    }
}

/// The dialog that is open. Favourites are named by id, so a dialog never holds a stale copy.
#[derive(Clone, PartialEq)]
pub enum Dialog {
    Star {
        text: Box<FindText>,
        name: String,
    },
    Rename {
        id: Uuid,
        name: String,
    },
    Delete {
        id: Uuid,
    },
    /// Clear the collection's recent queries.
    Clear,
}

/// Only which dialog: the text and the name are the person's.
impl std::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Star { .. } => "Dialog::Star",
            Self::Rename { .. } => "Dialog::Rename",
            Self::Delete { .. } => "Dialog::Delete",
            Self::Clear => "Dialog::Clear",
        })
    }
}

/// The popup's state: the search text, the open dialog and why its last confirm failed.
#[derive(Clone, Default, PartialEq)]
pub struct HistoryUi {
    pub search: String,
    pub dialog: Option<Dialog>,
    /// The connection, database and collection the open dialog was opened for: what it acts on,
    /// whatever is selected when it is confirmed.
    pub target: Option<(Uuid, String, String)>,
    /// The `history.error.*` key of the last failed confirm; cleared when the dialog changes.
    pub error: Option<&'static str>,
}

impl std::fmt::Debug for HistoryUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryUi")
            .field("dialog", &self.dialog.is_some())
            .finish()
    }
}

/// The `history.error.*` key for a refused change.
pub fn error_key(error: SavedError) -> &'static str {
    match error {
        SavedError::EmptyName => "history.error.empty_name",
        SavedError::InvalidName => "history.error.invalid_name",
        SavedError::NameTooLong => "history.error.name_too_long",
        SavedError::NameTaken => "history.error.name_taken",
        SavedError::TooMany => "history.error.too_many",
        SavedError::TooLarge => "history.error.too_large",
        SavedError::FavouritesTooLarge => "history.error.favourites_too_large",
        SavedError::Unsupported | SavedError::InvalidOptions => "history.error.unusable",
    }
}

/// Why `name` cannot be confirmed in a name dialog, so the button is off and the message shows
/// as the person types. `except` is the favourite being renamed (it may keep its own name).
pub fn name_problem(
    store: &QueryHistory,
    (connection, db, collection): (Uuid, &str, &str),
    name: &str,
    except: Option<Uuid>,
) -> Option<SavedError> {
    if let Err(e) = validate_name(name) {
        return Some(e);
    }
    store
        .favourites(connection)
        .iter()
        .any(|f| {
            Some(f.id) != except
                && f.db == db
                && f.collection == collection
                && same_name(&f.name, name)
        })
        .then_some(SavedError::NameTaken)
}

/// What the popup lists for one collection and one search.
#[derive(Debug)]
pub struct Listing<'a> {
    pub favourites: Vec<&'a Favourite>,
    /// The matching recent queries with their position in the collection's whole list (the
    /// number the load and star actions carry).
    pub recent: Vec<(usize, &'a Entry)>,
    /// What the collection holds before the search, for the empty states.
    pub favourites_total: usize,
    pub recent_total: usize,
}

impl Listing<'_> {
    /// How many rows the search left, for the count that is announced.
    pub fn matches(&self) -> usize {
        self.favourites.len() + self.recent.len()
    }
}

/// The favourites and recent queries of `(connection, db, collection)` that match `search`
/// (names and query text, ignoring case; an empty search matches all).
pub fn listing<'a>(
    store: &'a QueryHistory,
    (connection, db, collection): (Uuid, &str, &str),
    search: &str,
) -> Listing<'a> {
    let needle = search.trim().to_lowercase();
    let all_favourites: Vec<&Favourite> = store
        .favourites(connection)
        .iter()
        .filter(|f| f.db == db && f.collection == collection)
        .collect();
    let all_recent = store.history(connection, db, collection);
    Listing {
        favourites_total: all_favourites.len(),
        recent_total: all_recent.len(),
        favourites: all_favourites
            .into_iter()
            .filter(|f| contains(&f.name, &needle) || text_matches(&f.text, &needle))
            .collect(),
        recent: all_recent
            .iter()
            .enumerate()
            .filter(|(_, e)| text_matches(&e.text, &needle))
            .collect(),
    }
}

fn contains(haystack: &str, needle: &str) -> bool {
    needle.is_empty() || haystack.to_lowercase().contains(needle)
}

fn text_matches(text: &FindText, needle: &str) -> bool {
    [&text.filter, &text.sort, &text.projection]
        .iter()
        .any(|field| contains(field, needle))
}

/// The one line a row shows for a query: the filter with its whitespace folded (a pretty-printed
/// filter is one line here), or `no_filter` when only other fields are set.
///
/// A row never carries more than [`ROW_CHARS`] characters (what is shown, and the name a screen
/// reader gets): a filter can be 256 KiB, and a row of that size is no use to anyone. The cut
/// is marked with an ellipsis, and only as much of the filter is read as is kept.
pub fn one_line(text: &FindText, no_filter: &str) -> String {
    let mut line = String::new();
    let mut kept = 0;
    for word in text.filter.split_whitespace() {
        if kept > 0 {
            line.push(' ');
            kept += 1;
        }
        for c in word.chars() {
            if kept >= ROW_CHARS {
                // No space before the ellipsis.
                line.truncate(line.trim_end().len());
                line.push('\u{2026}');
                return line;
            }
            line.push(c);
            kept += 1;
        }
    }
    match line.is_empty() {
        true => no_filter.to_string(),
        false => line,
    }
}

/// The most characters of a query one row shows.
pub const ROW_CHARS: usize = 200;

/// How long ago a query ran, as a phrase ("5 min ago").
pub fn ran_phrase(ran_unix: i64, now_unix: i64, i18n: &I18n) -> String {
    let age = Age::of((now_unix - ran_unix).max(0));
    match age {
        Age::Now => i18n.t(age.key()),
        _ => i18n.t_with(age.key(), &[("n", &age.count().to_string())]),
    }
}

/// The `history.matches.*` key and count for the announced result of a search.
pub fn matches_phrase(count: usize, i18n: &I18n) -> String {
    match count {
        0 => i18n.t("history.matches_none"),
        1 => i18n.t("history.matches_one"),
        n => i18n.t_with("history.matches_many", &[("n", &n.to_string())]),
    }
}

#[cfg(test)]
#[path = "query_history_tests.rs"]
mod tests;
