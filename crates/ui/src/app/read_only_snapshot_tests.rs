//! An open session counts as read-only when the flag it opened with OR the saved connection's
//! live flag says so (ticking locks at once, unticking waits for a reconnect); writes on such a
//! session are refused with a banner and no command, ahead of the production warning.
use super::reducer::{Input, update};
use super::state::{AppState, Banner};
use crate::I18n;
use crate::app::banner::banner_text;
use crate::viewmodel::{DocAction, EditorAction};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::{Namespace, Page};
use curlybrace_core::test_support::mock_objects::mock_connection;
use uuid::Uuid;

fn config(name: &str, read_only: bool, production: bool) -> ConnectionConfig {
    ConnectionConfig {
        name: name.into(),
        read_only,
        production,
        id: Uuid::new_v4(),
        ..mock_connection()
    }
}

fn connected(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    let (mut s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    );
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["users".into()])
        .selecting(Namespace::new("app", "users"));
    s.page = Some(Page {
        docs: vec![doc! { "_id": 1 }],
        skip: 0,
        has_more: false,
    });
    s
}

fn set_read_only(state: &mut AppState, id: Uuid, read_only: bool) {
    for c in state.conn.connections.iter_mut().filter(|c| c.id == id) {
        c.read_only = read_only;
    }
}

#[test]
fn ticking_saved_flag_locks_open_session_at_once() {
    let cfg = config("plain", false, false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    assert!(!s.active_is_read_only());
    set_read_only(&mut s, id, true);
    assert!(s.active_is_read_only());
    assert!(s.session_read_only(id));
}

#[test]
fn unticking_keeps_snapshot_until_reconnect() {
    let cfg = config("ro", true, false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    set_read_only(&mut s, id, false);
    assert!(s.active_is_read_only(), "the snapshot still says read-only");
    let (s, _) = update(s, Input::Disconnect(id));
    let s = connected(s, id);
    assert!(
        !s.active_is_read_only(),
        "a reconnect picks up the new flag"
    );
}

#[test]
fn deleted_connection_keeps_snapshot_read_only() {
    let cfg = config("ro", true, false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    s.conn.connections.clear();
    assert!(s.active_is_read_only());
    assert!(s.session_read_only(id));
    let cfg = config("plain", false, false);
    let id = cfg.id;
    let mut s = connected(AppState::new(vec![cfg]), id);
    s.conn.connections.clear();
    assert!(!s.active_is_read_only());
}

#[test]
fn parked_session_keeps_its_read_only_snapshot() {
    let (a, b) = (config("a", true, false), config("b", false, false));
    let (ida, idb) = (a.id, b.id);
    let s = connected(AppState::new(vec![a, b]), ida);
    let mut s = connected(s, idb);
    set_read_only(&mut s, ida, false);
    assert!(s.session_read_only(ida), "parked A keeps its snapshot");
    assert!(!s.session_read_only(idb));
    let (s, _) = update(s, Input::Activate(ida));
    assert!(s.active_is_read_only());
    assert!(!s.session_read_only(idb), "parked B was never read-only");
    let (s, _) = update(s, Input::Activate(idb));
    assert!(!s.active_is_read_only());
    assert!(s.session_read_only(ida));
}

#[test]
fn the_embedded_session_is_never_read_only() {
    let s = AppState::new(vec![config("ro", true, false)]);
    assert!(!s.active_is_read_only());
    assert!(!s.session_read_only(Uuid::nil()));
}

fn write_inputs() -> Vec<(&'static str, Input)> {
    vec![
        (
            "insert",
            Input::Editor(EditorAction::Save {
                original_id: None,
                text: "{}".into(),
            }),
        ),
        (
            "edit save",
            Input::Editor(EditorAction::Save {
                original_id: Some(bson::Bson::Int32(1)),
                text: "{}".into(),
            }),
        ),
        ("clone", Input::Doc(DocAction::Clone(doc! { "_id": 1 }))),
        (
            "delete",
            Input::Doc(DocAction::Delete(bson::Bson::Int32(1))),
        ),
    ]
}

#[test]
fn insert_edit_clone_delete_are_refused_with_banner_and_send_no_command() {
    for production in [false, true] {
        for (name, input) in write_inputs() {
            let cfg = config("ro", true, production);
            let id = cfg.id;
            let s = connected(AppState::new(vec![cfg]), id);
            let (s, commands) = update(s, input);
            assert!(commands.is_empty(), "{name} (production {production})");
            assert_eq!(s.banner, Some(Banner::ReadOnly), "{name}");
            assert!(s.write_warning.is_none(), "no confirm modal: {name}");
            assert!(s.pending.is_none(), "{name}");
            let (head, _) = banner_text(&Banner::ReadOnly, &I18n::english());
            assert!(!head.contains('⟦'));
        }
    }
}

#[test]
fn a_writable_connection_still_writes() {
    let cfg = config("plain", false, false);
    let id = cfg.id;
    let s = connected(AppState::new(vec![cfg]), id);
    let (_, commands) = update(s, write_inputs().remove(0).1);
    assert_eq!(commands.len(), 1);
}

#[test]
fn read_only_failure_from_core_shows_localized_banner() {
    let cfg = config("ro", true, false);
    let id = cfg.id;
    let s = connected(AppState::new(vec![cfg]), id);
    let (s, _) = update(
        s,
        Input::Event(Event::Failed {
            session: id,
            context: "insert",
            error: CoreError::ReadOnly,
        }),
    );
    let banner = s.banner.expect("a banner");
    let (head, detail) = banner_text(&banner, &I18n::english());
    assert!(!head.contains('⟦'), "{head}");
    assert_eq!(
        detail.as_deref(),
        Some("This connection is read-only, so nothing was changed.")
    );
}
