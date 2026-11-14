//! Pure handlers for saving, opening, renaming and deleting saved pipelines from the Aggregate
//! tab. A pipeline belongs to the active session's connection. A change reaches the list only
//! after `pipelines.json` was written; a name problem or a failed write is shown inline in the
//! open dialog (never as a banner) and leaves the dialog, the draft and the list as they were.
//! Opening a pipeline sends nothing: the tab's own auto refresh previews it. Nothing here logs
//! a name or a stage.
use super::aggregate_actions::{AggregateAction, with_draft};
use super::reducer::Out;
use super::state::AppState;
use crate::viewmodel::AggregateState;
use crate::viewmodel::aggregate_saved::SavedDialog;
use crate::viewmodel::aggregate_view::AggregateView;
use curlybrace_core::saved_pipeline::{SavedError, SavedPipeline, SavedPipelines};
use uuid::Uuid;

/// Applies a saved-pipeline `action` to the active session; without a session nothing happens.
pub(super) fn on_action(state: AppState, action: AggregateAction) -> Out {
    let Some(conn) = state.active else {
        return (state, Vec::new());
    };
    let draft = (*state.aggregate).clone();
    let dialog = draft.saved.dialog.clone();
    let next = match (action, dialog) {
        (AggregateAction::SaveClicked, _) => save_clicked(state, draft),
        (AggregateAction::SaveNameEdited(name), Some(SavedDialog::Save { .. })) => shown(
            state,
            draft,
            SavedDialog::Save {
                name,
                replace_prompt: false,
            },
        ),
        (
            AggregateAction::SaveConfirmed,
            Some(SavedDialog::Save {
                name,
                replace_prompt,
            }),
        ) => save_confirmed(state, conn, draft, &name, replace_prompt),
        (AggregateAction::OpenMenuPicked(id), _) => open_picked(state, conn, draft, id),
        (AggregateAction::OpenConfirmed, Some(SavedDialog::ConfirmOpen { id })) => {
            match state.pipelines.find(conn, id).cloned() {
                Some(entry) => opened(state, draft, &entry),
                None => closed(state, draft),
            }
        }
        (AggregateAction::RenameStarted(id), _) => rename_started(state, conn, draft, id),
        (AggregateAction::RenameEdited(name), Some(SavedDialog::Rename { id, .. })) => {
            shown(state, draft, SavedDialog::Rename { id, name })
        }
        (AggregateAction::RenameConfirmed, Some(SavedDialog::Rename { id, name })) => {
            rename_confirmed(state, conn, draft, id, &name)
        }
        (AggregateAction::DeleteAsked(id), _) => delete_asked(state, conn, draft, id),
        (AggregateAction::DeleteConfirmed, Some(SavedDialog::ConfirmDelete { id })) => {
            delete_confirmed(state, conn, draft, id)
        }
        (AggregateAction::DialogCancelled, _) => closed(state, draft),
        // A confirm without its dialog (a late click) or an action of the draft: nothing.
        _ => state,
    };
    (next, Vec::new())
}

/// `state` showing `dialog`, with no error from an earlier attempt.
fn shown(state: AppState, draft: AggregateState, dialog: SavedDialog) -> AppState {
    with_draft(state, draft.with_saved_dialog(Some(dialog)))
}

fn closed(state: AppState, draft: AggregateState) -> AppState {
    with_draft(state, draft.with_saved_dialog(None))
}

/// `state` keeping the dialog open, saying why in `key`.
fn failed(state: AppState, draft: AggregateState, key: &'static str) -> AppState {
    with_draft(state, draft.with_saved_error(key))
}

/// The message key of a refused change to the list.
fn error_key(error: SavedError) -> &'static str {
    match error {
        SavedError::EmptyName => "aggregate.saved.error.empty_name",
        SavedError::NameTooLong => "aggregate.saved.error.name_too_long",
        SavedError::NameTaken => "aggregate.saved.error.name_taken",
        SavedError::TooMany => "aggregate.saved.error.too_many",
        SavedError::TooLarge => "aggregate.saved.error.too_large",
        SavedError::Unsupported => "aggregate.saved.error.unsupported",
    }
}

