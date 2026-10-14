//! Manager operations beyond save/delete: duplicate, folder changes and the connect stamp.
//! Like `connections::save`, they take the keychain and the config path as arguments.
use super::connections::{ConfigPath, delete, persist, refresh_flags, refused, save};
#[cfg(test)]
use super::persistence::PersistGuard;
use super::state::{AppState, Banner};
use crate::viewmodel::ConnectionForm;
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
fn with_conn(state: AppState, edit: impl FnOnce(&mut ConnectionsState)) -> AppState {
    let mut conn = state.conn;
    edit(&mut conn);
    AppState { conn, ..state }
}

/// Saves the form and closes the edit dialog only when the save went through. A rejected form,
/// a blocked or failing write or a keychain failure leave the dialog open (the error shows in
/// it; a banner shows behind it) with the typed values intact.
pub(super) fn save_and_close(
    state: AppState,
    form: &ConnectionForm,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
    now: i64,
) -> AppState {
    // Save against an empty banner slot so a banner that appears is always from this save, even
    // when it equals one that was already showing.
    let prior = state.banner.clone();
    let probe = AppState {
        banner: None,
        ..state
    };
    let next = save(probe, form, secrets, path, now);
    let succeeded = next.conn.error.is_none() && next.banner.is_none();
    let next = AppState {
        banner: next.banner.or(prior),
        ..next
    };
    if succeeded {
        with_conn(next, ConnectionsState::saved)
    } else {
        next
    }
}

