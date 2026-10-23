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

/// `id` connected (the runtime's answer included) with a page of `app.users` on screen.
fn connected(app: &mut CurlybraceApp, id: uuid::Uuid) {
    use curlybrace_core::runtime::Event;
    use curlybrace_core::store::{Namespace, Page};
    app.dispatch(Input::Connect(id));
    let epoch = app.state().epoch_counter;
    app.dispatch(Input::Event(Event::Connected {
        session: id,
        epoch,
        topology: None,
    }));
    let state = app.state_mut();
    state.sidebar = state
        .sidebar
        .clone()
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["users".into()])
        .selecting(Namespace::new("app", "users"));
    state.page = Some(Page {
        docs: vec![bson::doc! { "_id": 1 }],
        skip: 0,
        has_more: false,
    });
}

fn delete_one() -> Input {
    Input::Doc(crate::viewmodel::DocAction::Delete(bson::Bson::Int32(1)))
}

fn writes(commands: &[Command]) -> usize {
    commands.iter().filter(|c| c.is_write()).count()
}

#[test]
fn unticking_after_a_live_lock_keeps_the_session_read_only_until_reconnect() {
    let (mut app, mut commands, cfg, _dir) = fresh(false);
    connected(&mut app, cfg.id);
    saved_as(&mut app, &cfg, true);
    assert_eq!(locks(&drain(&mut commands)), 1, "the live lock is sent");
    assert!(app.state().active_is_read_only());

    // The core stays locked until a reconnect, so the UI must too.
    saved_as(&mut app, &cfg, false);
    assert!(app.state().active_is_read_only(), "still read-only");
    assert!(app.state().active_badge().read_only, "the badge stays");
    drain(&mut commands);
    app.dispatch(delete_one());
    assert_eq!(writes(&drain(&mut commands)), 0, "the write is refused");
    assert_eq!(app.state().banner, Some(Banner::ReadOnly));

    // A reconnect opens with the unticked saved flag: writable again.
    app.dispatch(Input::Disconnect(cfg.id));
    connected(&mut app, cfg.id);
    assert!(
        !app.state().active_is_read_only(),
        "writable after reconnect"
    );
    drain(&mut commands);
    app.dispatch(delete_one());
    assert_eq!(writes(&drain(&mut commands)), 1, "the write goes out");
}

#[test]
fn a_live_lock_raises_the_parked_and_compare_snapshots() {
    let (mut app, mut commands, cfg, _dir) = fresh(false);
    let other = ConnectionConfig {
        id: uuid::Uuid::new_v4(),
        ..mock_connection()
    };
    app.state_mut().conn.connections.push(other.clone());
    connected(&mut app, cfg.id);
    connected(&mut app, other.id);
    app.state_mut().compare.connections = Some((other.id, cfg.id));
    saved_as(&mut app, &cfg, true);
    saved_as(&mut app, &cfg, false);
    drain(&mut commands);
    let state = app.state();
    assert!(
        state.session_read_only(cfg.id),
        "the parked view stays locked"
    );
    assert!(
        !state.session_read_only(other.id),
        "the other session is untouched"
    );
    assert_eq!(state.compare.connections_read_only, (false, true));
}
