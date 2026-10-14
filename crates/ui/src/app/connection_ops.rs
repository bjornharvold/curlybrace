//! Manager operations beyond save/delete: duplicate, folder changes and the connect stamp.
//! Like `connections::save`, they take the keychain and the config path as arguments.
use super::connections::{ConfigPath, persist, refused};
#[cfg(test)]
use super::persistence::PersistGuard;
use super::state::{AppState, Banner};
use crate::views::connections::ConnectionsState;
use curlybrace_core::connection::{ConnectionConfig, unique_name};
use curlybrace_core::connection_folders::{
    deleted_folder, moved_to_folder, normalize_folder, renamed_folder,
};
use curlybrace_core::error::CoreError;
use curlybrace_core::secrets::{SecretKind, SecretStore, copy_all, secret_key};
use uuid::Uuid;

fn with_list(state: AppState, list: Vec<ConnectionConfig>, path: &ConfigPath) -> AppState {
    let banner = persist(path, &list).err().map(Banner::Persist);
    AppState {
        conn: ConnectionsState {
            connections: list,
            error: None,
            ..state.conn
        },
        banner: banner.or(state.banner),
        ..state
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
    AppState {
        conn: ConnectionsState {
            connections: list,
            selected: Some(copy.id),
            error: None,
            ..state.conn
        },
        ..state
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
        return refused(state, e);
    }
    match normalize_folder(&folder) {
        Ok(folder) => {
            let list = moved_to_folder(&state.conn.connections, id, folder, now);
            with_list(state, list, path)
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
        return refused(state, e);
    }
    match renamed_folder(&state.conn.connections, &from, &to, now) {
        Ok(list) => with_list(state, list, path),
        Err(e) => with_error(state, e),
    }
}

pub(super) fn delete_folder(
    state: AppState,
    name: String,
    now: i64,
    path: &ConfigPath,
) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let list = deleted_folder(&state.conn.connections, &name, now);
    with_list(state, list, path)
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
mod tests {
    use super::*;
    use curlybrace_core::secrets::{MemorySecretStore, SecretKind, secret_key};

    fn setup() -> (tempfile::TempDir, ConfigPath, MemorySecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let path: ConfigPath = Ok(dir.path().join("connections.json"));
        (dir, path, MemorySecretStore::new())
    }

    fn state_with(list: Vec<ConnectionConfig>) -> AppState {
        AppState::new(list)
    }

    #[test]
    fn duplicate_copies_config_and_secrets_and_persists() {
        let (_dir, path, secrets) = setup();
        let src = ConnectionConfig::new("prod", "mongodb://h").with_save_stamp(1);
        secrets
            .set(&secret_key(src.id, SecretKind::Password), "pw")
            .unwrap();
        let state = duplicate(
            state_with(vec![src.clone()]),
            src.id,
            "prod copy".into(),
            50,
            &secrets,
            &path,
        );
        assert_eq!(state.conn.connections.len(), 2);
        let copy = &state.conn.connections[1];
        assert_eq!(copy.name, "prod copy");
        assert_ne!(copy.id, src.id);
        assert_eq!(
            secrets
                .get(&secret_key(copy.id, SecretKind::Password))
                .unwrap()
                .as_deref(),
            Some("pw")
        );
        let on_disk = std::fs::read_to_string(path.as_ref().unwrap()).unwrap();
        assert!(on_disk.contains("prod copy"));
        assert!(!on_disk.contains("pw\""), "no secret in the file");
    }

    struct FailingSet(MemorySecretStore);
    impl SecretStore for FailingSet {
        fn get(&self, k: &str) -> curlybrace_core::error::Result<Option<String>> {
            self.0.get(k)
        }
        fn set(&self, _: &str, _: &str) -> curlybrace_core::error::Result<()> {
            Err(CoreError::Secret("boom".into()))
        }
        fn delete(&self, k: &str) -> curlybrace_core::error::Result<()> {
            self.0.delete(k)
        }
    }

    #[test]
    fn duplicate_that_fails_to_copy_secrets_adds_nothing() {
        let (_dir, path, memory) = setup();
        let src = ConnectionConfig::new("prod", "mongodb://h");
        memory
            .set(&secret_key(src.id, SecretKind::Password), "pw")
            .unwrap();
        let secrets = FailingSet(memory);
        let state = duplicate(
            state_with(vec![src.clone()]),
            src.id,
            "copy".into(),
            50,
            &secrets,
            &path,
        );
        assert_eq!(state.conn.connections.len(), 1);
        assert!(
            state.banner.is_some(),
            "the failure is surfaced, not swallowed"
        );
        assert!(!path.as_ref().unwrap().exists(), "nothing was written");
    }

    #[test]
    fn duplicate_whose_persist_fails_removes_the_copied_secrets() {
        let (dir, _path, secrets) = setup();
        // A path whose parent is a file cannot be written.
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let path: ConfigPath = Ok(blocker.join("connections.json"));
        let src = ConnectionConfig::new("prod", "mongodb://h");
        secrets
            .set(&secret_key(src.id, SecretKind::Password), "pw")
            .unwrap();
        let state = duplicate(
            state_with(vec![src.clone()]),
            src.id,
            "copy".into(),
            50,
            &secrets,
            &path,
        );
        assert_eq!(state.conn.connections.len(), 1);
        assert!(state.banner.is_some());
        // Only the source's secret remains.
        assert!(
            secrets
                .get(&secret_key(src.id, SecretKind::Password))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn unknown_id_is_a_no_op() {
        let (_dir, path, secrets) = setup();
        let state = duplicate(
            state_with(vec![]),
            Uuid::new_v4(),
            "x".into(),
            1,
            &secrets,
            &path,
        );
        assert!(state.conn.connections.is_empty());
    }

    #[test]
    fn move_rename_and_delete_folder_persist_the_new_list() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let state = move_to_folder(state_with(vec![a.clone()]), a.id, "work".into(), 5, &path);
        assert_eq!(state.conn.connections[0].folder.as_deref(), Some("work"));
        let state = rename_folder(state, "work".into(), "play".into(), 6, &path);
        assert_eq!(state.conn.connections[0].folder.as_deref(), Some("play"));
        let state = delete_folder(state, "play".into(), 7, &path);
        assert_eq!(
            state.conn.connections.len(),
            1,
            "deleting a folder keeps its connections"
        );
        assert_eq!(state.conn.connections[0].folder, None);
        assert!(
            std::fs::read_to_string(path.as_ref().unwrap())
                .unwrap()
                .contains("\"a\"")
        );
    }

    #[test]
    fn invalid_folder_name_sets_an_error_and_changes_nothing() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let state = move_to_folder(
            state_with(vec![a.clone()]),
            a.id,
            "bad\nname".into(),
            5,
            &path,
        );
        assert!(state.conn.error.is_some());
        assert_eq!(state.conn.connections[0].folder, None);
    }

    #[test]
    fn mark_connected_stamps_and_persists() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let state = mark_connected(state_with(vec![a.clone()]), a.id, 42, &path);
        assert_eq!(state.conn.connections[0].last_connected, Some(42));
        assert!(
            std::fs::read_to_string(path.as_ref().unwrap())
                .unwrap()
                .contains("42")
        );
    }

    #[test]
    fn mark_connected_with_a_blocked_guard_shows_a_banner_and_keeps_the_list() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let mut state = state_with(vec![a.clone()]);
        state.persist = PersistGuard::blocked();
        let state = mark_connected(state, a.id, 42, &path);
        assert!(state.banner.is_some());
        assert_eq!(state.conn.connections[0].last_connected, None);
    }

    #[test]
    fn mark_connected_refreshes_the_open_form_so_a_later_save_keeps_the_stamp() {
        use crate::app::connections::save;
        use crate::viewmodel::ConnectionForm;
        let (_dir, path, secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let mut state = state_with(vec![a.clone()]);
        state.conn.selected = Some(a.id);
        state.conn.form = ConnectionForm::from_config(&a);
        assert_eq!(state.conn.form.last_connected, None);
        let state = mark_connected(state, a.id, 42, &path);
        assert_eq!(state.conn.form.last_connected, Some(42));
        let form = state.conn.form.clone();
        let state = save(state, &form, &secrets, &path, 99);
        assert_eq!(state.conn.connections[0].last_connected, Some(42));
    }
}
