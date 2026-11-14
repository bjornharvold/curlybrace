//! The administration dialogs in the reducer: opening, the typed confirmation, the read-only and
//! production guards, and what a success or a refusal does to the dialog and to what is open.

use super::reducer::{Input, update};
use super::state::{AppState, Banner, ConnStatus};
use super::write_guard::{WriteKind, WriteWarningAction};
use crate::viewmodel::{AdminAction, AdminKind, AdminRequest, SidebarAction};
use curlybrace_core::error::CoreError;
use curlybrace_core::runtime::{Command, EMBEDDED_SESSION, Event};
use curlybrace_core::store::Namespace;

fn orders() -> Namespace {
    Namespace::new("shop", "orders")
}

/// Connected embedded session with `shop.orders` open.
fn connected() -> AppState {
    let mut s = AppState::new(vec![]);
    s.open = vec![EMBEDDED_SESSION];
    s.active = Some(EMBEDDED_SESSION);
    s.status = ConnStatus::Connected;
    s.sidebar = s
        .sidebar
        .with_databases(vec!["shop".into()])
        .with_collections("shop", vec!["orders".into(), "items".into()])
        .toggled("shop")
        .selecting(orders());
    s
}

fn request(kind: AdminKind, collection: Option<&str>) -> Input {
    Input::Sidebar(SidebarAction::Admin(AdminRequest {
        kind,
        db: "shop".into(),
        collection: collection.map(str::to_owned),
    }))
}

fn act(s: AppState, a: AdminAction) -> (AppState, Vec<Command>) {
    update(s, Input::Sidebar(SidebarAction::AdminDialog(a)))
}

/// A drop dialog on `orders` with the exact name typed.
fn ready_to_drop(s: AppState) -> AppState {
    let (s, _) = update(s, request(AdminKind::DropCollection, Some("orders")));
    act(s, AdminAction::Typed("orders".into())).0
}

fn changed(db: &str) -> Input {
    Input::Event(Event::AdminChanged {
        session: EMBEDDED_SESSION,
        db: db.into(),
    })
}

#[test]
fn a_menu_entry_opens_its_dialog_once() {
    let (s, commands) = update(connected(), request(AdminKind::Rename, Some("orders")));
    assert!(commands.is_empty());
    let dialog = s.admin.as_ref().expect("the dialog is open");
    assert_eq!(dialog.kind, AdminKind::Rename);
    assert_eq!(dialog.session, EMBEDDED_SESSION);
    // A second request while one is open changes nothing.
    let (again, _) = update(s, request(AdminKind::DropDatabase, None));
    assert_eq!(again.admin.unwrap().kind, AdminKind::Rename);
}

#[test]
fn a_read_only_connection_never_opens_a_dialog() {
    let mut s = connected();
    s.read_only = true;
    let (s, commands) = update(s, request(AdminKind::DropCollection, Some("orders")));
    assert!(commands.is_empty());
    assert!(s.admin.is_none());
    assert!(matches!(s.banner, Some(Banner::ReadOnly)));
}

#[test]
fn nothing_opens_while_disconnected() {
    let mut s = connected();
    s.status = ConnStatus::Failed;
    let (s, _) = update(s, request(AdminKind::DropCollection, Some("orders")));
    assert!(s.admin.is_none());
    assert!(matches!(s.banner, Some(Banner::NotConnected)));
}

#[test]
fn a_drop_sends_nothing_until_the_exact_name_is_typed() {
    let (s, _) = update(
        connected(),
        request(AdminKind::DropCollection, Some("orders")),
    );
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(commands.is_empty());
    let (s, _) = act(s, AdminAction::Typed("Orders".into()));
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(commands.is_empty(), "case matters");
    assert!(s.pending.is_none());
}

