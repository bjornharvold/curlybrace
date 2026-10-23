//! Saving a connection that has open sessions with `read_only` newly ticked sends the live lock.
use super::*;
use crate::views::connections::ConnectionAction;
use curlybrace_core::runtime::{Command, RuntimeHandle};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::test_support::mock_objects::mock_connection;
use std::sync::Arc;

type Commands = tokio::sync::mpsc::UnboundedReceiver<Command>;

fn fresh(read_only: bool) -> (CurlybraceApp, Commands, ConnectionConfig, tempfile::TempDir) {
    let (tx, commands) = tokio::sync::mpsc::unbounded_channel();
    let (_events, rx) = tokio::sync::mpsc::unbounded_channel();
    let cfg = ConnectionConfig {
        read_only,
        ..mock_connection()
    };
    let dir = tempfile::tempdir().unwrap();
    let app = CurlybraceApp::new(
        RuntimeHandle { tx, rx },
        I18n::english(),
        vec![cfg.clone()],
        Arc::new(MemorySecretStore::default()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")));
    (app, commands, cfg, dir)
}

fn drain(commands: &mut Commands) -> Vec<Command> {
    std::iter::from_fn(|| commands.try_recv().ok()).collect()
}

fn locks(commands: &[Command]) -> usize {
    commands
        .iter()
        .filter(|c| matches!(c, Command::LockReadOnly { .. }))
        .count()
}

fn saved_as(app: &mut CurlybraceApp, cfg: &ConnectionConfig, read_only: bool) {
    let form = crate::viewmodel::ConnectionForm {
        read_only,
        ..crate::viewmodel::ConnectionForm::from_config(cfg)
    };
    app.dispatch_connection(ConnectionAction::Save(Box::new(form)));
}

#[test]
fn saving_newly_read_only_connection_sends_lock_command() {
    let (mut app, mut commands, cfg, _dir) = fresh(false);
    app.dispatch(Input::Connect(cfg.id));
    drain(&mut commands);
    saved_as(&mut app, &cfg, true);
    let sent = drain(&mut commands);
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::LockReadOnly { connection } if *connection == cfg.id)),
        "{sent:?}"
    );
    assert_eq!(locks(&sent), 1);
}

#[test]
fn saving_does_not_lock_when_unchanged_unticked_or_closed() {
    // Already read-only and saved again: unchanged.
    let (mut app, mut commands, cfg, _dir) = fresh(true);
    app.dispatch(Input::Connect(cfg.id));
    drain(&mut commands);
    saved_as(&mut app, &cfg, true);
    assert_eq!(locks(&drain(&mut commands)), 0, "unchanged");
    // Unticking.
    saved_as(&mut app, &cfg, false);
    assert_eq!(locks(&drain(&mut commands)), 0, "unticked");
    // Newly ticked with no open session.
    let (mut app, mut commands, cfg, _dir) = fresh(false);
    saved_as(&mut app, &cfg, true);
    assert_eq!(locks(&drain(&mut commands)), 0, "no open session");
}

#[test]
fn saving_newly_read_only_connection_locks_a_compare_session_too() {
    let (mut app, mut commands, cfg, _dir) = fresh(false);
    app.state_mut().compare.connections = Some((cfg.id, uuid::Uuid::new_v4()));
    saved_as(&mut app, &cfg, true);
    assert_eq!(locks(&drain(&mut commands)), 1);
}
