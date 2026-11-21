//! The History popup's own actions in the reducer (`FindAction::History`): searching, loading a
//! query into the find bar and the favourite dialogs. Split from `query_history_app`.
use super::{Op, bar_of};
use crate::app::state::AppState;
use crate::viewmodel::query_history::{Dialog, HistoryAction, HistoryUi, error_key};
use curlybrace_core::query_history::FindText;
use uuid::Uuid;

/// The active connection and the selected collection, which the popup's lists belong to.
pub(in crate::app) fn target(state: &AppState) -> Option<(Uuid, String, String)> {
    let ns = state.selected_ns()?;
    Some((state.active?, ns.db.clone(), ns.collection.clone()))
}

/// Whether the open dialog was opened for another collection than the one selected now, or for
/// one that is no longer selected at all.
pub(in crate::app) fn stale_dialog(state: &AppState) -> bool {
    let ui = &state.query_history.ui;
    ui.dialog.is_some() && ui.target != target(state)
}

/// `ui` with `dialog` open for `target`.
fn opened(ui: HistoryUi, target: (Uuid, String, String), dialog: Option<Dialog>) -> HistoryUi {
    HistoryUi {
        target: dialog.as_ref().map(|_| target),
        dialog,
        error: None,
        ..ui
    }
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
pub(in crate::app) fn on_action(
    state: AppState,
    action: HistoryAction,
) -> crate::app::reducer::Out {
    let ui = state.query_history.ui.clone();
    let target = target(&state);
    let store = &state.query_history.store;
    let next = match (action, target) {
        (HistoryAction::Search(search), _) => with_ui(state, HistoryUi { search, ..ui }),
        (HistoryAction::DialogCancelled, _) => with_ui(
            state,
            HistoryUi {
                dialog: None,
                target: None,
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
        (HistoryAction::StarAsked(i), Some(t)) => {
            let text = store
                .history(t.0, &t.1, &t.2)
                .get(i)
                .map(|e| e.text.clone());
            let dialog = text.map(|text| Dialog::Star {
                text: Box::new(text),
                name: String::new(),
            });
            with_ui(state, opened(ui, t, dialog))
        }
        (HistoryAction::RenameAsked(id), Some(t)) => {
            let dialog =
                store
                    .favourites(t.0)
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| Dialog::Rename {
                        id,
                        name: f.name.clone(),
                    });
            with_ui(state, opened(ui, t, dialog))
        }
        (HistoryAction::DeleteAsked(id), Some(t)) => {
            with_ui(state, opened(ui, t, Some(Dialog::Delete { id })))
        }
        (HistoryAction::ClearAsked, Some(t)) => with_ui(state, opened(ui, t, Some(Dialog::Clear))),
        (HistoryAction::Confirmed, current) => match (&ui.target, current) {
            (Some(opened_for), Some(now)) if *opened_for == now => confirmed(state, ui, now),
            // The collection changed (or went) since the dialog was opened: it asks nothing of
            // the new one, and is closed.
            _ => with_ui(
                state,
                HistoryUi {
                    dialog: None,
                    target: None,
                    error: None,
                    ..ui
                },
            ),
        },
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
            id: Uuid::new_v4(),
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
            target: None,
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
