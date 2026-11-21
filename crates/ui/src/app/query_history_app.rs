//! The query history inside the running app (#72): recording a find where it actually goes out,
//! the "Remember queries" switch, writing `query_history.json` from the dispatch loop, and
//! following a connection that is deleted or duplicated. The pure rules are in
//! `curlybrace_core::query_history`; the file rules are in `query_history_store`.
//!
//! The history holds the text of queries, which can contain personal values: nothing here logs a
//! query, a name or a namespace, only error kinds.
use super::persistence::{self, PersistGuard};
use super::query_history_store::{self as store, HistorySaveError, Reload};
use super::state::{AppState, Banner};
use super::{ConfigPath, CurlybraceApp, connection_ops};
use crate::viewmodel::aggregate_options::OptionsForm;
use crate::viewmodel::query_history::{Dialog, HistoryAction, HistoryUi, error_key};
use crate::viewmodel::{FindBarState, submit_find};
use curlybrace_core::pipeline_options::PipelineOptions;
use curlybrace_core::query::QueryText;
use curlybrace_core::query_history::{FindText, QueryHistory, SavedError};
use curlybrace_core::store::Query;
use std::path::PathBuf;
use uuid::Uuid;

/// One change to the store. Changes are kept until they are on disk so that, when another window
/// wrote the file first, they can be made again on top of what it wrote.
#[derive(Clone)]
pub(super) enum Op {
    Record {
        connection: Uuid,
        db: String,
        collection: String,
        text: Box<FindText>,
        now: i64,
    },
    Star {
        connection: Uuid,
        db: String,
        collection: String,
        name: String,
        text: Box<FindText>,
        now: i64,
    },
    Rename {
        connection: Uuid,
        id: Uuid,
        name: String,
    },
    Unstar {
        connection: Uuid,
        id: Uuid,
    },
    ClearNamespace {
        connection: Uuid,
        db: String,
        collection: String,
    },
    Remember(bool),
    ForgetConnection(Uuid),
    CopyConnection {
        from: Uuid,
        to: Uuid,
    },
}

/// Only the kind of change: the rest is the person's text.
impl std::fmt::Debug for Op {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Record { .. } => "Op::Record",
            Self::Star { .. } => "Op::Star",
            Self::Rename { .. } => "Op::Rename",
            Self::Unstar { .. } => "Op::Unstar",
            Self::ClearNamespace { .. } => "Op::ClearNamespace",
            Self::Remember(_) => "Op::Remember",
            Self::ForgetConnection(_) => "Op::ForgetConnection",
            Self::CopyConnection { .. } => "Op::CopyConnection",
        })
    }
}

impl Op {
    fn apply(&self, store: &QueryHistory) -> Result<QueryHistory, SavedError> {
        match self {
            Self::Record {
                connection,
                db,
                collection,
                text,
                now,
            } => store.recorded(*connection, db, collection, (**text).clone(), *now),
            Self::Star {
                connection,
                db,
                collection,
                name,
                text,
                now,
            } => store.starred(*connection, db, collection, name, (**text).clone(), *now),
            Self::Rename {
                connection,
                id,
                name,
            } => store.renamed(*connection, *id, name),
            Self::Unstar { connection, id } => Ok(store.unstarred(*connection, *id)),
            Self::ClearNamespace {
                connection,
                db,
                collection,
            } => Ok(store.cleared(*connection, db, collection)),
            Self::Remember(on) => Ok(store.with_remembering(*on)),
            Self::ForgetConnection(id) => Ok(store.without_connection(*id)),
            Self::CopyConnection { from, to } => Ok(store.with_connection_copied(*from, *to)),
        }
    }
}

/// The history, where its file is and what is still to be written.
#[derive(Debug, Default)]
pub struct QueryHistoryState {
    pub store: QueryHistory,
    /// `None` when there is no settings folder (or a test did not set one): the history then
    /// lives in memory only and nothing is ever written to the real config directory.
    pub path: Option<PathBuf>,
    /// Whether the file may be replaced (not when a bad one could not be backed up).
    pub guard: PersistGuard,
    /// What this window last read from or wrote to the file, to notice another window.
    pub seen: super::pipelines_store::DiskStamp,
    /// The History popup: its search, dialog and last error.
    pub ui: HistoryUi,
    /// Changes applied to `store` and not yet on disk, oldest first.
    pending: Vec<Op>,
    /// The backup copies of the file are to be deleted (they hold query text).
    purge_backups: bool,
    /// A failed write was announced; no second banner until a write works.
    failing: bool,
}

