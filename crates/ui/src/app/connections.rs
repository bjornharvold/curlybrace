//! Connection manager operations that touch the keychain and the config file. They take the
//! side-effect seams as arguments (`SecretStore`, path) so tests run against memory and a tempdir.
use super::persistence::{PersistError, PersistGuard, back_up, save_connections, startup};
use super::state::{AppState, Banner};
use crate::viewmodel::{ConnectionForm, save_connection, stored_secret_flags};
use crate::views::connections::ConnectionsState;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::error::CoreError;
use mongostudio_core::secrets::{SecretKind, SecretStore, secret_key};
use std::path::PathBuf;
use uuid::Uuid;

const ALL_KINDS: [SecretKind; 3] = [
    SecretKind::Password,
    SecretKind::SshPassword,
    SecretKind::SshPassphrase,
];

/// Where the connection list lives; an unknown config dir stays a typed error.
pub type ConfigPath = Result<PathBuf, PersistError>;

/// The saved list for startup, the save guard and the banner to show about it (if any).
/// Anything unusual about the file is backed up before it can be overwritten; if that fails
/// the guard refuses saves.
pub fn load_startup(path: &ConfigPath) -> (Vec<ConnectionConfig>, PersistGuard, Option<Banner>) {
    let path = match path {
        Ok(p) => p,
        Err(e) => {
            return (
                Vec::new(),
                PersistGuard::default(),
                Some(Banner::Persist(e.clone())),
            );
        }
    };
    let started = startup(path, &back_up);
    let banner = started.recovery.map(Banner::Recovery);
    (started.connections, started.guard, banner)
}

fn persist(path: &ConfigPath, list: &[ConnectionConfig]) -> Result<(), PersistError> {
    let result = match path {
        Ok(p) => save_connections(p, list),
        Err(e) => Err(e.clone()),
    };
    if let Err(e) = &result {
        eprintln!("could not save the connection list: {e:?}");
    }
    result
}

/// Recomputes which secrets exist for every saved connection. A keychain error is surfaced as a
/// banner; the previous flags for that connection are kept rather than assumed empty.
pub fn refresh_flags(state: AppState, secrets: &dyn SecretStore) -> AppState {
    let mut stored = state.conn.stored.clone();
    let mut error: Option<CoreError> = None;
    for cfg in &state.conn.connections {
        match stored_secret_flags(cfg.id, secrets) {
            Ok(flags) => {
                stored.insert(cfg.id, flags);
            }
            Err(e) => error = Some(e),
        }
    }
    let banner = match error {
        Some(e) => Some(Banner::Failure {
            context: "secrets",
            error: e,
        }),
        None => state.banner,
    };
    AppState {
        conn: ConnectionsState {
            stored,
            ..state.conn
        },
        banner,
        ..state
    }
}

fn with_conn_error(state: AppState, error: CoreError) -> AppState {
    AppState {
        conn: ConnectionsState {
            error: Some(error),
            ..state.conn
        },
        ..state
    }
}

/// Validates the form, stores its secrets, saves the config list, and returns the form to
/// "unchanged" by re-selecting the saved connection.
pub fn save(
    state: AppState,
    form: &ConnectionForm,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
) -> AppState {
    let cfg = match form.to_config() {
        Ok(cfg) => cfg,
        Err(e) => return with_conn_error(state, e),
    };
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    if let Err(error) = save_connection(&cfg, form, secrets) {
        let banner = Some(Banner::Failure {
            context: "save_connection",
            error,
        });
        return AppState { banner, ..state };
    }
    let list = upserted(&state.conn.connections, cfg.clone());
    let banner = persist(path, &list).err().map(Banner::Persist);
    let next = AppState {
        conn: ConnectionsState {
            connections: list,
            selected: Some(cfg.id),
            form: ConnectionForm::from_config(&cfg),
            error: None,
            ..state.conn
        },
        banner: banner.or(state.banner),
        ..state
    };
    refresh_flags(next, secrets)
}

