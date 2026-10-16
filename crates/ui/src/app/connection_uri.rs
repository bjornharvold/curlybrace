//! To URI: builds the string for the form; a real password only on explicit request.
use super::state::AppState;
use crate::viewmodel::ConnectionForm;
use crate::views::connections::ConnectionsState;
use curlybrace_core::error::{CoreError, Result};
use curlybrace_core::secrets::{SecretKind, SecretStore, SecretString, secret_key};
use curlybrace_core::uri::{PasswordMode, build_uri};

/// The password to embed: the typed one wins; a saved one marked for removal is not read; else
/// the keychain one. A keychain error is an error,
/// never a silently masked string.
fn password_for(
    form: &ConnectionForm,
    cfg_id: uuid::Uuid,
    secrets: &dyn SecretStore,
) -> Result<Option<String>> {
    if !form.password.is_empty() {
        Ok(Some(form.password.clone()))
    } else if form.clear_password {
        Ok(None)
    } else {
        secrets.get(&secret_key(cfg_id, SecretKind::Password))
    }
}

fn with_error(state: AppState, error: CoreError) -> AppState {
    let conn = ConnectionsState {
        error: Some(error),
        ..state.conn
    };
    AppState { conn, ..state }
}

/// Builds the connection string for the form. Without `include_password` the output holds the
/// mask and the typed or stored password is never read.
pub(super) fn build(
    state: AppState,
    form: &ConnectionForm,
    include_password: bool,
    secrets: &dyn SecretStore,
) -> AppState {
    let cfg = match form.to_config() {
        Ok(cfg) => cfg,
        Err(error) => return with_error(state, error),
    };
    let password = if include_password {
        match password_for(form, cfg.id, secrets) {
            Ok(password) => password,
            Err(error) => return with_error(state, error),
        }
    } else {
        None
    };
    let mode = match password.as_deref() {
        Some(pw) => PasswordMode::Include(pw),
        None => PasswordMode::Masked,
    };
    let text = build_uri(&cfg, mode);
    let conn = ConnectionsState {
        built_uri: Some(SecretString::new(text)),
        built_for: Some(form.clone()),
        built_with_password: password.is_some(),
        error: None,
        ..state.conn
    };
    AppState { conn, ..state }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curlybrace_core::error::CoreError;
    use curlybrace_core::secrets::{MemorySecretStore, SecretKind, secret_key};

    fn form() -> ConnectionForm {
        ConnectionForm {
            name: "n".into(),
            uri: "mongodb://h".into(),
            username: "bob".into(),
            ..ConnectionForm::default()
        }
    }

    fn built(state: &AppState) -> String {
        state
            .conn
            .built_uri
            .as_ref()
            .expect("built")
            .expose()
            .to_owned()
    }

    #[test]
    fn a_stored_connection_with_a_bad_host_list_can_still_be_copied() {
        let f = ConnectionForm {
            uri: "mongodb://admin:2024/S3CR3T@db".into(),
            ..form()
        };
        let s = build(AppState::default(), &f, false, &MemorySecretStore::new());
        assert!(s.conn.error.is_none());
        assert!(s.conn.built_uri.is_some());
    }

    #[test]
    fn without_include_password_the_mask_is_used_and_the_typed_password_never_appears() {
        let f = ConnectionForm {
            password: "typedpw".into(),
            ..form()
        };
        let s = build(AppState::default(), &f, false, &MemorySecretStore::new());
        let text = built(&s);
        assert!(text.contains("****") && !text.contains("typedpw"));
        assert!(!s.conn.built_with_password);
        assert_eq!(s.conn.built_for, Some(f));
        assert!(!format!("{:?}", s.conn).contains("typedpw"));
    }

    #[test]
    fn a_typed_password_wins_over_the_stored_one_when_included() {
        let f = ConnectionForm {
            password: "typedpw".into(),
            ..form()
        };
        let store = MemorySecretStore::new();
        store
            .set(&secret_key(f.id, SecretKind::Password), "storedpw")
            .unwrap();
        let s = build(AppState::default(), &f, true, &store);
        let text = built(&s);
        assert!(text.contains("typedpw") && !text.contains("storedpw"));
        assert!(s.conn.built_with_password);
        let debug = format!("{:?}", s.conn);
        assert!(!debug.contains("typedpw") && !debug.contains("storedpw"));
    }

    #[test]
    fn a_cleared_saved_password_is_not_read_from_the_keychain_when_included() {
        let f = ConnectionForm {
            clear_password: true,
            ..form()
        };
        let store = MemorySecretStore::new();
        store
            .set(&secret_key(f.id, SecretKind::Password), "storedpw")
            .unwrap();
        let s = build(AppState::default(), &f, true, &store);
        assert!(!built(&s).contains("storedpw"));
        assert!(!s.conn.built_with_password);
    }

    #[test]
    fn a_typed_password_still_wins_when_the_saved_one_is_marked_for_removal() {
        let f = ConnectionForm {
            clear_password: true,
            password: "typedpw".into(),
            ..form()
        };
        let s = build(AppState::default(), &f, true, &MemorySecretStore::new());
        assert!(built(&s).contains("typedpw"));
        assert!(s.conn.built_with_password);
    }

    #[test]
    fn an_empty_password_field_reads_the_keychain_when_included() {
        let f = form();
        let store = MemorySecretStore::new();
        store
            .set(&secret_key(f.id, SecretKind::Password), "storedpw")
            .unwrap();
        let s = build(AppState::default(), &f, true, &store);
        assert!(built(&s).contains("storedpw"));
        assert!(s.conn.built_with_password);
        assert!(!format!("{:?}", s.conn).contains("storedpw"));
    }

    #[test]
    fn including_a_missing_password_builds_a_masked_string_without_the_flag() {
        let s = build(
            AppState::default(),
            &form(),
            true,
            &MemorySecretStore::new(),
        );
        assert!(built(&s).contains("****"));
        assert!(!s.conn.built_with_password);
    }

    #[test]
    fn a_keychain_read_error_is_reported_and_builds_nothing() {
        struct Broken;
        impl SecretStore for Broken {
            fn get(&self, _: &str) -> curlybrace_core::error::Result<Option<String>> {
                Err(CoreError::Secret("locked".into()))
            }
            fn set(&self, _: &str, _: &str) -> curlybrace_core::error::Result<()> {
                Ok(())
            }
            fn delete(&self, _: &str) -> curlybrace_core::error::Result<()> {
                Ok(())
            }
        }
        let s = build(AppState::default(), &form(), true, &Broken);
        assert!(s.conn.built_uri.is_none());
        assert_eq!(s.conn.error, Some(CoreError::Secret("locked".into())));
    }

    #[test]
    fn an_invalid_form_sets_an_error() {
        let s = build(
            AppState::default(),
            &ConnectionForm::default(),
            false,
            &MemorySecretStore::new(),
        );
        assert!(s.conn.built_uri.is_none() && s.conn.error.is_some());
    }

    #[test]
    fn a_new_build_replaces_the_previous_one_and_clears_the_error() {
        let mut s = build(
            AppState::default(),
            &form(),
            false,
            &MemorySecretStore::new(),
        );
        s.conn.error = Some(CoreError::AuthFailed);
        let s = build(s, &form(), false, &MemorySecretStore::new());
        assert!(s.conn.error.is_none() && s.conn.built_uri.is_some());
    }
}