impl AppState {
    /// Loads the history from `path`. A file that cannot be used is backed up and the history
    /// starts empty, with the recovery banner (an earlier banner keeps its place). An unknown
    /// config folder gives an empty history that is kept in memory only.
    pub fn with_query_history_path(self, path: ConfigPath) -> Self {
        let Ok(path) = path else {
            return self;
        };
        let backup = |p: &std::path::Path, tag: &str| {
            persistence::back_up_limited(p, tag, Some(store::MAX_FILE_BYTES as u64 + 1))
        };
        let started = store::load(&path, &backup);
        let state = Self {
            query_history: Box::new(QueryHistoryState {
                store: started.settings,
                path: Some(path),
                guard: started.guard,
                seen: started.seen,
                ..QueryHistoryState::default()
            }),
            ..self
        };
        match started.recovery {
            Some(recovery) => state.with_recovery_notice(recovery),
            None => state,
        }
    }

    /// Applies `op` to the history in memory and queues it for the file (written by
    /// [`Self::query_history_flush`] at the end of the step). A refused change leaves the
    /// history as it was.
    pub(super) fn query_history_apply(&mut self, op: Op) -> Result<(), SavedError> {
        let next = op.apply(&self.query_history.store)?;
        if next != self.query_history.store {
            self.query_history.store = next;
            self.query_history.pending.push(op);
        }
        Ok(())
    }

    /// Writes the history when a step changed it (the one hook of the dispatch loop). A step that
    /// changed nothing costs a length check. The file is checked for another window's write
    /// first; when it changed, its history is taken and this window's changes are made again on
    /// top, once. A failure is logged by kind, shown once in a banner and retried with the next
    /// change.
    pub fn query_history_flush(&mut self) {
        self.query_history_backups_purged();
        if self.query_history.pending.is_empty() {
            return;
        }
        let Some(path) = self.query_history.path.clone() else {
            self.query_history.pending.clear();
            return;
        };
        match self.query_history_written(&path) {
            Ok(()) => self.query_history.failing = false,
            Err(e) => self.query_history_failed(&e),
        }
    }

    fn query_history_written(&mut self, path: &std::path::Path) -> Result<(), HistorySaveError> {
        let first = self.query_history_try(path);
        if first != Err(HistorySaveError::ChangedOnDisk) {
            return first;
        }
        self.query_history_followed(path)?;
        self.query_history_try(path)
    }

    fn query_history_try(&mut self, path: &std::path::Path) -> Result<(), HistorySaveError> {
        let qh = &mut *self.query_history;
        let written = store::saved(path, &qh.guard, qh.seen, &qh.store)?;
        qh.store = written.store;
        qh.seen = written.stamp;
        qh.pending.clear();
        Ok(())
    }

    /// Another window wrote the file: its history becomes the base and the unwritten changes of
    /// this window are made again on top. A file that cannot be used is kept as a copy first; a
    /// copy that cannot be made blocks saving, so the file is never overwritten unseen.
    fn query_history_followed(&mut self, path: &std::path::Path) -> Result<(), HistorySaveError> {
        let qh = &mut *self.query_history;
        match store::reloaded(path) {
            Reload::Usable(base, seen) => {
                qh.store = qh.pending.iter().fold(base, |store, op| {
                    op.apply(&store).unwrap_or_else(|e| {
                        log::debug!("query history change not repeated: {}", e.error_variant());
                        store
                    })
                });
                qh.seen = seen;
                Ok(())
            }
            Reload::Unusable { seen, tag } => match persistence::back_up(path, tag) {
                Ok(_) => {
                    qh.seen = seen;
                    Ok(())
                }
                Err(e) => {
                    log::warn!("could not back up the query history ({tag}): {}", e.kind());
                    qh.guard = PersistGuard::blocked();
                    Err(HistorySaveError::Blocked)
                }
            },
        }
    }

    fn query_history_failed(&mut self, error: &HistorySaveError) {
        log::warn!("could not save the query history: {error:?}");
        if !self.query_history.failing && self.banner.is_none() {
            self.query_history.failing = true;
            self.banner = Some(Banner::QueryHistoryPersist(error.clone()));
        }
    }

    /// Deletes the backup copies of the file once the switch was turned off: they hold the text
    /// it erased.
    fn query_history_backups_purged(&mut self) {
        let (true, Some(path)) = (
            self.query_history.purge_backups,
            self.query_history.path.clone(),
        ) else {
            return;
        };
        self.query_history.purge_backups = false;
        if let Err(e) = store::remove_backups(&path) {
            self.query_history_failed(&HistorySaveError::Persist(e));
        }
    }
}

