//! Manager operations beyond save/delete: duplicate, folder changes and the connect stamp.
//! Like `connections::save`, they take the keychain and the config path as arguments.
use super::connections::{ConfigPath, deleted, persist, refresh_flags, refused};
use super::dialog_banner::DialogBannerWatch;
#[cfg(test)]
use super::persistence::PersistGuard;
use super::state::{AppState, Banner};
use crate::viewmodel::ConnectionForm;
use crate::views::connections::{ConnectionsState, EditMode};
use curlybrace_core::connection::{ConnectionConfig, unique_name};
use curlybrace_core::connection_folders::{
    deleted_folder, moved_to_folder, normalize_folder, renamed_folder,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Command;
use curlybrace_core::secrets::{SecretKind, SecretStore, copy_all, secret_key};
use uuid::Uuid;

fn with_list(state: AppState, list: Vec<ConnectionConfig>, path: &ConfigPath) -> AppState {
    with_list_saved(state, list, path).0
}

/// [`with_list`], plus whether the list was written to disk.
fn with_list_saved(
    state: AppState,
    list: Vec<ConnectionConfig>,
    path: &ConfigPath,
) -> (AppState, bool) {
    let write = persist(path, &list);
    let saved = write.is_ok();
    let banner = write.err().map(Banner::Persist);
    let next = AppState {
        conn: ConnectionsState {
            connections: list,
            error: None,
            ..state.conn
        },
        banner: banner.or(state.banner),
        ..state
    };
    (next, saved)
}

/// A folder change went through: the folder prompt, kept open while it was rejected, closes.
fn prompt_closed(state: AppState) -> AppState {
    AppState {
        conn: ConnectionsState {
            folder_prompt: None,
            ..state.conn
        },
        ..state
    }
}

/// Runs `edit` on an owned copy of the connections view state (the closing methods take
/// `&mut self`; the handlers stay value-in, value-out).
pub(super) fn with_conn(state: AppState, edit: impl FnOnce(&mut ConnectionsState)) -> AppState {
    let mut conn = state.conn;
    edit(&mut conn);
    AppState { conn, ..state }
}

/// Closes the edit dialog if it is open (connect, defensive close): tools reset, typed secrets
/// dropped. Does nothing otherwise.
pub(super) fn dialog_closed(state: AppState) -> AppState {
    if state.conn.editing {
        with_conn(state, ConnectionsState::close_edit)
    } else {
        state
    }
}

/// The ids of the saved connections, for `purge_removed` to compare against later.
pub(super) fn saved_ids(state: &AppState) -> Vec<Uuid> {
    state.conn.connections.iter().map(|c| c.id).collect()
}

/// One `PurgeHistory` for every id in `before` that is no longer a saved connection, each once:
/// the usage history of a connection that is gone has no one left to read it. Only deletes call
/// this; closing a session, importing and saving never remove an id.
pub(super) fn purge_removed(before: &[Uuid], state: &AppState) -> Vec<Command> {
    let mut seen = std::collections::BTreeSet::new();
    before
        .iter()
        .filter(|id| state.connection(**id).is_none() && seen.insert(**id))
        .map(|id| Command::PurgeHistory { connection: *id })
        .collect()
}

/// Deletes the connection; deleting the one the dialog is editing closes the dialog.
#[cfg(test)]
pub(super) fn delete_and_close(
    state: AppState,
    id: Uuid,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
) -> AppState {
    delete_and_close_saved(state, id, secrets, path).0
}

/// [`delete_and_close`], plus whether the delete was written to disk (what a history purge may
/// rely on).
pub(super) fn delete_and_close_saved(
    state: AppState,
    id: Uuid,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
) -> (AppState, bool) {
    let was_edited = state.conn.editing && state.conn.form.id == id;
    let (next, saved) = deleted(state, id, secrets, path);
    let gone = !next.conn.connections.iter().any(|c| c.id == id);
    match was_edited && gone {
        true => (with_conn(next, ConnectionsState::close_edit), saved),
        false => (next, saved),
    }
}

fn with_error(state: AppState, error: CoreError) -> AppState {
    AppState {
        conn: ConnectionsState {
            error: Some(error),
            ..state.conn
        },
        ..state
    }
}

/// Adds a copy of `id` named `copy_name` (made unique), with its keychain secrets. All or
/// nothing: a failed secret copy or a failed save leaves neither a config nor secrets behind.
pub(super) fn duplicate(
    state: AppState,
    id: Uuid,
    copy_name: String,
    now: i64,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let Some(source) = state.conn.connections.iter().find(|c| c.id == id) else {
        return state;
    };
    let name = unique_name(&copy_name, &state.conn.connections);
    let copy = source.duplicate(name, now);
    if let Err(error) = copy_all(secrets, id, copy.id) {
        return AppState {
            banner: Some(Banner::Failure {
                context: "duplicate_secrets",
                error,
            }),
            ..state
        };
    }
    let mut list = state.conn.connections.clone();
    list.push(copy.clone());
    if let Err(e) = persist(path, &list) {
        for kind in SecretKind::ALL {
            let _ = secrets.delete(&secret_key(copy.id, kind));
        }
        return AppState {
            banner: Some(Banner::Persist(e)),
            ..state
        };
    }
    // The copy opens in the edit dialog, with the tools of the previous form dropped.
    let mut conn = ConnectionsState {
        connections: list,
        selected: Some(copy.id),
        form: ConnectionForm::from_config(&copy),
        error: None,
        notice: None,
        editing: true,
        edit_mode: EditMode::Duplicate,
        ..state.conn
    };
    conn.reset_tools();
    open_watching_banner(AppState { conn, ..state }, secrets)
}

/// Refreshes the flags of a state whose dialog was just opened, handing the banner watch over
/// open: a banner showing BEFORE this step stays out of the dialog, one this step raised (a
/// failed flag read, even an identical one) is new. The refresh runs against an empty banner
/// slot to tell the two apart, and the earlier banner is put back if none was raised.
fn open_watching_banner(state: AppState, secrets: &dyn SecretStore) -> AppState {
    let prior = state.banner.clone();
    let next = refresh_flags(
        AppState {
            banner: None,
            ..state
        },
        secrets,
    );
    let baseline = match next.banner {
        None => prior.as_ref(),
        Some(_) => None,
    };
    let watch = DialogBannerWatch::default().observed(true, baseline);
    AppState {
        banner: next.banner.or(prior),
        dialog_banner_watch: watch,
        ..next
    }
}

pub(super) fn move_to_folder(
    state: AppState,
    id: Uuid,
    folder: String,
    now: i64,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        // The prompt closes too, so OK never looks dead; the banner explains.
        return prompt_closed(refused(state, e));
    }
    match normalize_folder(&folder) {
        Ok(folder) => {
            let list = moved_to_folder(&state.conn.connections, id, folder, now);
            prompt_closed(with_list(state, list, path))
        }
        Err(e) => with_error(state, e),
    }
}

