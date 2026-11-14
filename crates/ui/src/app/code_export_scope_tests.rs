//! The Copy as code dialog belongs to the session and collection it was opened for: it closes when
//! another session comes into view, the collection changes, the connection is lost or closed, or
//! the connection manager is shown, and stays open for everything else.
use super::{CodeAction, CodeSource};
use crate::app::AggregateAction;
use crate::app::reducer::{Input, Out, update};
use crate::app::state::AppState;
use crate::viewmodel::{CollectionTab, SidebarAction};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::Event;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn users() -> Namespace {
    Namespace::new("app", "users")
}

fn orders() -> Namespace {
    Namespace::new("app", "orders")
}

fn app(names: &[&str]) -> (AppState, Vec<Uuid>) {
    let cfgs: Vec<_> = names
        .iter()
        .map(|n| ConnectionConfig::new(n, "mongodb://localhost"))
        .collect();
    let ids = cfgs.iter().map(|c| c.id).collect();
    (AppState::new(cfgs), ids)
}

fn ev(state: AppState, event: Event) -> Out {
    update(state, Input::Event(event))
}

/// Connects `id` and answers the Connect; it ends up active with `users` selected.
fn opened(state: AppState, id: Uuid) -> AppState {
    let (s, _) = update(state, Input::Connect(id));
    let epoch = s.epoch_counter;
    let connected = Event::Connected {
        session: id,
        epoch,
        topology: None,
    };
    let s = ev(s, connected).0;
    update(s, Input::Sidebar(SidebarAction::Open(users()))).0
}

/// The dialog opened for the query bar of the session in view.
fn with_dialog(state: AppState) -> AppState {
    let (s, _) = update(
        state,
        Input::Aggregate(AggregateAction::Code(CodeAction::Open(CodeSource::Find))),
    );
    assert!(s.code_export.dialog.is_some(), "the fixture opens it");
    s
}

fn two_sessions() -> (AppState, Uuid, Uuid) {
    let (s, ids) = app(&["a", "b"]);
    let (a, b) = (ids[0], ids[1]);
    let s = opened(opened(s, a), b);
    (s, a, b)
}

#[test]
fn the_dialog_remembers_the_session_it_was_opened_in() {
    let (s, _, b) = two_sessions();
    let s = with_dialog(s);
    let dialog = s.code_export.dialog.as_ref().unwrap();
    assert_eq!(dialog.session, b);
    assert_eq!(dialog.namespace(), &users());
}

#[test]
fn another_collection_closes_it() {
    let (s, _, _) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Open(orders())));
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn another_session_in_view_closes_it_even_on_the_same_collection() {
    let (s, a, _) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(s, Input::Activate(a));
    assert_eq!(s.selected_ns(), Some(&users()), "the same collection name");
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn closing_the_session_closes_it() {
    let (s, _, b) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(s, Input::Disconnect(b));
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn losing_the_connection_closes_it() {
    let (s, _, b) = two_sessions();
    let s = with_dialog(s);
    let lost = Event::Failed {
        session: b,
        context: "find",
        error: CoreError::Unreachable,
    };
    let (s, _) = ev(s, lost);
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn showing_the_connection_manager_closes_it() {
    let (s, _, _) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(s, Input::ShowConnections);
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn it_does_not_come_back_with_the_old_session() {
    let (s, a, b) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(s, Input::Activate(a));
    let (s, _) = update(s, Input::Activate(b));
    assert_eq!(s.code_export.dialog, None);
}

#[test]
fn staying_on_the_session_and_collection_keeps_it_open() {
    let (s, _, _) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(s, Input::Tab(CollectionTab::Aggregate));
    assert!(s.code_export.dialog.is_some());
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::Code(CodeAction::SetSetup(false))),
    );
    assert!(s.code_export.dialog.is_some());
}

#[test]
fn the_remembered_language_and_options_outlive_a_closed_dialog() {
    let (s, a, _) = two_sessions();
    let s = with_dialog(s);
    let (s, _) = update(
        s,
        Input::Aggregate(AggregateAction::Code(CodeAction::SetSetup(false))),
    );
    let (s, _) = update(s, Input::Activate(a));
    assert_eq!(s.code_export.dialog, None);
    assert!(!s.code_export.last.1.setup, "the options are remembered");
}