/// Writes `next`: the state holds it as the list when that worked; a failure leaves the list as
/// it was (`persist_pipelines` changes it only after the write) and gives the message key.
fn persisted(mut state: AppState, next: SavedPipelines) -> (AppState, Option<&'static str>) {
    let error = state.persist_pipelines(next).err().map(|e| {
        log::warn!("could not save the saved pipelines: {e:?}");
        e.i18n_key()
    });
    (state, error)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// The `db.collection` the pipeline is saved from, as a hint for the list.
fn ns_hint(state: &AppState) -> Option<String> {
    state
        .selected_ns()
        .map(|ns| format!("{}.{}", ns.db, ns.collection))
}

/// The Save dialog, starting from the opened pipeline's name so a re-save is one confirmation
/// (and a second one, to replace).
fn save_clicked(state: AppState, draft: AggregateState) -> AppState {
    let name = draft
        .saved
        .opened
        .as_ref()
        .map(|(_, name)| name.clone())
        .unwrap_or_default();
    shown(
        state,
        draft,
        SavedDialog::Save {
            name,
            replace_prompt: false,
        },
    )
}

fn save_confirmed(
    state: AppState,
    conn: Uuid,
    draft: AggregateState,
    name: &str,
    replace: bool,
) -> AppState {
    if draft.text_is_invalid() {
        return failed(state, draft, "aggregate.saved.error.invalid_text");
    }
    let result = state.pipelines.saved(
        conn,
        name,
        ns_hint(&state),
        draft.as_saved_stages(),
        now_unix(),
        replace,
    );
    let next = match result {
        Ok(next) => next,
        Err(SavedError::NameTaken) => {
            let name = name.to_string();
            return shown(
                state,
                draft,
                SavedDialog::Save {
                    name,
                    replace_prompt: true,
                },
            );
        }
        Err(e) => return failed(state, draft, error_key(e)),
    };
    let Some(entry) = next.find_by_name(conn, name).cloned() else {
        return failed(state, draft, "aggregate.saved.error.empty_name");
    };
    match persisted(state, next) {
        (state, None) => with_draft(state, draft.saved_as(entry.id, &entry.name)),
        (state, Some(key)) => failed(state, draft, key),
    }
}

/// Opens the picked pipeline, or asks first when the draft has changes that would be lost.
fn open_picked(state: AppState, conn: Uuid, draft: AggregateState, id: Uuid) -> AppState {
    let Some(entry) = state.pipelines.find(conn, id).cloned() else {
        return state;
    };
    match draft.is_modified() {
        true => shown(state, draft, SavedDialog::ConfirmOpen { id }),
        false => opened(state, draft, &entry),
    }
}

/// The draft replaced by `entry`; typing the view still held for the old draft is dropped (it
/// would land in a stage of the new one), the view's chosen mode stays.
fn opened(state: AppState, draft: AggregateState, entry: &SavedPipeline) -> AppState {
    let view = AggregateView {
        mode: state.aggregate_view.mode,
        ..AggregateView::default()
    };
    AppState {
        aggregate_view: Box::new(view),
        ..with_draft(state, draft.opened_from(entry))
    }
}

fn rename_started(state: AppState, conn: Uuid, draft: AggregateState, id: Uuid) -> AppState {
    let Some(name) = state.pipelines.find(conn, id).map(|p| p.name.clone()) else {
        return state;
    };
    shown(state, draft, SavedDialog::Rename { id, name })
}

fn rename_confirmed(
    state: AppState,
    conn: Uuid,
    draft: AggregateState,
    id: Uuid,
    name: &str,
) -> AppState {
    let next = match state.pipelines.renamed(conn, id, name) {
        Ok(next) => next,
        Err(e) => return failed(state, draft, error_key(e)),
    };
    let shown_name = next.find(conn, id).map(|p| p.name.clone());
    match persisted(state, next) {
        (state, None) => {
            let renamed_opened = draft.saved.opened.as_ref().is_some_and(|(o, _)| *o == id);
            let draft = match (renamed_opened, shown_name) {
                (true, Some(n)) => draft.with_opened_name(&n),
                _ => draft,
            };
            closed(state, draft)
        }
        (state, Some(key)) => failed(state, draft, key),
    }
}

fn delete_asked(state: AppState, conn: Uuid, draft: AggregateState, id: Uuid) -> AppState {
    match state.pipelines.find(conn, id) {
        Some(_) => shown(state, draft, SavedDialog::ConfirmDelete { id }),
        None => state,
    }
}

/// Deletes the pipeline; when it was the opened one the draft stays as an unsaved draft.
fn delete_confirmed(state: AppState, conn: Uuid, draft: AggregateState, id: Uuid) -> AppState {
    let next = state.pipelines.deleted(conn, id);
    match persisted(state, next) {
        (state, None) => {
            let was_opened = draft.saved.opened.as_ref().is_some_and(|(o, _)| *o == id);
            let draft = if was_opened {
                draft.without_opened()
            } else {
                draft
            };
            closed(state, draft)
        }
        (state, Some(key)) => failed(state, draft, key),
    }
}

#[cfg(test)]
#[path = "aggregate_saved_actions_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "aggregate_saved_manage_tests.rs"]
mod manage_tests;
