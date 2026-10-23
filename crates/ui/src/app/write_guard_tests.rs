//! The production write guard: writes are parked until the user confirms them.
use super::*;
use crate::app::reducer::{Input, update};
use crate::app::state::{Banner, ConnStatus, Pending};
use crate::viewmodel::{DocAction, EditorAction};
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{Command, Event};
use curlybrace_core::store::{Namespace, Page};
use curlybrace_core::test_support::mock_objects::mock_connection;
use uuid::Uuid;

fn users() -> Namespace {
    Namespace::new("app", "users")
}

fn config(production: bool) -> ConnectionConfig {
    ConnectionConfig {
        production,
        ..mock_connection()
    }
}

/// Opens `cfg` as the active session, connected, with `app.users` selected showing one document.
fn browsing(cfg: ConnectionConfig) -> (AppState, Uuid) {
    let id = cfg.id;
    let (s, _) = update(AppState::new(vec![cfg]), Input::Connect(id));
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
        .selecting(users());
    s.page = Some(Page {
        docs: vec![doc! { "_id": 1, "who": "ann" }],
        skip: 0,
        has_more: false,
    });
    (s, id)
}

fn production() -> (AppState, Uuid) {
    browsing(config(true))
}

fn insert() -> Input {
    Input::Editor(EditorAction::Save {
        original_id: None,
        text: "{}".into(),
    })
}

fn replace() -> Input {
    Input::Editor(EditorAction::Save {
        original_id: Some(bson::Bson::Int32(1)),
        text: "{}".into(),
    })
}

fn clone_doc() -> Input {
    Input::Doc(DocAction::Clone(doc! { "_id": 1, "who": "ann" }))
}

fn delete() -> Input {
    Input::Doc(DocAction::Delete(bson::Bson::Int32(1)))
}

fn confirm() -> Input {
    Input::WriteWarning(WriteWarningAction::Confirm)
}

fn cancel() -> Input {
    Input::WriteWarning(WriteWarningAction::Cancel)
}

fn kind_of(s: &AppState) -> Option<WriteKind> {
    s.write_warning.as_ref().map(|w| w.kind)
}

#[test]
fn insert_replace_clone_on_production_park_instead_of_sending() {
    let cases = [
        (insert(), WriteKind::Insert),
        (replace(), WriteKind::Replace),
        (clone_doc(), WriteKind::Clone),
    ];
    for (input, kind) in cases {
        let (s, _) = production();
        let (s, commands) = update(s, input);
        assert!(commands.is_empty(), "{kind:?} must not be sent");
        assert!(s.pending.is_none(), "{kind:?} must not set pending");
        assert_eq!(kind_of(&s), Some(kind));
        let parked = s.write_warning.as_ref().unwrap();
        assert!(parked.command.is_write());
        assert_eq!(parked.pending.ns, users());
    }
}

#[test]
fn non_production_sends_immediately_unchanged() {
    for input in [insert(), replace(), clone_doc(), delete()] {
        let (s, _) = browsing(config(false));
        let (s, commands) = update(s, input);
        assert_eq!(commands.len(), 1);
        assert!(commands[0].is_write());
        assert!(s.pending.is_some());
        assert!(s.write_warning.is_none());
    }
}

#[test]
fn the_embedded_session_is_never_production() {
    let mut s = AppState::new(vec![config(true)]);
    s.status = ConnStatus::Connected;
    s.sidebar = s.sidebar.selecting(users());
    assert!(!s.active_is_production());
    let (s, commands) = update(s, insert());
    assert_eq!(commands.len(), 1);
    assert!(s.write_warning.is_none());
}

#[test]
fn confirm_sends_the_parked_command_once_and_sets_pending() {
    let (s, id) = production();
    let (s, _) = update(s, insert());
    let (s, commands) = update(s, confirm());
    assert_eq!(commands.len(), 1);
    assert!(
        matches!(&commands[0], Command::Insert { session, ns, .. } if *session == id && *ns == users())
    );
    assert!(s.write_warning.is_none());
    let pending = s.pending.as_ref().expect("pending set");
    assert_eq!(pending.ns, users());
    assert!(pending.editor.is_some(), "an editor save keeps its key");
    let (s, again) = update(s, confirm());
    assert!(again.is_empty());
    assert!(s.pending.is_some());
}