impl CurlybraceApp {
    /// Loads the query history from `path` (see [`AppState::with_query_history_path`]).
    pub fn with_query_history_path(mut self, path: ConfigPath) -> Self {
        self.state = std::mem::take(&mut self.state).with_query_history_path(path);
        self
    }

    /// A delete that reached the disk takes the history and favourites of the connections it
    /// removed along.
    pub(super) fn forget_queries_of_removed(&mut self, saved: bool, before: &[Uuid]) {
        if !saved {
            return;
        }
        let removed: Vec<Uuid> = before
            .iter()
            .copied()
            .filter(|id| self.state.connection(*id).is_none())
            .collect();
        for id in removed {
            self.state
                .query_history_apply_infallible(Op::ForgetConnection(id));
        }
        self.state.query_history_flush();
    }

    /// A duplicate starts with the favourites of the connection it was copied from, never its
    /// history.
    pub(super) fn copy_queries_to_new(&mut self, source: Uuid, before: &[Uuid]) {
        for id in connection_ops::saved_ids(&self.state)
            .into_iter()
            .filter(|id| !before.contains(id))
        {
            let op = Op::CopyConnection {
                from: source,
                to: id,
            };
            self.state.query_history_apply_infallible(op);
        }
        self.state.query_history_flush();
    }
}

impl AppState {
    /// [`Self::query_history_apply`] for a change that cannot be refused.
    fn query_history_apply_infallible(&mut self, op: Op) {
        if let Err(e) = self.query_history_apply(op) {
            log::debug!("query history change refused: {}", e.error_variant());
        }
    }
}

/// The typed text of the find bar and the options it applies, for the history. A form with a
/// problem in it is not an option that applies: it is not kept.
pub(super) fn find_text_of(find: &FindBarState) -> FindText {
    let text = &find.text;
    FindText {
        filter: text.filter.clone(),
        sort: text.sort.clone(),
        projection: text.projection.clone(),
        skip: text.skip.clone(),
        limit: text.limit.clone(),
        options: find.options.parsed().ok().cloned().unwrap_or_default(),
    }
}

/// The find bar a stored query makes: its text and options, no error.
pub(super) fn bar_of(text: &FindText) -> FindBarState {
    let options: &PipelineOptions = &text.options;
    FindBarState {
        options: OptionsForm::from_options(options),
        text: QueryText {
            filter: text.filter.clone(),
            sort: text.sort.clone(),
            projection: text.projection.clone(),
            skip: text.skip.clone(),
            limit: text.limit.clone(),
        },
        ..FindBarState::default()
    }
}

/// Records the find that was just sent (`query`) under what the find bar says, for the active
/// connection and the selected collection. Called only where a find actually goes out. When the
/// bar no longer parses to `query` (it was edited while a guard check held the find back) the
/// text is not what was sent, so nothing is recorded.
pub(super) fn recorded(mut state: AppState, query: &Query) -> AppState {
    if !state.query_history.store.remembering() {
        return state;
    }
    let (Some(connection), Some(ns)) = (state.active, state.selected_ns().cloned()) else {
        return state;
    };
    if submit_find(&state.find).ok().as_ref() != Some(query) {
        return state;
    }
    let op = Op::Record {
        connection,
        db: ns.db,
        collection: ns.collection,
        text: Box::new(find_text_of(&state.find)),
        now: curlybrace_core::clock::now_secs(),
    };
    if let Err(e) = state.query_history_apply(op) {
        log::debug!("query not recorded: {}", e.error_variant());
    }
    state
}

/// `ToolsAction::SetRememberQueries`: the switch set. Off erases the history at once, in memory
/// and in the file, and deletes the backup copies of the file; favourites stay. When saving is
/// off (a newer file, a failed backup) the switch applies for this run and the banner says the
/// file is not updated.
pub(super) fn remember_set(mut state: AppState, on: bool) -> AppState {
    state.query_history_apply_infallible(Op::Remember(on));
    if !on {
        state.query_history.purge_backups = true;
    }
    state
}

/// The active connection and the selected collection, which the popup's lists belong to.
pub(super) fn target(state: &AppState) -> Option<(Uuid, String, String)> {
    let ns = state.selected_ns()?;
    Some((state.active?, ns.db.clone(), ns.collection.clone()))
}

fn with_ui(mut state: AppState, ui: HistoryUi) -> AppState {
    state.query_history.ui = ui;
    state
}