fn upserted(list: &[ConnectionConfig], cfg: ConnectionConfig) -> Vec<ConnectionConfig> {
    let mut out = list.to_vec();
    match out.iter_mut().find(|c| c.id == cfg.id) {
        Some(slot) => *slot = cfg,
        None => out.push(cfg),
    }
    out
}

fn refused(state: AppState, error: PersistError) -> AppState {
    AppState {
        banner: Some(Banner::Persist(error)),
        ..state
    }
}

/// Removes the connection's keychain secrets (all three kinds are always attempted) and then,
/// only if every deletion succeeded, the config itself. A failure keeps the config so the user
/// can retry instead of leaving orphaned secrets behind.
pub fn delete(state: AppState, id: Uuid, secrets: &dyn SecretStore, path: &ConfigPath) -> AppState {
    if let Err(e) = state.persist.check() {
        return refused(state, e);
    }
    let failures: Vec<CoreError> = ALL_KINDS
        .iter()
        .filter_map(|kind| secrets.delete(&secret_key(id, *kind)).err())
        .collect();
    if let Some(error) = failures.into_iter().next() {
        let banner = Some(Banner::Failure {
            context: "delete_secrets",
            error,
        });
        return AppState { banner, ..state };
    }
    let list: Vec<ConnectionConfig> = state
        .conn
        .connections
        .iter()
        .filter(|c| c.id != id)
        .cloned()
        .collect();
    let banner = persist(path, &list)
        .err()
        .map(Banner::Persist)
        .or(state.banner);
    let was_selected = state.conn.selected == Some(id);
    let mut stored = state.conn.stored;
    stored.remove(&id);
    let conn = ConnectionsState {
        connections: list,
        stored,
        selected: if was_selected {
            None
        } else {
            state.conn.selected
        },
        form: if was_selected {
            ConnectionForm::default()
        } else {
            state.conn.form
        },
        error: None,
    };
    AppState {
        conn,
        banner,
        ..state
    }
}

#[cfg(test)]
mod tests {
    use super::super::persistence::{Recovery, load_connections};
    use super::*;
    use crate::viewmodel::SecretEdit;
    use mongostudio_core::secrets::MemorySecretStore;

    fn form(name: &str, password: &str) -> ConnectionForm {
        ConnectionForm {
            name: name.into(),
            uri: "mongodb://localhost:27017".into(),
            password: password.into(),
            ..ConnectionForm::default()
        }
    }

    fn path(dir: &tempfile::TempDir) -> ConfigPath {
        Ok(dir.path().join("connections.json"))
    }

    struct FailingStore;
    impl SecretStore for FailingStore {
        fn get(&self, _: &str) -> mongostudio_core::error::Result<Option<String>> {
            Err(CoreError::Secret("locked".into()))
        }
        fn set(&self, _: &str, _: &str) -> mongostudio_core::error::Result<()> {
            Err(CoreError::Secret("locked".into()))
        }
        fn delete(&self, _: &str) -> mongostudio_core::error::Result<()> {
            Err(CoreError::Secret("locked".into()))
        }
    }