#[test]
fn the_confirmed_drop_is_sent_and_the_dialog_holds_still_meanwhile() {
    let (s, commands) = act(ready_to_drop(connected()), AdminAction::Confirmed);
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        &commands[0],
        Command::DropCollection { ns, .. } if *ns == orders()
    ));
    assert!(s.pending.is_some());
    // Edits and Cancel are refused while the write is in flight.
    let (s, _) = act(s, AdminAction::Typed("x".into()));
    assert_eq!(s.admin.as_ref().unwrap().typed, "orders");
    let (s, _) = act(s, AdminAction::Cancelled);
    assert!(s.admin.is_some());
    // A second confirm does not send a second drop.
    let (_, commands) = act(s, AdminAction::Confirmed);
    assert!(commands.is_empty());
}

#[test]
fn a_connection_locked_while_the_dialog_is_open_refuses_the_send() {
    let mut s = ready_to_drop(connected());
    s.read_only = true;
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(commands.is_empty());
    assert!(matches!(s.banner, Some(Banner::ReadOnly)));
    assert!(s.pending.is_none());
}

#[test]
fn a_dialog_opened_on_another_session_does_not_send_here() {
    let mut s = ready_to_drop(connected());
    let mut dialog = *s.admin.take().unwrap();
    dialog.session = uuid::Uuid::from_u128(7);
    s.admin = Some(Box::new(dialog));
    let (_, commands) = act(s, AdminAction::Confirmed);
    assert!(commands.is_empty());
}

#[test]
fn a_production_connection_parks_the_write_for_its_warning() {
    let mut s = ready_to_drop(connected());
    s.production = true;
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(commands.is_empty(), "nothing is sent before the warning");
    let parked = s.write_warning.as_ref().expect("the warning is open");
    assert_eq!(parked.kind, WriteKind::DropCollection);
    let (sent, commands) = update(s, Input::WriteWarning(WriteWarningAction::Confirm));
    assert_eq!(commands.len(), 1);
    assert!(sent.pending.is_some());
}

#[test]
fn cancelling_the_production_warning_leaves_the_dialog_open_and_idle() {
    let mut s = ready_to_drop(connected());
    s.production = true;
    let (s, _) = act(s, AdminAction::Confirmed);
    let (s, commands) = update(s, Input::WriteWarning(WriteWarningAction::Cancel));
    assert!(commands.is_empty());
    assert!(s.admin.is_some() && s.pending.is_none() && s.write_warning.is_none());
}

#[test]
fn a_dropped_collection_that_was_open_is_no_longer_shown() {
    let (s, _) = act(ready_to_drop(connected()), AdminAction::Confirmed);
    let (s, commands) = update(s, changed("shop"));
    assert!(s.admin.is_none() && s.pending.is_none());
    assert_eq!(s.selected_ns(), None);
    assert!(s.page.is_none() && s.editor.is_none());
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::ListDatabases { .. }))
    );
}

#[test]
fn dropping_another_collection_keeps_the_open_one() {
    let (s, _) = update(
        connected(),
        request(AdminKind::DropCollection, Some("items")),
    );
    let (s, _) = act(s, AdminAction::Typed("items".into()));
    let (s, _) = act(s, AdminAction::Confirmed);
    let (s, _) = update(s, changed("shop"));
    assert_eq!(s.selected_ns(), Some(&orders()));
    assert!(s.admin.is_none());
}

#[test]
fn dropping_the_database_of_the_open_collection_clears_it() {
    let (s, _) = update(connected(), request(AdminKind::DropDatabase, None));
    let (s, _) = act(s, AdminAction::Typed("shop".into()));
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(matches!(&commands[0], Command::DropDatabase { db, .. } if db == "shop"));
    let (s, _) = update(s, changed("shop"));
    assert_eq!(s.selected_ns(), None);
}