#[test]
fn cancel_drops_it_and_sends_nothing() {
    let (s, _) = production();
    let (s, _) = update(s, clone_doc());
    let (s, commands) = update(s, cancel());
    assert!(commands.is_empty());
    assert!(s.write_warning.is_none() && s.pending.is_none());
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.pending.is_none());
}

#[test]
fn second_write_while_warning_open_is_ignored() {
    let (s, _) = production();
    let (s, _) = update(s, insert());
    let (s, commands) = update(s, clone_doc());
    assert!(commands.is_empty());
    assert_eq!(
        kind_of(&s),
        Some(WriteKind::Insert),
        "neither replaced nor queued"
    );
    let (s, commands) = update(s, confirm());
    assert_eq!(commands.len(), 1);
    assert!(matches!(commands[0], Command::Insert { .. }));
    let (_, commands) = update(s, confirm());
    assert!(commands.is_empty(), "nothing was queued behind it");
}

#[test]
fn no_warning_when_write_would_be_ignored() {
    // Another mutation in flight.
    let (mut s, _) = production();
    s.pending = Some(Pending {
        ns: users(),
        editor: None,
    });
    let (s, commands) = update(s, insert());
    assert!(commands.is_empty() && s.write_warning.is_none());

    // No selected collection.
    let (mut s, _) = production();
    s.sidebar = s.sidebar.clone().with_databases(vec!["app".into()]);
    s.sidebar.selected = None;
    let (s, commands) = update(s, insert());
    assert!(commands.is_empty() && s.write_warning.is_none());

    // Not connected.
    let (mut s, _) = production();
    s.status = ConnStatus::Disconnected;
    let (s, commands) = update(s, insert());
    assert!(commands.is_empty() && s.write_warning.is_none());
}

#[test]
fn confirm_after_session_closed_or_switched_sends_nothing() {
    // Closed.
    let (s, id) = production();
    let (s, _) = update(s, insert());
    let (s, _) = update(s, Input::Disconnect(id));
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.pending.is_none() && s.write_warning.is_none());

    // Switched to another session.
    let (s, _) = production();
    let (s, _) = update(s, insert());
    let other = config(false);
    let other_id = other.id;
    let mut s = s;
    s.conn.connections.push(other);
    let (s, _) = update(s, Input::Connect(other_id));
    let epoch = s.epoch_counter;
    let (s, _) = update(
        s,
        Input::Event(Event::Connected {
            session: other_id,
            epoch,
            topology: None,
        }),
    );
    assert_eq!(s.active, Some(other_id));
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.write_warning.is_none());
    assert!(s.pending.is_none());
}

#[test]
fn confirm_with_a_mutation_in_flight_sends_nothing() {
    let (s, _) = production();
    let (mut s, _) = update(s, insert());
    s.pending = Some(Pending {
        ns: users(),
        editor: None,
    });
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.write_warning.is_none());
}

#[test]
fn confirmed_delete_does_not_warn_twice() {
    let (s, id) = production();
    let (s, commands) = update(s, delete());
    assert_eq!(commands.len(), 1);
    assert!(matches!(&commands[0], Command::Delete { session, .. } if *session == id));
    assert!(s.write_warning.is_none());
    assert!(s.pending.is_some());
}

#[test]
fn session_cleared_drops_a_parked_write() {
    let (s, _) = production();
    let (s, _) = update(s, insert());
    assert!(s.write_warning.is_some());
    assert!(s.session_cleared().write_warning.is_none());
}

fn reopened(s: AppState, id: Uuid) -> AppState {
    let (s, _) = update(s, Input::Connect(id));
    let epoch = s.epoch_counter;
    update(
        s,
        Input::Event(Event::Connected {
            session: id,
            epoch,
            topology: None,
        }),
    )
    .0
}