/// Deletes the connection; deleting the one the dialog is editing closes the dialog.
pub(super) fn delete_and_close(
    state: AppState,
    id: Uuid,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
) -> AppState {
    let was_edited = state.conn.editing && state.conn.form.id == id;
    let next = delete(state, id, secrets, path);
    let gone = !next.conn.connections.iter().any(|c| c.id == id);
    if was_edited && gone {
        with_conn(next, ConnectionsState::close_edit)
    } else {
        next
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
        ..state.conn
    };
    conn.reset_tools();
    refresh_flags(AppState { conn, ..state }, secrets)
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
    use crate::views::connections::FolderPrompt;
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

    #[test]
    fn duplicate_loads_the_copy_into_the_form_and_refreshes_its_flags() {
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
        let copy = state.conn.connections[1].clone();
        assert_eq!(state.conn.selected, Some(copy.id));
        assert_eq!(state.conn.form.id, copy.id);
        assert_eq!(state.conn.form.name, "prod copy");
        assert!(state.conn.form.password.is_empty(), "no secret in the form");
        assert!(state.conn.stored[&copy.id].password, "flags refreshed");
    }

    #[test]
    fn duplicate_of_a_max_length_name_still_validates() {
        let (_dir, path, secrets) = setup();
        let long = "a".repeat(curlybrace_core::connection::MAX_NAME_LEN);
        let src = ConnectionConfig::new(&long, "mongodb://h").with_save_stamp(1);
        let state = duplicate(
            state_with(vec![src.clone()]),
            src.id,
            long,
            50,
            &secrets,
            &path,
        );
        assert_eq!(state.conn.connections.len(), 2);
        assert!(state.conn.connections[1].validate().is_ok());
    }

    /// Wraps a memory store, can fail `set` for one kind of secret, and remembers every key that
    /// was written successfully so a test can prove none of them survived a rollback.
    struct Recording {
        inner: MemorySecretStore,
        fail_suffix: Option<&'static str>,
        written: std::sync::Mutex<Vec<String>>,
    }

    impl Recording {
        fn new(fail_suffix: Option<&'static str>) -> Self {
            Self {
                inner: MemorySecretStore::new(),
                fail_suffix,
                written: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn written_keys(&self) -> Vec<String> {
            self.written.lock().unwrap().clone()
        }
    }

    impl SecretStore for Recording {
        fn get(&self, k: &str) -> curlybrace_core::error::Result<Option<String>> {
            self.inner.get(k)
        }
        fn set(&self, k: &str, v: &str) -> curlybrace_core::error::Result<()> {
            if self.fail_suffix.is_some_and(|s| k.ends_with(s)) {
                return Err(CoreError::Secret("boom".into()));
            }
            self.written.lock().unwrap().push(k.to_owned());
            self.inner.set(k, v)
        }
        fn delete(&self, k: &str) -> curlybrace_core::error::Result<()> {
            self.inner.delete(k)
        }
    }

    /// Every key ever written through `store` after seeding, other than the source's own.
    fn assert_no_copy_secrets_remain(store: &Recording, src: &ConnectionConfig, seeded: usize) {
        let keys = store.written_keys();
        for key in keys.iter().skip(seeded) {
            assert!(
                !key.contains(&src.id.to_string()),
                "only copy keys are checked"
            );
            assert_eq!(store.get(key).unwrap(), None, "left behind: {key}");
        }
        for kind in [SecretKind::Password, SecretKind::SshPassword] {
            assert!(
                store.get(&secret_key(src.id, kind)).unwrap().is_some(),
                "source intact"
            );
        }
    }

    fn seed(store: &Recording, src: &ConnectionConfig) -> usize {
        store
            .set(&secret_key(src.id, SecretKind::Password), "pw")
            .unwrap();
        store
            .set(&secret_key(src.id, SecretKind::SshPassword), "sp")
            .unwrap();
        store.written_keys().len()
    }

    #[test]
    fn duplicate_that_fails_to_copy_secrets_adds_nothing() {
        let (_dir, path, _memory) = setup();
        // The password copy succeeds and the SSH password copy fails: copy_all must roll back.
        let secrets = Recording::new(Some("ssh-password"));
        let src = ConnectionConfig::new("prod", "mongodb://h");
        secrets
            .inner
            .set(&secret_key(src.id, SecretKind::Password), "pw")
            .unwrap();
        secrets
            .inner
            .set(&secret_key(src.id, SecretKind::SshPassword), "sp")
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
        assert!(
            state.banner.is_some(),
            "the failure is surfaced, not swallowed"
        );
        assert!(!path.as_ref().unwrap().exists(), "nothing was written");
        assert_eq!(
            secrets.written_keys().len(),
            1,
            "the first kind really was written"
        );
        assert_no_copy_secrets_remain(&secrets, &src, 0);
    }

    #[test]
    fn duplicate_whose_persist_fails_removes_the_copied_secrets() {
        let (dir, _path, _memory) = setup();
        // A path whose parent is a file cannot be written.
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let path: ConfigPath = Ok(blocker.join("connections.json"));
        let secrets = Recording::new(None);
        let src = ConnectionConfig::new("prod", "mongodb://h");
        let seeded = seed(&secrets, &src);
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
        assert_eq!(
            secrets.written_keys().len(),
            seeded + 2,
            "both secrets were copied first"
        );
        assert_no_copy_secrets_remain(&secrets, &src, seeded);
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

    fn with_prompt(state: AppState, prompt: FolderPrompt) -> AppState {
        AppState {
            conn: ConnectionsState {
                folder_prompt: Some(prompt),
                ..state.conn
            },
            ..state
        }
    }

    #[test]
    fn a_successful_move_or_rename_closes_the_folder_prompt() {
        let (_dir, path, _secrets) = setup();
        let mut a = ConnectionConfig::new("a", "mongodb://h");
        a.folder = Some("old".into());
        let prompt = FolderPrompt::Move {
            id: a.id,
            input: "new".into(),
        };
        let state = with_prompt(state_with(vec![a.clone()]), prompt);
        let state = move_to_folder(state, a.id, "new".into(), 5, &path);
        assert_eq!(state.conn.folder_prompt, None);
        let prompt = FolderPrompt::Rename {
            from: "new".into(),
            input: "newer".into(),
        };
        let state = with_prompt(state, prompt);
        let state = rename_folder(state, "new".into(), "newer".into(), 6, &path);
        assert_eq!(state.conn.folder_prompt, None);
        assert_eq!(state.conn.connections[0].folder.as_deref(), Some("newer"));
    }

    #[test]
    fn a_rejected_move_or_rename_keeps_the_prompt_open_with_the_typed_text() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let moving = FolderPrompt::Move {
            id: a.id,
            input: "bad\nname".into(),
        };
        let state = with_prompt(state_with(vec![a.clone()]), moving.clone());
        let state = move_to_folder(state, a.id, "bad\nname".into(), 5, &path);
        assert_eq!(state.conn.folder_prompt, Some(moving));
        assert_eq!(state.conn.error, Some(CoreError::InvalidFolderName));
        let renaming = FolderPrompt::Rename {
            from: "x".into(),
            input: "  ".into(),
        };
        let state = with_prompt(state, renaming.clone());
        let state = rename_folder(state, "x".into(), "  ".into(), 6, &path);
        assert_eq!(state.conn.folder_prompt, Some(renaming));
        assert_eq!(state.conn.error, Some(CoreError::InvalidFolderName));
    }

    fn editing(form: ConnectionForm, list: Vec<ConnectionConfig>) -> AppState {
        let mut state = state_with(list);
        state.conn.form = form;
        state.conn.editing = true;
        state.conn.uri_input = "pasted".into();
        state.conn.notice = Some(crate::views::connections::Notice::Exported { count: 1 });
        state
    }

    fn named_form(name: &str) -> ConnectionForm {
        ConnectionForm {
            name: name.into(),
            uri: "mongodb://h".into(),
            ..ConnectionForm::default()
        }
    }

    #[test]
    fn a_successful_save_closes_the_dialog_resets_the_tools_and_drops_the_notice() {
        let (_dir, path, secrets) = setup();
        let form = named_form("a");
        let state = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        assert_eq!(state.conn.connections.len(), 1);
        assert!(!state.conn.editing);
        assert!(state.conn.uri_input.is_empty() && state.conn.notice.is_none());
        assert!(state.banner.is_none());
    }

    #[test]
    fn a_rejected_save_keeps_the_dialog_open_with_the_error() {
        let (_dir, path, secrets) = setup();
        let form = named_form("   ");
        let state = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        assert!(state.conn.editing);
        assert_eq!(state.conn.error, Some(CoreError::InvalidName));
        assert_eq!(state.conn.uri_input, "pasted", "tools untouched");
    }

    #[test]
    fn a_blocked_save_keeps_the_dialog_open_even_if_the_same_banner_was_showing() {
        let (_dir, path, secrets) = setup();
        let form = named_form("a");
        let mut state = editing(form.clone(), vec![]);
        state.persist = PersistGuard::blocked();
        let first = save_and_close(state, &form, &secrets, &path, 5);
        assert!(first.conn.editing && first.banner.is_some());
        let shown = first.banner.clone();
        let again = save_and_close(first, &form, &secrets, &path, 6);
        assert!(again.conn.editing, "an identical banner is not a success");
        assert_eq!(again.banner, shown);
        assert!(again.conn.connections.is_empty());
    }

    #[test]
    fn a_save_whose_write_fails_keeps_the_dialog_open_with_a_banner() {
        let (dir, _path, secrets) = setup();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let path: ConfigPath = Ok(blocker.join("connections.json"));
        let form = named_form("a");
        let state = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        assert!(state.conn.editing);
        assert!(matches!(state.banner, Some(Banner::Persist(_))));
    }

    #[test]
    fn duplicate_opens_the_copy_in_the_dialog_with_fresh_tools() {
        let (_dir, path, secrets) = setup();
        let src = ConnectionConfig::new("prod", "mongodb://h");
        let mut state = state_with(vec![src.clone()]);
        state.conn.uri_input = "pasted".into();
        state.conn.notice = Some(crate::views::connections::Notice::Exported { count: 1 });
        let state = duplicate(state, src.id, "copy".into(), 50, &secrets, &path);
        assert!(state.conn.editing && state.conn.uri_input.is_empty());
        assert!(state.conn.notice.is_none());
    }

    #[test]
    fn deleting_the_edited_connection_closes_the_dialog_but_deleting_another_does_not() {
        let (_dir, path, secrets) = setup();
        let (a, b) = (
            ConnectionConfig::new("a", "mongodb://h"),
            ConnectionConfig::new("b", "mongodb://h"),
        );
        let form = ConnectionForm::from_config(&a);
        let state = editing(form, vec![a.clone(), b.clone()]);
        let state = delete_and_close(state, b.id, &secrets, &path);
        assert!(state.conn.editing, "another connection was deleted");
        let state = delete_and_close(state, a.id, &secrets, &path);
        assert!(!state.conn.editing && state.conn.uri_input.is_empty());
    }

    #[test]
    fn a_folder_change_refused_by_the_guard_closes_the_prompt_too() {
        let (_dir, path, _secrets) = setup();
        let a = ConnectionConfig::new("a", "mongodb://h");
        let prompt = FolderPrompt::Move {
            id: a.id,
            input: "x".into(),
        };
        let mut state = with_prompt(state_with(vec![a.clone()]), prompt);
        state.persist = PersistGuard::blocked();
        let state = move_to_folder(state, a.id, "x".into(), 5, &path);
        assert_eq!(state.conn.folder_prompt, None);
        assert!(state.banner.is_some());
        let prompt = FolderPrompt::Rename {
            from: "x".into(),
            input: "y".into(),
        };
        let state = with_prompt(state, prompt);
        let state = rename_folder(state, "x".into(), "y".into(), 6, &path);
        assert_eq!(state.conn.folder_prompt, None);
        assert_eq!(state.conn.connections[0].folder, None, "nothing changed");
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