    #[test]
    fn save_stores_secret_persists_list_and_resets_the_form() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        let f = form("prod", "hunter2");
        let s = save(AppState::new(vec![]), &f, &secrets, &path(&dir));
        assert_eq!(s.conn.connections.len(), 1);
        assert_eq!(s.conn.selected, Some(f.id));
        assert_eq!(s.conn.form.password, "");
        assert_eq!(s.conn.form.password_edit(), SecretEdit::Unchanged);
        assert!(s.conn.stored[&f.id].password);
        assert_eq!(s.banner, None);
        let text = std::fs::read_to_string(dir.path().join("connections.json")).unwrap();
        assert!(text.contains("prod") && !text.contains("hunter2"));
        assert_eq!(
            secrets
                .get(&secret_key(f.id, SecretKind::Password))
                .unwrap(),
            Some("hunter2".into())
        );
    }

    #[test]
    fn saving_again_updates_in_place_and_keeps_an_unchanged_secret() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        let f = form("prod", "pw");
        let s = save(AppState::new(vec![]), &f, &secrets, &path(&dir));
        let edited = ConnectionForm {
            name: "prod2".into(),
            ..s.conn.form.clone()
        };
        let s = save(s, &edited, &secrets, &path(&dir));
        assert_eq!(s.conn.connections.len(), 1);
        assert_eq!(s.conn.connections[0].name, "prod2");
        assert!(s.conn.stored[&f.id].password);
    }

    #[test]
    fn invalid_form_sets_the_form_error_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        let s = save(
            AppState::new(vec![]),
            &form("", "pw"),
            &secrets,
            &path(&dir),
        );
        assert!(s.conn.error.is_some() && s.conn.connections.is_empty());
        assert!(!dir.path().join("connections.json").exists());
    }

    #[test]
    fn keychain_failure_on_save_is_a_banner_and_nothing_is_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let s = save(
            AppState::new(vec![]),
            &form("a", "pw"),
            &FailingStore,
            &path(&dir),
        );
        assert!(matches!(
            s.banner,
            Some(Banner::Failure {
                context: "save_connection",
                ..
            })
        ));
        assert!(s.conn.connections.is_empty());
        assert!(!dir.path().join("connections.json").exists());
    }

    #[test]
    fn unknown_config_dir_is_reported_but_the_connection_stays_usable() {
        let secrets = MemorySecretStore::new();
        let s = save(
            AppState::new(vec![]),
            &form("a", ""),
            &secrets,
            &Err(PersistError::ConfigDirUnknown),
        );
        assert_eq!(s.conn.connections.len(), 1);
        assert_eq!(
            s.banner,
            Some(Banner::Persist(PersistError::ConfigDirUnknown))
        );
    }

    #[test]
    fn keychain_errors_while_reading_flags_are_a_banner_not_no_secret() {
        let cfg = ConnectionConfig::new("a", "mongodb://localhost");
        let id = cfg.id;
        let mut start = AppState::new(vec![cfg]);
        start.conn.stored.insert(
            id,
            crate::viewmodel::StoredSecrets {
                password: true,
                ssh_secret: false,
            },
        );
        let s = refresh_flags(start, &FailingStore);
        assert!(matches!(
            s.banner,
            Some(Banner::Failure {
                context: "secrets",
                ..
            })
        ));
        assert!(s.conn.stored[&id].password, "previous flags are kept");
    }

    #[test]
    fn delete_removes_config_and_all_three_secrets_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        let s = save(
            AppState::new(vec![]),
            &form("a", "pw"),
            &secrets,
            &path(&dir),
        );
        let id = s.conn.connections[0].id;
        for kind in ALL_KINDS {
            secrets.set(&secret_key(id, kind), "x").unwrap();
        }
        let s = delete(s, id, &secrets, &path(&dir));
        assert!(s.conn.connections.is_empty() && s.conn.selected.is_none());
        assert!(!s.conn.stored.contains_key(&id));
        for kind in ALL_KINDS {
            assert_eq!(secrets.get(&secret_key(id, kind)).unwrap(), None);
        }
        let loaded = load_connections(&dir.path().join("connections.json")).unwrap();
        assert!(loaded.connections.is_empty());
        // Idempotent: deleting again is fine and shows no banner.
        let s = delete(s, id, &secrets, &path(&dir));
        assert_eq!(s.banner, None);
    }

    /// Fails `delete` for one secret kind while `broken` is set; records every attempt.
    struct FlakyStore {
        inner: MemorySecretStore,
        fail_suffix: &'static str,
        broken: std::sync::atomic::AtomicBool,
        attempts: std::sync::Mutex<Vec<String>>,
    }

    impl SecretStore for FlakyStore {
        fn get(&self, key: &str) -> mongostudio_core::error::Result<Option<String>> {
            self.inner.get(key)
        }
        fn set(&self, key: &str, value: &str) -> mongostudio_core::error::Result<()> {
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> mongostudio_core::error::Result<()> {
            self.attempts.lock().unwrap().push(key.to_string());
            let broken = self.broken.load(std::sync::atomic::Ordering::SeqCst);
            if broken && key.ends_with(self.fail_suffix) {
                return Err(CoreError::Secret("locked".into()));
            }
            self.inner.delete(key)
        }
    }

    #[test]
    fn a_failing_secret_delete_keeps_the_config_attempts_every_kind_and_retry_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = FlakyStore {
            inner: MemorySecretStore::new(),
            fail_suffix: "ssh-password",
            broken: std::sync::atomic::AtomicBool::new(false),
            attempts: Default::default(),
        };
        let s = save(AppState::new(vec![]), &form("a", "pw"), &store, &path(&dir));
        let id = s.conn.connections[0].id;
        for kind in ALL_KINDS {
            store.inner.set(&secret_key(id, kind), "x").unwrap();
        }
        store.attempts.lock().unwrap().clear();
        store
            .broken
            .store(true, std::sync::atomic::Ordering::SeqCst);

        let s = delete(s, id, &store, &path(&dir));
        assert_eq!(
            store.attempts.lock().unwrap().len(),
            3,
            "all kinds attempted"
        );
        assert_eq!(s.conn.connections.len(), 1, "config kept for a retry");
        assert!(matches!(
            s.banner,
            Some(Banner::Failure {
                context: "delete_secrets",
                ..
            })
        ));
        let on_disk = load_connections(&dir.path().join("connections.json")).unwrap();
        assert_eq!(on_disk.connections.len(), 1);

        store
            .broken
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let s = delete(s, id, &store, &path(&dir));
        assert!(s.conn.connections.is_empty());
        for kind in ALL_KINDS {
            assert_eq!(store.inner.get(&secret_key(id, kind)).unwrap(), None);
        }
    }

    #[test]
    fn a_blocked_guard_refuses_save_and_delete_before_touching_anything() {
        let dir = tempfile::tempdir().unwrap();
        let secrets = MemorySecretStore::new();
        let mut start = AppState::new(vec![]);
        start.persist = PersistGuard::blocked();
        let s = save(start, &form("a", "pw"), &secrets, &path(&dir));
        assert_eq!(s.banner, Some(Banner::Persist(PersistError::Blocked)));
        assert!(s.conn.connections.is_empty());
        assert!(!dir.path().join("connections.json").exists());
        let id = form("a", "").id;
        secrets
            .set(&secret_key(id, SecretKind::Password), "x")
            .unwrap();
        let s = delete(s, id, &secrets, &path(&dir));
        assert_eq!(
            secrets.get(&secret_key(id, SecretKind::Password)).unwrap(),
            Some("x".into())
        );
        assert_eq!(s.banner, Some(Banner::Persist(PersistError::Blocked)));
    }

    #[test]
    fn startup_reports_recovery_and_never_overwrites_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("connections.json");
        let good = serde_json::to_value(ConnectionConfig::new("ok", "mongodb://h")).unwrap();
        std::fs::write(&file, serde_json::json!([good, {"id": 1}]).to_string()).unwrap();
        let (list, guard, banner) = load_startup(&Ok(file.clone()));
        assert_eq!(list.len(), 1);
        assert!(guard.check().is_ok());
        assert_eq!(
            banner,
            Some(Banner::Recovery(Recovery::Skipped {
                count: 1,
                backed_up: true
            }))
        );

        std::fs::write(&file, "{ corrupt").unwrap();
        let (list, _, banner) = load_startup(&Ok(file.clone()));
        assert!(list.is_empty());
        assert_eq!(
            banner,
            Some(Banner::Recovery(Recovery::Corrupt { backed_up: true }))
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ corrupt");

        let (_, _, banner) = load_startup(&Err(PersistError::ConfigDirUnknown));
        assert_eq!(
            banner,
            Some(Banner::Persist(PersistError::ConfigDirUnknown))
        );
    }
}