/// The find bar of `state` replaced by what a stored query holds. Nothing is sent: the person
/// presses Run, and running an edited bar records a new entry, never a change to the stored one.
fn loaded(state: AppState, text: Option<FindText>) -> AppState {
    match text {
        Some(text) => {
            let state = AppState {
                find: bar_of(&text),
                ..state
            };
            with_ui(state, HistoryUi::default())
        }
        None => state,
    }
}

/// The History popup's own actions (`FindAction::History`): searching, loading a query into the
/// find bar, and the favourite dialogs. Sends no command.
pub(super) fn on_action(state: AppState, action: HistoryAction) -> super::reducer::Out {
    let ui = state.query_history.ui.clone();
    let target = target(&state);
    let store = &state.query_history.store;
    let next = match (action, target) {
        (HistoryAction::Search(search), _) => with_ui(state, HistoryUi { search, ..ui }),
        (HistoryAction::DialogCancelled, _) => with_ui(
            state,
            HistoryUi {
                dialog: None,
                error: None,
                ..ui
            },
        ),
        (HistoryAction::NameEdited(name), _) => {
            let dialog = match ui.dialog.clone() {
                Some(Dialog::Star { text, .. }) => Some(Dialog::Star { text, name }),
                Some(Dialog::Rename { id, .. }) => Some(Dialog::Rename { id, name }),
                other => other,
            };
            with_ui(
                state,
                HistoryUi {
                    dialog,
                    error: None,
                    ..ui
                },
            )
        }
        (HistoryAction::LoadRecent(i), Some((c, db, coll))) => {
            let text = store.history(c, &db, &coll).get(i).map(|e| e.text.clone());
            loaded(state, text)
        }
        (HistoryAction::LoadFavourite(id), Some((c, ..))) => {
            let text = store
                .favourites(c)
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.text.clone());
            loaded(state, text)
        }
        (HistoryAction::StarAsked(i), Some((c, db, coll))) => {
            let text = store.history(c, &db, &coll).get(i).map(|e| e.text.clone());
            let dialog = text.map(|text| Dialog::Star {
                text: Box::new(text),
                name: String::new(),
            });
            with_ui(
                state,
                HistoryUi {
                    dialog,
                    error: None,
                    ..ui
                },
            )
        }
        (HistoryAction::RenameAsked(id), Some((c, ..))) => {
            let dialog = store
                .favourites(c)
                .iter()
                .find(|f| f.id == id)
                .map(|f| Dialog::Rename {
                    id,
                    name: f.name.clone(),
                });
            with_ui(
                state,
                HistoryUi {
                    dialog,
                    error: None,
                    ..ui
                },
            )
        }
        (HistoryAction::DeleteAsked(id), Some(_)) => with_ui(
            state,
            HistoryUi {
                dialog: Some(Dialog::Delete { id }),
                error: None,
                ..ui
            },
        ),
        (HistoryAction::ClearAsked, Some(_)) => with_ui(
            state,
            HistoryUi {
                dialog: Some(Dialog::Clear),
                error: None,
                ..ui
            },
        ),
        (HistoryAction::Confirmed, Some(target)) => confirmed(state, ui, target),
        // A request with no connection or collection behind it (a late click): nothing.
        _ => state,
    };
    (next, Vec::new())
}

/// The open dialog confirmed: the change is made, and the dialog closes when it was accepted. A
/// refusal (a taken name, a full list) stays in the dialog as a message.
fn confirmed(
    mut state: AppState,
    ui: HistoryUi,
    (connection, db, collection): (Uuid, String, String),
) -> AppState {
    let Some(dialog) = ui.dialog.clone() else {
        return state;
    };
    let op = match dialog {
        Dialog::Star { text, name } => Op::Star {
            connection,
            db,
            collection,
            name,
            text,
            now: curlybrace_core::clock::now_secs(),
        },
        Dialog::Rename { id, name } => Op::Rename {
            connection,
            id,
            name,
        },
        Dialog::Delete { id } => Op::Unstar { connection, id },
        Dialog::Clear => Op::ClearNamespace {
            connection,
            db,
            collection,
        },
    };
    let error = state.query_history_apply(op).err().map(error_key);
    let ui = match error {
        None => HistoryUi {
            dialog: None,
            error: None,
            ..ui
        },
        Some(key) => HistoryUi {
            error: Some(key),
            ..ui
        },
    };
    with_ui(state, ui)
}

#[cfg(test)]
#[path = "query_history_app_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_history_actions_tests.rs"]
mod action_tests;

#[cfg(test)]
#[path = "query_history_log_tests.rs"]
mod log_tests;