#[test]
fn a_reopened_session_does_not_revive_a_parked_write() {
    let (s, id) = production();
    let (s, _) = update(s, insert());
    let (s, _) = update(s, Input::Disconnect(id));
    assert!(s.write_warning.is_none(), "closing drops the warning");
    let s = reopened(s, id);
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.pending.is_none() && s.write_warning.is_none());
}

#[test]
fn a_reconnect_after_a_lost_session_does_not_revive_a_parked_write() {
    let (s, id) = production();
    let (s, _) = update(s, insert());
    let (s, _) = update(
        s,
        Input::Event(Event::Failed {
            session: id,
            context: "find",
            error: curlybrace_core::error::CoreError::Unreachable,
        }),
    );
    assert_eq!(s.status, ConnStatus::Failed);
    assert!(s.write_warning.is_none(), "losing the session drops it");
    let s = reopened(s, id);
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.pending.is_none());
}

#[test]
fn the_epoch_alone_rejects_a_parked_write_from_an_earlier_opening() {
    let (s, _) = production();
    let (mut s, _) = update(s, insert());
    s.latest_requested_epoch += 1;
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.pending.is_none());
}

#[test]
fn closing_one_session_clears_its_warning_and_another_session_writes_normally() {
    let a = config(true);
    let b = config(false);
    let (a_id, b_id) = (a.id, b.id);
    let (s, _) = browsing(a);
    let mut s = s;
    s.conn.connections.push(b);
    let (s, _) = update(s, insert());
    assert!(s.write_warning.is_some());
    let s = reopened(s, b_id);
    // B is active now with its own selection and page.
    let mut s = s;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["app".into()])
        .with_collections("app", vec!["users".into()])
        .selecting(users());
    let (s, _) = update(s, Input::Disconnect(a_id));
    assert!(s.write_warning.is_none());
    let (s, commands) = update(s, insert());
    assert_eq!(commands.len(), 1, "B is not blocked by A's old warning");
    assert!(s.pending.is_some());
}

#[test]
fn confirm_still_sends_when_the_production_flag_is_edited_off_meanwhile() {
    let (s, id) = production();
    let (mut s, _) = update(s, insert());
    for c in s.conn.connections.iter_mut().filter(|c| c.id == id) {
        c.production = false;
    }
    let (s, commands) = update(s, confirm());
    assert_eq!(commands.len(), 1, "the user explicitly confirmed it");
    assert!(s.pending.is_some());
}

fn not_sent(s: &AppState) -> bool {
    matches!(s.banner, Some(Banner::WriteNotSent))
}

#[test]
fn a_stale_confirm_says_the_write_was_not_sent() {
    // A mutation is in flight.
    let (s, _) = production();
    let (mut s, _) = update(s, insert());
    s.pending = Some(Pending {
        ns: users(),
        editor: None,
    });
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && not_sent(&s));

    // The epoch moved on.
    let (s, _) = production();
    let (mut s, _) = update(s, insert());
    s.latest_requested_epoch += 1;
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && not_sent(&s));

    // Another session is active.
    let (s, _) = production();
    let (s, _) = update(s, insert());
    let other = config(false);
    let other_id = other.id;
    let mut s = s;
    s.conn.connections.push(other);
    let s = reopened(s, other_id);
    assert_eq!(s.active, Some(other_id));
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && not_sent(&s));

    // Not connected any more.
    let (s, _) = production();
    let (mut s, _) = update(s, insert());
    s.status = ConnStatus::Disconnected;
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && not_sent(&s));
}

#[test]
fn a_valid_confirm_and_a_cancel_raise_no_banner() {
    let (s, _) = production();
    let (s, _) = update(s, insert());
    let (s, commands) = update(s, confirm());
    assert_eq!(commands.len(), 1);
    assert!(s.banner.is_none());
    let (s, _) = production();
    let (s, _) = update(s, insert());
    let (s, _) = update(s, cancel());
    assert!(s.banner.is_none());
}

#[test]
fn a_closed_or_lost_session_has_nothing_to_confirm_so_no_banner() {
    let (s, id) = production();
    let (s, _) = update(s, insert());
    let (s, _) = update(s, Input::Disconnect(id));
    let (s, commands) = update(s, confirm());
    assert!(commands.is_empty() && s.banner.is_none());
}