#[test]
fn renaming_the_open_collection_clears_it_but_duplicating_does_not() {
    let (s, _) = update(connected(), request(AdminKind::Rename, Some("orders")));
    let (s, _) = act(s, AdminAction::NameEdited("orders2".into()));
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(matches!(
        &commands[0],
        Command::RenameCollection { new_name, .. } if new_name == "orders2"
    ));
    let (renamed, _) = update(s, changed("shop"));
    assert_eq!(renamed.selected_ns(), None);

    let (s, _) = update(connected(), request(AdminKind::Duplicate, Some("orders")));
    let (s, commands) = act(s, AdminAction::Confirmed);
    assert!(matches!(&commands[0], Command::DuplicateCollection { .. }));
    let (copied, _) = update(s, changed("shop"));
    assert_eq!(copied.selected_ns(), Some(&orders()));
}

#[test]
fn a_view_is_created_from_the_row_it_was_asked_on() {
    let (s, _) = update(connected(), request(AdminKind::CreateView, Some("orders")));
    let (s, _) = act(s, AdminAction::NameEdited("recent".into()));
    let (s, _) = act(s, AdminAction::PipelineEdited("[{\"$limit\": 5}]".into()));
    let (_, commands) = act(s, AdminAction::Confirmed);
    assert!(matches!(
        &commands[0],
        Command::CreateView { ns, source, pipeline_json, .. }
            if *ns == Namespace::new("shop", "recent")
                && source == "orders"
                && pipeline_json.contains("$limit")
    ));
}

#[test]
fn a_refusal_stays_in_the_dialog_so_the_name_can_be_fixed() {
    let (s, _) = update(connected(), request(AdminKind::Rename, Some("orders")));
    let (s, _) = act(s, AdminAction::NameEdited("items".into()));
    let (s, _) = act(s, AdminAction::Confirmed);
    let (s, commands) = update(
        s,
        Input::Event(Event::Failed {
            session: EMBEDDED_SESSION,
            context: "rename_collection",
            error: CoreError::NamespaceExists,
        }),
    );
    assert!(commands.is_empty());
    assert!(s.pending.is_none(), "the write is over");
    assert!(s.banner.is_none(), "the dialog shows it, not a banner");
    assert_eq!(
        s.admin.as_ref().unwrap().error,
        Some(CoreError::NamespaceExists)
    );
    // Editing clears the refusal and the write can be sent again.
    let (s, _) = act(s, AdminAction::NameEdited("items2".into()));
    assert!(s.admin.as_ref().unwrap().error.is_none());
    let (_, commands) = act(s, AdminAction::Confirmed);
    assert_eq!(commands.len(), 1);
}

#[test]
fn a_refusal_after_the_dialog_was_closed_is_a_banner() {
    let (s, _) = update(
        connected(),
        Input::Event(Event::Failed {
            session: EMBEDDED_SESSION,
            context: "drop_collection",
            error: CoreError::NotFound,
        }),
    );
    assert!(matches!(s.banner, Some(Banner::Failure { .. })));
}

#[test]
fn cancel_closes_an_idle_dialog() {
    let (s, _) = update(connected(), request(AdminKind::Rename, Some("orders")));
    let (s, _) = act(s, AdminAction::Cancelled);
    assert!(s.admin.is_none());
}

#[test]
fn a_success_with_no_dialog_of_ours_changes_nothing_but_the_listings() {
    let (s, commands) = update(connected(), changed("shop"));
    assert!(s.admin.is_none());
    assert_eq!(s.selected_ns(), Some(&orders()));
    assert!(!commands.is_empty());
}

#[test]
fn closing_a_session_drops_its_dialog() {
    let (s, _) = update(connected(), request(AdminKind::Rename, Some("orders")));
    assert!(s.admin.is_some());
    let s = s.session_dropped(EMBEDDED_SESSION);
    assert!(
        s.admin.is_none(),
        "no dialog is left that nothing can cancel"
    );
}

#[test]
fn a_dialog_left_on_another_session_does_not_block_a_new_request() {
    let (mut s, _) = update(connected(), request(AdminKind::Rename, Some("orders")));
    let mut stale = *s.admin.take().unwrap();
    stale.session = uuid::Uuid::from_u128(9);
    s.admin = Some(Box::new(stale));
    let (s, _) = update(s, request(AdminKind::Duplicate, Some("orders")));
    let dialog = s.admin.expect("the new dialog is open");
    assert_eq!(dialog.kind, AdminKind::Duplicate);
    assert_eq!(dialog.session, EMBEDDED_SESSION);
}

