//! The global switch for the automatic index verdict: the `explain.json` file, the reducer rules
//! and the save through the app. Every file lives in a tempdir.
use super::explain_actions::VerdictState;
use super::explain_settings::{ExplainSettings, SaveError, load, save, saved};
use super::explain_tests::{act, browsing, explained, ns, query_a, verdict};
use super::persistence::{self, PersistError, Recovery, back_up};
use super::reducer::Input;
use super::*;
use crate::viewmodel::FindAction;
use curlybrace_core::runtime::Command;
use curlybrace_core::secrets::MemorySecretStore;

fn off() -> ExplainSettings {
    ExplainSettings { automatic: false }
}

fn app_in(dir: &tempfile::TempDir) -> CurlybraceApp {
    let (tx, _commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        Arc::new(MemorySecretStore::default()),
    )
    .with_explain_path(Ok(dir.path().join("explain.json")))
}

#[test]
fn defaults_to_automatic() {
    assert!(ExplainSettings::default().automatic);
    assert!(AppState::new(vec![]).explain_enabled());
}

#[test]
fn missing_file_is_default() {
    let dir = tempfile::tempdir().unwrap();
    let started = load(&dir.path().join("explain.json"), &back_up);
    assert_eq!(started.settings, ExplainSettings::default());
    assert_eq!(started.recovery, None);
    assert_eq!(started.guard.check(), Ok(()));
}

#[test]
fn corrupt_file_is_backed_up_and_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("explain.json");
    for bad in ["{ not json", "[true]", "{\"automatic\": \"no\"}"] {
        std::fs::write(&path, bad).unwrap();
        let started = load(&path, &back_up);
        assert_eq!(started.settings, ExplainSettings::default(), "{bad}");
        assert_eq!(
            started.recovery,
            Some(Recovery::Explain { backed_up: true }),
            "{bad}"
        );
        assert_eq!(started.guard.check(), Ok(()));
    }
    let failing = |_: &std::path::Path, _: &str| Err(std::io::ErrorKind::PermissionDenied.into());
    std::fs::write(&path, "{ not json").unwrap();
    let blocked = load(&path, &failing);
    assert_eq!(
        blocked.recovery,
        Some(Recovery::Explain { backed_up: false })
    );
    assert_eq!(blocked.guard.check(), Err(PersistError::Blocked));
}

#[test]
fn a_missing_field_is_automatic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("explain.json");
    std::fs::write(&path, "{}").unwrap();
    assert!(load(&path, &back_up).settings.automatic);
}

#[test]
fn round_trips_false() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("explain.json");
    save(&path, &off()).unwrap();
    assert_eq!(load(&path, &back_up).settings, off());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"automatic\": false"), "{text}");
    save(&path, &ExplainSettings::default()).unwrap();
    assert_eq!(load(&path, &back_up).settings, ExplainSettings::default());
}

#[test]
fn disabled_run_issues_only_find() {
    let state = AppState {
        explain: off(),
        ..browsing()
    };
    assert!(!state.explain_enabled());
    let (s, cmds) = act(state, FindAction::Run(query_a()));
    assert!(
        matches!(cmds.as_slice(), [Command::Find { .. }]),
        "{cmds:?}"
    );
    assert_eq!(s.verdict, VerdictState::None);
}

#[test]
fn toggling_off_clears_the_verdict() {
    let (s, _) = act(browsing(), FindAction::Run(query_a()));
    let s = explained(s, ns(), query_a(), Ok(verdict()));
    assert!(matches!(s.verdict, VerdictState::Ready { .. }));
    let (s, cmds) = act(s, FindAction::SetAutomaticVerdict(false));
    assert!(!s.explain_enabled());
    assert_eq!(s.verdict, VerdictState::None);
    assert!(cmds.is_empty(), "{cmds:?}");
    let (s, cmds) = act(s, FindAction::SetAutomaticVerdict(true));
    assert!(s.explain_enabled() && cmds.is_empty());
}

#[test]
fn toggling_persists_when_guard_allows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("explain.json");
    let mut app = app_in(&dir);
    app.dispatch(Input::Find(FindAction::SetAutomaticVerdict(false)));
    assert_eq!(load(&path, &back_up).settings, off());
    app.dispatch(Input::Find(FindAction::SetAutomaticVerdict(true)));
    assert_eq!(load(&path, &back_up).settings, ExplainSettings::default());
    assert_eq!(app.state().banner, None);
}

#[test]
fn toggling_does_not_write_when_guard_is_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("explain.json");
    std::fs::write(&path, "{ not json").unwrap();
    let mut app = app_in(&dir);
    app.explain_guard = persistence::PersistGuard::blocked();
    app.state_mut().banner = None;
    app.dispatch(Input::Find(FindAction::SetAutomaticVerdict(false)));
    assert!(!app.state().explain_enabled(), "applies for this run");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    assert!(matches!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::Explain { backed_up: false }))
    ));
    assert_eq!(
        saved(&path, &app.explain_guard, &off()),
        Err(SaveError::Blocked)
    );
}

#[test]
fn a_corrupt_file_at_launch_shows_the_recovery_banner() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("explain.json"), "nope").unwrap();
    let app = app_in(&dir);
    assert_eq!(
        app.state().banner,
        Some(Banner::Recovery(Recovery::Explain { backed_up: true }))
    );
    assert!(app.state().explain_enabled());
}
