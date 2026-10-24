//! Tests of the app shell: runtime timeouts and the connection-action wiring.
use super::*;

#[test]
fn data_commands_keep_the_short_timeout_and_connect_outlasts_the_host_key_prompt() {
    let (prompt, _rx) = InteractivePrompt::channel(Arc::new(|| {}), HOST_KEY_PROMPT_TIMEOUT);
    let config = runtime_config(Some(PathBuf::from("/kh")), None, prompt);
    assert_eq!(config.command_timeout, COMMAND_TIMEOUT);
    assert!(config.connect_timeout > HOST_KEY_PROMPT_TIMEOUT);
    assert_eq!(config.known_hosts_path, Some(PathBuf::from("/kh")));
}

/// An app whose runtime is two channels the test holds: it sees the commands and feeds events.
fn app_with_open_dialog(
    guard: persistence::PersistGuard,
) -> (
    CurlybraceApp,
    tokio::sync::mpsc::UnboundedSender<Event>,
    tokio::sync::mpsc::UnboundedReceiver<curlybrace_core::runtime::Command>,
    tempfile::TempDir,
    uuid::Uuid,
) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (events, rx) = tokio::sync::mpsc::unbounded_channel();
    let a = ConnectionConfig::new("a", "mongodb://a");
    let id = a.id;
    let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
    let dir = tempfile::tempdir().unwrap();
    let mut app = CurlybraceApp::new(RuntimeHandle { tx, rx }, I18n::english(), vec![a], secrets)
        .with_config_path(Ok(dir.path().join("connections.json")))
        .with_persist_guard(guard);
    let conn = &mut app.state_mut().conn;
    conn.editing = true;
    conn.selected = Some(id);
    conn.form.password = "typed".into();
    conn.uri_input = "mongodb://u:p@h".into();
    conn.built_uri = Some(curlybrace_core::secrets::SecretString::new(
        "mongodb://u:p@h",
    ));
    (app, events, commands, dir, id)
}

fn assert_dialog_closed_and_secrets_gone(app: &CurlybraceApp) {
    let conn = &app.state().conn;
    assert!(!conn.editing, "the dialog must not survive");
    assert!(conn.form.password.is_empty() && conn.uri_input.is_empty());
    assert!(conn.built_uri.is_none());
}

#[test]
fn a_completed_connect_closes_the_edit_dialog_and_drops_its_secrets() {
    for guard in [
        persistence::PersistGuard::default(),
        persistence::PersistGuard::blocked(),
    ] {
        let (mut app, events, _commands, _dir, id) = app_with_open_dialog(guard);
        app.dispatch(Input::Connect(id));
        let epoch = app.state().latest_requested_epoch;
        events
            .send(Event::Connected {
                session: id,
                epoch,
                topology: None,
            })
            .unwrap();
        app.pump();
        assert_eq!(app.state().status, ConnStatus::Connected);
        assert_dialog_closed_and_secrets_gone(&app);
    }
}

#[test]
fn a_connect_action_with_the_dialog_open_closes_it() {
    let (mut app, _events, _commands, _dir, id) =
        app_with_open_dialog(persistence::PersistGuard::default());
    app.handle_connection_action(ConnectionAction::Connect(id));
    assert_dialog_closed_and_secrets_gone(&app);
}

#[test]
fn a_stopped_runtime_also_ends_the_compare_session() {
    use crate::app::compare::CompareAction;
    use crate::viewmodel::compare::{RunOverview, RunView, SessionPhase};
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    drop(commands);
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let handle = RuntimeHandle { tx, rx };
    let a = ConnectionConfig::new("a", "mongodb://a");
    let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
    let mut app = CurlybraceApp::new(handle, I18n::english(), vec![a], secrets);
    app.state_mut().compare.session = Some(SessionPhase::Ready(4));
    app.state_mut().compare.next_session = 5;
    app.state_mut().compare.overview = RunOverview::default().started(2, &[(0, 1)]).0;
    // Any command now finds the runtime gone.
    app.dispatch(Input::Compare(CompareAction::Cancel));
    let compare = &app.state().compare;
    assert_eq!(compare.session, None);
    assert_eq!(compare.run_view(), RunView::Idle);
    assert_eq!(compare.next_session, 5, "ids are never reused");
    assert!(matches!(
        app.state().banner,
        Some(Banner::Failure {
            context: "runtime",
            ..
        })
    ));
}

#[test]
fn a_stopped_runtime_does_not_leave_a_test_running_forever() {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    drop(commands);
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let secrets = Arc::new(curlybrace_core::secrets::MemorySecretStore::default());
    let mut app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        Vec::new(),
        secrets,
    );
    let form = crate::viewmodel::ConnectionForm {
        name: "n".into(),
        uri: "mongodb://localhost:27017".into(),
        ..Default::default()
    };
    app.dispatch(Input::TestConnection(Box::new(form)));
    assert_eq!(app.state().conn.test, TestState::Idle);
    assert!(matches!(
        app.state().banner,
        Some(Banner::Failure {
            context: "runtime",
            ..
        })
    ));
}

/// What `launch` builds: the shipped app records history at the default location.
#[test]
fn the_shipped_runtime_config_records_index_history() {
    let (prompt, _rx) = InteractivePrompt::channel(Arc::new(|| {}), HOST_KEY_PROMPT_TIMEOUT);
    let config = runtime_config(None, default_history_path(), prompt);
    assert_eq!(config.history_path, default_history_path());
    // Not vacuous: wherever the OS config directory is known, history really is on.
    if dirs::config_dir().is_some() {
        assert!(config.history_path.is_some());
    }
}