/// A drop in flight whose dialog belongs to some other session.
fn in_flight_for_another_session() -> AppState {
    let (mut s, _) = act(ready_to_drop(connected()), AdminAction::Confirmed);
    let mut other = *s.admin.take().unwrap();
    other.session = uuid::Uuid::from_u128(9);
    s.admin = Some(Box::new(other));
    s
}

#[test]
fn a_success_does_not_close_the_dialog_of_another_session() {
    let (done, _) = update(in_flight_for_another_session(), changed("shop"));
    assert!(done.admin.is_some());
}

#[test]
fn a_refusal_is_not_attached_to_the_dialog_of_another_session() {
    let (failed, _) = update(
        in_flight_for_another_session(),
        Input::Event(Event::Failed {
            session: EMBEDDED_SESSION,
            context: "drop_collection",
            error: CoreError::NotFound,
        }),
    );
    assert!(failed.admin.unwrap().error.is_none());
}

fn failed_with(s: AppState, context: &'static str, error: CoreError) -> (AppState, Vec<Command>) {
    update(
        s,
        Input::Event(Event::Failed {
            session: EMBEDDED_SESSION,
            context,
            error,
        }),
    )
}

fn sent_rename() -> AppState {
    let (s, _) = update(connected(), request(AdminKind::Rename, Some("orders")));
    let (s, _) = act(s, AdminAction::NameEdited("orders2".into()));
    act(s, AdminAction::Confirmed).0
}

#[test]
fn an_unknown_outcome_shows_the_error_and_refreshes_what_may_have_changed() {
    for error in [
        CoreError::WriteOutcomeUnknown,
        CoreError::Database("timeout".into()),
    ] {
        let (s, commands) = failed_with(sent_rename(), "rename_collection", error.clone());
        assert_eq!(s.admin.as_ref().unwrap().error, Some(error));
        assert!(s.pending.is_none());
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::ListDatabases { .. })),
            "the write may have applied, so the listings are read again"
        );
        assert!(
            commands
                .iter()
                .any(|c| matches!(c, Command::ListCollections { db, .. } if db == "shop"))
        );
    }
}

#[test]
fn a_plain_refusal_refreshes_nothing() {
    let (_, commands) = failed_with(
        sent_rename(),
        "rename_collection",
        CoreError::NamespaceExists,
    );
    assert!(commands.is_empty());
}

#[test]
fn a_lost_connection_closes_the_dialog_and_raises_the_banner() {
    let (s, _) = failed_with(sent_rename(), "rename_collection", CoreError::Unreachable);
    assert!(s.admin.is_none(), "nothing is left to resend into");
    assert!(s.pending.is_none());
    assert!(s.banner.is_some());
}

#[test]
fn dropping_the_open_collection_closes_its_copy_as_code_dialog_at_once() {
    use super::aggregate_actions::AggregateAction;
    use super::code_export_actions::{CodeAction, CodeSource};
    let (s, _) = update(
        connected(),
        Input::Aggregate(AggregateAction::Code(CodeAction::Open(CodeSource::Find))),
    );
    assert!(s.code_export.dialog.is_some(), "the fixture opens it");
    let (s, _) = update(s, request(AdminKind::DropDatabase, None));
    assert!(s.code_export.dialog.is_some(), "the request alone keeps it");
    let (s, _) = act(s, AdminAction::Typed("shop".into()));
    let (s, _) = act(s, AdminAction::Confirmed);
    let (s, _) = update(s, changed("shop"));
    assert_eq!(s.selected_ns(), None);
    assert_eq!(
        s.code_export.dialog, None,
        "not left on a vanished namespace"
    );
}