pub(super) fn rename_folder(
    state: AppState,
    from: String,
    to: String,
    now: i64,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return prompt_closed(refused(state, e));
    }
    match renamed_folder(&state.conn.connections, &from, &to, now) {
        Ok(list) => prompt_closed(with_list(state, list, path)),
        Err(e) => with_error(state, e),
    }
}

#[cfg(test)]
pub(super) fn delete_folder(
    state: AppState,
    name: String,
    now: i64,
    path: &ConfigPath,
) -> AppState {
    delete_folder_saved(state, name, now, path).0
}

/// [`delete_folder`], plus whether the new list was written to disk.
pub(super) fn delete_folder_saved(
    state: AppState,
    name: String,
    now: i64,
    path: &ConfigPath,
) -> (AppState, bool) {
    if let Err(e) = state.persist.check() {
        return (refused(state, e), false);
    }
    let list = deleted_folder(&state.conn.connections, &name, now);
    with_list_saved(state, list, path)
}

/// Stamps `last_connected` after a successful connect. A blocked or failing save shows the
/// persistence banner; it never pretends the stamp was saved. The edit form, if it shows this
/// connection, carries the new stamp so a later save does not overwrite it with a stale value.
pub(super) fn mark_connected(state: AppState, id: Uuid, now: i64, path: &ConfigPath) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let list: Vec<ConnectionConfig> = state
        .conn
        .connections
        .iter()
        .map(|c| {
            if c.id == id {
                c.clone().with_connected_at(now)
            } else {
                c.clone()
            }
        })
        .collect();
    let stamped = list
        .iter()
        .find(|c| c.id == id)
        .and_then(|c| c.last_connected);
    let form = if state.conn.form.id == id {
        crate::viewmodel::ConnectionForm {
            last_connected: stamped.or(state.conn.form.last_connected),
            ..state.conn.form.clone()
        }
    } else {
        state.conn.form.clone()
    };
    let state = AppState {
        conn: ConnectionsState { form, ..state.conn },
        ..state
    };
    with_list(state, list, path)
}

#[cfg(test)]
#[path = "connection_ops_tests.rs"]
mod tests;
