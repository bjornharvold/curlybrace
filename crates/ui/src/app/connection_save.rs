//! Save from the edit dialog: runs `connections::save` and decides whether the dialog closes.
use super::connection_ops::with_conn;
use super::connections::{ConfigPath, save};
use super::state::{AppState, Banner};
use crate::viewmodel::ConnectionForm;
use crate::views::connections::ConnectionsState;
use curlybrace_core::secrets::SecretStore;

/// What the dialog does after a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SaveOutcome {
    Closed,
    StayOpen,
}

/// Classifies the result of a save from facts observed around it.
/// - A blocked guard or a form error: nothing was saved, stay open (the error or the banner
///   explains).
/// - A failed keychain write (`save_connection`): `save` returned early, the list and the typed
///   values are untouched, stay open.
/// - Anything else closes, including a failed flag refresh (`secrets`) and a failed file write
///   (`Persist`): the connection is saved in memory, and the banner reports the problem.
pub(super) fn classify(blocked: bool, has_error: bool, new_banner: Option<&Banner>) -> SaveOutcome {
    let keychain_write_failed = matches!(
        new_banner,
        Some(Banner::Failure {
            context: "save_connection",
            ..
        })
    );
    if blocked || has_error || keychain_write_failed {
        SaveOutcome::StayOpen
    } else {
        SaveOutcome::Closed
    }
}

/// Saves the form; on `Closed` the dialog closes, its tools reset and the notice drops.
pub(super) fn save_and_close(
    state: AppState,
    form: &ConnectionForm,
    secrets: &dyn SecretStore,
    path: &ConfigPath,
    now: i64,
) -> AppState {
    let blocked = state.persist.check().is_err();
    // Save against an empty banner slot so any banner seen afterwards came from this save, even
    // when an identical one was already showing; the old banner is restored if none came.
    let prior = state.banner.clone();
    let probe = AppState {
        banner: None,
        ..state
    };
    let next = save(probe, form, secrets, path, now);
    let outcome = classify(blocked, next.conn.error.is_some(), next.banner.as_ref());
    let next = AppState {
        banner: next.banner.or(prior),
        ..next
    };
    match outcome {
        SaveOutcome::Closed => with_conn(next, ConnectionsState::saved),
        SaveOutcome::StayOpen => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::persistence::PersistGuard;
    use curlybrace_core::connection::ConnectionConfig;
    use curlybrace_core::error::{CoreError, Result};
    use curlybrace_core::secrets::MemorySecretStore;
    use uuid::Uuid;

    fn setup() -> (tempfile::TempDir, ConfigPath, MemorySecretStore) {
        let dir = tempfile::tempdir().unwrap();
        let path: ConfigPath = Ok(dir.path().join("connections.json"));
        (dir, path, MemorySecretStore::new())
    }

    fn named_form(name: &str) -> ConnectionForm {
        ConnectionForm {
            name: name.into(),
            uri: "mongodb://h".into(),
            ..ConnectionForm::default()
        }
    }

    fn editing(form: ConnectionForm, list: Vec<ConnectionConfig>) -> AppState {
        let mut state = AppState::new(list);
        state.conn.form = form;
        state.conn.editing = true;
        state.conn.uri_input = "pasted".into();
        state.conn.notice = Some(crate::views::connections::Notice::Exported { count: 1 });
        state
    }

    /// Fails `set` always, and `get` for keys of one connection.
    struct Failing {
        inner: MemorySecretStore,
        fail_set: bool,
        fail_get_for: Option<Uuid>,
    }

    impl SecretStore for Failing {
        fn get(&self, k: &str) -> Result<Option<String>> {
            if self
                .fail_get_for
                .is_some_and(|id| k.contains(&id.to_string()))
            {
                return Err(CoreError::Secret("boom".into()));
            }
            self.inner.get(k)
        }
        fn set(&self, k: &str, v: &str) -> Result<()> {
            if self.fail_set {
                return Err(CoreError::Secret("boom".into()));
            }
            self.inner.set(k, v)
        }
        fn delete(&self, k: &str) -> Result<()> {
            self.inner.delete(k)
        }
    }

    #[test]
    fn the_classification_covers_every_branch() {
        let failure = |context| Banner::Failure {
            context,
            error: CoreError::Secret("x".into()),
        };
        assert_eq!(classify(false, false, None), SaveOutcome::Closed);
        assert_eq!(classify(true, false, None), SaveOutcome::StayOpen);
        assert_eq!(classify(false, true, None), SaveOutcome::StayOpen);
        let keychain = failure("save_connection");
        assert_eq!(
            classify(false, false, Some(&keychain)),
            SaveOutcome::StayOpen
        );
        let refresh = failure("secrets");
        assert_eq!(classify(false, false, Some(&refresh)), SaveOutcome::Closed);
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
    fn a_rejected_form_keeps_the_dialog_open_with_the_error() {
        let (_dir, path, secrets) = setup();
        let form = named_form("   ");
        let state = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        assert!(state.conn.editing);
        assert_eq!(state.conn.error, Some(CoreError::InvalidName));
        assert_eq!(state.conn.uri_input, "pasted", "tools untouched");
    }

    #[test]
    fn a_blocked_guard_keeps_the_dialog_open_even_if_the_same_banner_was_showing() {
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
    fn a_failed_keychain_write_keeps_the_dialog_open_and_the_typed_password() {
        let (_dir, path, _memory) = setup();
        let secrets = Failing {
            inner: MemorySecretStore::new(),
            fail_set: true,
            fail_get_for: None,
        };
        let form = ConnectionForm {
            password: "typed".into(),
            ..named_form("a")
        };
        let state = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        assert!(state.conn.editing);
        assert!(matches!(
            state.banner,
            Some(Banner::Failure {
                context: "save_connection",
                ..
            })
        ));
        assert_eq!(state.conn.form.password, "typed", "the user can retry");
        assert!(state.conn.connections.is_empty());
    }

    #[test]
    fn a_failed_flag_refresh_still_closes_because_the_save_went_through() {
        let (_dir, path, memory) = setup();
        let other = ConnectionConfig::new("other", "mongodb://o");
        let secrets = Failing {
            inner: memory,
            fail_set: false,
            fail_get_for: Some(other.id),
        };
        let form = named_form("a");
        let state = save_and_close(
            editing(form.clone(), vec![other]),
            &form,
            &secrets,
            &path,
            5,
        );
        assert_eq!(state.conn.connections.len(), 2, "the save went through");
        assert!(!state.conn.editing);
        assert!(matches!(
            state.banner,
            Some(Banner::Failure {
                context: "secrets",
                ..
            })
        ));
    }

    #[test]
    fn a_failed_file_write_closes_with_the_persist_banner() {
        let (dir, _path, secrets) = setup();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, b"x").unwrap();
        let path: ConfigPath = Ok(blocker.join("connections.json"));
        let form = named_form("a");
        let state = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        assert_eq!(state.conn.connections.len(), 1, "kept in memory");
        assert!(!state.conn.editing);
        assert!(matches!(state.banner, Some(Banner::Persist(_))));
    }
}
