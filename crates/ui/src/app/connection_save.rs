//! Save from the edit dialog: runs `connections::save` and decides whether the dialog closes.
use super::connection_ops::with_conn;
use super::connections::{ConfigPath, save};
use super::state::{AppState, Banner};
use crate::viewmodel::ConnectionForm;
use crate::views::connections::Notice;
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

/// A banner about an earlier failed save: the keychain write or the file write.
fn is_save_failure(banner: &Banner) -> bool {
    matches!(
        banner,
        Banner::Persist(_)
            | Banner::Failure {
                context: "save_connection",
                ..
            }
    )
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
    // Open sessions (browse or compare) keep the settings they connected with: an edit only shows
    // on the next connect, apart from ticking Production, which applies at once.
    let edits_open_session = state.is_open(form.id) || state.compare.involves(form.id);
    // Save against an empty banner slot so any banner seen afterwards came from this save, even
    // when an identical one was already showing; the old banner is restored if none came.
    let prior = state.banner.clone();
    let probe = AppState {
        banner: None,
        ..state
    };
    let next = save(probe, form, secrets, path, now);
    let outcome = classify(blocked, next.conn.error.is_some(), next.banner.as_ref());
    // A save that went through cleanly makes the earlier "could not save" banners stale: drop
    // those (and only those) instead of restoring them.
    let prior = match outcome {
        SaveOutcome::Closed => prior.filter(|b| !is_save_failure(b)),
        SaveOutcome::StayOpen => prior,
    };
    // A banner this save raised while the dialog stays open belongs to the dialog, even if an
    // identical one was showing when it opened.
    let claims_banner = outcome == SaveOutcome::StayOpen && next.banner.is_some();
    let watch = match claims_banner {
        true => next.dialog_banner_watch.claimed(),
        false => next.dialog_banner_watch,
    };
    let next = AppState {
        banner: next.banner.or(prior),
        dialog_banner_watch: watch,
        ..next
    };
    match outcome {
        SaveOutcome::Closed => with_conn(next, |conn| {
            conn.saved();
            conn.notice = edits_open_session.then_some(Notice::AppliesOnReconnect);
        }),
        SaveOutcome::StayOpen => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::persistence::{PersistError, PersistGuard};
    use crate::app::state::ConnStatus;
    use crate::views::connections::ConnectionsState;
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
    fn a_successful_retry_drops_the_stale_save_banners_but_keeps_unrelated_ones() {
        let (_dir, path, secrets) = setup();
        let stale_banners = [
            Banner::Failure {
                context: "save_connection",
                error: CoreError::Secret("x".into()),
            },
            Banner::Persist(PersistError::Write(std::io::ErrorKind::Other)),
        ];
        for stale in stale_banners {
            let mut state = editing(named_form("a"), Vec::new());
            state.banner = Some(stale.clone());
            let next = save_and_close(state, &named_form("a"), &secrets, &path, 1);
            assert_eq!(next.banner, None, "{stale:?}");
        }
        let mut state = editing(named_form("b"), Vec::new());
        state.banner = Some(Banner::NotConnected);
        let next = save_and_close(state, &named_form("b"), &secrets, &path, 1);
        assert_eq!(next.banner, Some(Banner::NotConnected));
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

    fn existing(name: &str) -> ConnectionConfig {
        ConnectionConfig::new(name, "mongodb://h")
    }

    /// `state` with `cfg` saved, the dialog open on it, and `open` sessions.
    fn editing_saved(cfg: &ConnectionConfig, open: &[(Uuid, ConnStatus)]) -> AppState {
        let mut state = editing(ConnectionForm::from_config(cfg), vec![cfg.clone()]);
        for (id, status) in open {
            let view = crate::app::session::SessionView {
                status: *status,
                ..Default::default()
            };
            state = state.opened_with(*id, view);
        }
        state
    }

    #[test]
    fn editing_an_open_connection_says_changes_apply_on_reconnect() {
        let (_dir, path, secrets) = setup();
        let cfg = existing("a");
        let other = existing("b");
        for status in [
            ConnStatus::Connected,
            ConnStatus::Connecting,
            ConnStatus::Failed,
        ] {
            // Open as the active session, then park it behind another one.
            for parked in [false, true] {
                let mut open = vec![(cfg.id, status)];
                if parked {
                    open.push((other.id, ConnStatus::Connected));
                }
                let form = ConnectionForm {
                    name: "renamed".into(),
                    ..ConnectionForm::from_config(&cfg)
                };
                let state = editing_saved(&cfg, &open);
                assert!(state.is_open(cfg.id));
                let next = save_and_close(state, &form, &secrets, &path, 9);
                assert_eq!(
                    next.conn.notice,
                    Some(Notice::AppliesOnReconnect),
                    "{status:?} parked={parked}"
                );
            }
        }
    }

    #[test]
    fn no_edit_notice_for_a_closed_new_or_unsuccessful_save() {
        let (_dir, path, secrets) = setup();
        let cfg = existing("a");
        // Saved but not open.
        let form = ConnectionForm::from_config(&cfg);
        let next = save_and_close(editing_saved(&cfg, &[]), &form, &secrets, &path, 9);
        assert_eq!(next.conn.notice, None, "closed connection");
        // A new connection while another one is open.
        let other = existing("b");
        let state = editing_saved(&other, &[(other.id, ConnStatus::Connected)]);
        let fresh = named_form("new");
        let next = save_and_close(state, &fresh, &secrets, &path, 9);
        assert_eq!(next.conn.notice, None, "new connection");
        // An open connection whose save is rejected.
        let bad = ConnectionForm {
            name: "  ".into(),
            ..ConnectionForm::from_config(&cfg)
        };
        let state = editing_saved(&cfg, &[(cfg.id, ConnStatus::Connected)]);
        let next = save_and_close(state, &bad, &secrets, &path, 9);
        assert!(next.conn.editing);
        assert!(!matches!(
            next.conn.notice,
            Some(Notice::AppliesOnReconnect)
        ));
        // An open connection whose keychain write fails.
        let failing = Failing {
            inner: MemorySecretStore::new(),
            fail_set: true,
            fail_get_for: None,
        };
        let typed = ConnectionForm {
            password: "typed".into(),
            ..ConnectionForm::from_config(&cfg)
        };
        let state = editing_saved(&cfg, &[(cfg.id, ConnStatus::Connected)]);
        let next = save_and_close(state, &typed, &failing, &path, 9);
        assert!(!matches!(
            next.conn.notice,
            Some(Notice::AppliesOnReconnect)
        ));
    }

    #[test]
    fn a_duplicate_of_an_open_connection_gets_no_edit_notice() {
        let (_dir, path, secrets) = setup();
        let cfg = existing("a");
        let state = editing_saved(&cfg, &[(cfg.id, ConnStatus::Connected)]);
        let copy = ConnectionForm {
            id: Uuid::new_v4(),
            name: "a (copy)".into(),
            ..ConnectionForm::from_config(&cfg)
        };
        let next = save_and_close(state, &copy, &secrets, &path, 9);
        assert_eq!(next.conn.notice, None);
    }

    #[test]
    fn editing_a_connection_open_only_in_compare_says_changes_apply_on_reconnect() {
        let (_dir, path, secrets) = setup();
        let cfg = existing("a");
        let other = existing("b");
        let mut state = editing_saved(&cfg, &[]);
        state.compare.connections = Some((other.id, cfg.id));
        let form = ConnectionForm {
            production: true,
            ..ConnectionForm::from_config(&cfg)
        };
        let next = save_and_close(state, &form, &secrets, &path, 9);
        assert_eq!(next.conn.notice, Some(Notice::AppliesOnReconnect));
    }

    /// `state` with the dialog open and the watch having seen it open with `banner` showing.
    fn watching(mut state: AppState, banner: Option<Banner>) -> AppState {
        state.banner = banner.clone();
        state.dialog_banner_watch =
            std::mem::take(&mut state.dialog_banner_watch).observed(true, banner.as_ref());
        state
    }

    #[test]
    fn a_retried_keychain_failure_is_shown_again_in_the_dialog() {
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
        let first = save_and_close(editing(form.clone(), vec![]), &form, &secrets, &path, 5);
        let shown = first.banner.clone().expect("the failure banner");
        // Cancel and reopen with the failure still showing at the top, then retry.
        let reopened = watching(editing(form.clone(), vec![]), first.banner);
        assert!(!reopened.dialog_banner_watch.is_new(&shown), "predates");
        let again = save_and_close(reopened, &form, &secrets, &path, 6);
        assert_eq!(again.banner.as_ref(), Some(&shown));
        assert!(again.dialog_banner_watch.is_new(&shown), "shown again");
    }

    #[test]
    fn a_stale_banner_the_save_did_not_raise_stays_unclaimed() {
        let (_dir, path, secrets) = setup();
        let stale = Banner::NotConnected;
        let bad = named_form("   ");
        let state = watching(editing(bad.clone(), vec![]), Some(stale.clone()));
        let next = save_and_close(state, &bad, &secrets, &path, 5);
        assert!(next.conn.editing);
        assert!(!next.dialog_banner_watch.is_new(&stale));
    }

    #[test]
    fn the_edit_notice_follows_the_clearing_rules() {
        let mut conn = ConnectionsState {
            notice: Some(Notice::AppliesOnReconnect),
            ..ConnectionsState::default()
        };
        conn.filter_edited();
        assert_eq!(conn.notice, None);
        conn.notice = Some(Notice::AppliesOnReconnect);
        conn.saved();
        assert_eq!(conn.notice, None);
    }
}
