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
fn duplicate_of_a_near_max_length_name_still_validates() {
    for len in 195..=curlybrace_core::connection::MAX_NAME_LEN {
        let (_dir, path, secrets) = setup();
        let long = "a".repeat(len);
        let src = ConnectionConfig::new(&long, "mongodb://h").with_save_stamp(1);
        let state = duplicate(
            state_with(vec![src.clone()]),
            src.id,
            format!("{long} copy"),
            50,
            &secrets,
            &path,
        );
        assert_eq!(state.conn.connections.len(), 2, "len {len}");
        assert!(state.conn.connections[1].validate().is_ok(), "len {len}");
    }
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
    assert_eq!(
        state.conn.edit_mode,
        crate::views::connections::EditMode::Duplicate
    );
}

/// Reads succeed only for the keys of `readable`: the copy's flag read fails after the copy.
struct OnlyReadsOf {
    inner: MemorySecretStore,
    readable: Uuid,
}

impl SecretStore for OnlyReadsOf {
    fn get(&self, k: &str) -> curlybrace_core::error::Result<Option<String>> {
        match k.contains(&self.readable.to_string()) {
            true => self.inner.get(k),
            false => Err(CoreError::Secret("boom".into())),
        }
    }
    fn set(&self, k: &str, v: &str) -> curlybrace_core::error::Result<()> {
        self.inner.set(k, v)
    }
    fn delete(&self, k: &str) -> curlybrace_core::error::Result<()> {
        self.inner.delete(k)
    }
}

/// The watch as the screen leaves it one frame after `state` came out of an action.
fn next_frame(mut state: AppState) -> AppState {
    state.dialog_banner_watch = std::mem::take(&mut state.dialog_banner_watch)
        .observed(state.conn.editing, state.banner.as_ref());
    state
}

fn secrets_failure() -> Banner {
    Banner::Failure {
        context: "secrets",
        error: CoreError::Secret("boom".into()),
    }
}

#[test]
fn a_duplicate_whose_flag_read_fails_shows_that_failure_in_the_dialog() {
    let (_dir, path, _) = setup();
    let src = ConnectionConfig::new("prod", "mongodb://h");
    let secrets = OnlyReadsOf {
        inner: MemorySecretStore::new(),
        readable: src.id,
    };
    let state = duplicate(
        state_with(vec![src.clone()]),
        src.id,
        "c".into(),
        5,
        &secrets,
        &path,
    );
    assert!(state.conn.editing);
    assert_eq!(state.banner, Some(secrets_failure()));
    let state = next_frame(state);
    assert!(state.dialog_banner_watch.is_new(&secrets_failure()));
    let state = next_frame(state);
    assert!(
        state.dialog_banner_watch.is_new(&secrets_failure()),
        "stays"
    );
}

#[test]
fn a_duplicate_raising_the_same_failure_as_one_already_showing_still_shows_it() {
    let (_dir, path, _) = setup();
    let src = ConnectionConfig::new("prod", "mongodb://h");
    let secrets = OnlyReadsOf {
        inner: MemorySecretStore::new(),
        readable: src.id,
    };
    let mut state = state_with(vec![src.clone()]);
    state.banner = Some(secrets_failure());
    let state = duplicate(state, src.id, "c".into(), 5, &secrets, &path);
    assert!(
        next_frame(state)
            .dialog_banner_watch
            .is_new(&secrets_failure())
    );
}

#[test]
fn a_duplicate_that_raises_nothing_keeps_a_stale_banner_out_of_the_dialog() {
    let (_dir, path, secrets) = setup();
    let src = ConnectionConfig::new("prod", "mongodb://h");
    let mut state = state_with(vec![src.clone()]);
    state.banner = Some(Banner::NotConnected);
    let state = duplicate(state, src.id, "c".into(), 5, &secrets, &path);
    assert_eq!(state.banner, Some(Banner::NotConnected));
    let state = next_frame(state);
    assert!(state.conn.editing);
    assert!(!state.dialog_banner_watch.is_new(&Banner::NotConnected));
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
