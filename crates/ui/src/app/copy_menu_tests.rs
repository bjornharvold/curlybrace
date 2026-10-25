//! Sidebar selection, Copy and Paste: the menu entries and the reducer behind them.
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::copy_clipboard::{
    MENU_COPY_COLLECTION, MENU_COPY_COLLECTIONS, MENU_COPY_DATABASE,
};
use crate::viewmodel::copy_menu::EMPTY_CLIPBOARD_KEY;
use crate::viewmodel::{
    CopyClipboard, NodeRef, PasteBlock, PasteEntry, PasteTarget, SelectMode, SidebarAction,
    SidebarState,
};
use curlybrace_core::copy::protocol::CopyScope;
use curlybrace_core::store::Namespace;
use uuid::Uuid;

fn sid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const A: u128 = 1;
const B: u128 = 2;

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// `A` (active): shop with orders, users, items. `B` (parked): crm with contacts.
fn state(b_read_only: bool) -> AppState {
    let mut state = AppState {
        open: vec![sid(A), sid(B)],
        active: Some(sid(A)),
        status: ConnStatus::Connected,
        sidebar: SidebarState::default()
            .with_databases(names(&["shop", "logs"]))
            .toggled("shop")
            .with_collections("shop", names(&["orders", "users", "items"])),
        ..AppState::default()
    };
    state.parked.insert(
        sid(B),
        SessionView {
            status: ConnStatus::Connected,
            read_only: b_read_only,
            sidebar: SidebarState::default()
                .with_databases(names(&["crm"]))
                .toggled("crm")
                .with_collections("crm", names(&["contacts"])),
            ..SessionView::default()
        },
    );
    state
}

fn coll(session: u128, db: &str, c: &str) -> NodeRef {
    NodeRef::Collection {
        session: sid(session),
        ns: Namespace::new(db, c),
    }
}

fn db(session: u128, name: &str) -> NodeRef {
    NodeRef::Database {
        session: sid(session),
        db: name.to_string(),
    }
}

fn act(state: AppState, action: SidebarAction) -> AppState {
    let (next, commands) = update(state, Input::Sidebar(action));
    assert!(commands.is_empty(), "{commands:?}");
    next
}

fn select(state: AppState, node: NodeRef, mode: SelectMode) -> AppState {
    act(state, SidebarAction::Select { node, mode })
}

fn orders_and_users() -> AppState {
    let s = select(state(false), coll(A, "shop", "orders"), SelectMode::Single);
    select(s, coll(A, "shop", "users"), SelectMode::Toggle)
}

#[test]
fn copy_entry_names_the_count() {
    let s = state(false);
    assert_eq!(s.copy_entry(), None, "nothing selected");
    let s = select(s, coll(A, "shop", "orders"), SelectMode::Single);
    assert_eq!(s.copy_entry(), Some((MENU_COPY_COLLECTION, 1)));
    let s = select(s, coll(A, "shop", "users"), SelectMode::Toggle);
    assert_eq!(s.copy_entry(), Some((MENU_COPY_COLLECTIONS, 2)));
    let s = select(s, db(A, "shop"), SelectMode::Single);
    assert_eq!(s.copy_entry(), Some((MENU_COPY_DATABASE, 1)));
}

#[test]
fn copy_entry_is_disabled_for_collections_of_two_databases() {
    let s = select(state(false), coll(A, "shop", "orders"), SelectMode::Single);
    let s = select(s, coll(A, "logs", "x"), SelectMode::Toggle);
    assert_eq!(s.copy_entry(), None);
}

#[test]
fn paste_entry_is_disabled_with_the_block_reason_key() {
    let clip = CopyClipboard::collections(sid(A), "shop", names(&["orders"])).unwrap();
    let s = AppState {
        clipboard: Some(clip),
        ..state(false)
    };
    let onto_collection = PasteTarget::Collection {
        session: sid(B),
        ns: Namespace::new("crm", "contacts"),
    };
    assert_eq!(
        s.paste_entry(&onto_collection),
        PasteEntry::Disabled("copy.paste_blocked.collections_onto_collection")
    );
    let onto_connection = PasteTarget::Connection { session: sid(B) };
    assert_eq!(
        s.paste_entry(&onto_connection),
        PasteEntry::Disabled("copy.paste_blocked.collections_onto_connection")
    );
    let onto_database = PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    };
    assert_eq!(s.paste_entry(&onto_database), PasteEntry::Enabled);
}

#[test]
fn paste_entry_is_disabled_while_the_clipboard_is_empty() {
    let target = PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    };
    assert_eq!(
        state(false).paste_entry(&target),
        PasteEntry::Disabled(EMPTY_CLIPBOARD_KEY)
    );
}

#[test]
fn paste_is_disabled_on_a_read_only_connection() {
    let s = act(orders_and_users(), SidebarAction::Copy);
    let s = AppState { ..s };
    let target = PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    };
    assert_eq!(s.paste_entry(&target), PasteEntry::Enabled);
    let read_only = AppState {
        clipboard: s.clipboard,
        ..state(true)
    };
    assert_eq!(
        read_only.paste_entry(&target),
        PasteEntry::Disabled("copy.paste_blocked.read_only_target")
    );
    // The reducer refuses it too, with a banner and no request.
    let next = act(read_only, SidebarAction::Paste(target));
    assert_eq!(next.pending_paste, None);
    assert_eq!(
        next.banner,
        Some(Banner::PasteBlocked(PasteBlock::ReadOnlyTarget))
    );
}

#[test]
fn copy_action_stores_a_reference_clipboard() {
    let s = act(orders_and_users(), SidebarAction::Copy);
    assert_eq!(
        s.clipboard,
        Some(CopyClipboard::Collections {
            session: sid(A),
            db: "shop".into(),
            names: names(&["orders", "users"]),
        })
    );
}

#[test]
fn copy_of_an_uncopyable_selection_keeps_the_old_clipboard() {
    let s = act(orders_and_users(), SidebarAction::Copy);
    let s = select(s, coll(A, "shop", "orders"), SelectMode::Single);
    let s = select(s, coll(A, "logs", "x"), SelectMode::Toggle);
    let before = s.clipboard.clone();
    let s = act(s, SidebarAction::Copy);
    assert_eq!(s.clipboard, before);
}

#[test]
fn paste_action_opens_the_paste_dialog_request() {
    let s = AppState {
        banner: Some(Banner::NotConnected),
        ..act(orders_and_users(), SidebarAction::Copy)
    };
    let target = PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    };
    let s = act(s, SidebarAction::Paste(target));
    let request = s.pending_paste.expect("a paste request");
    assert_eq!((request.source, request.target), (sid(A), sid(B)));
    assert!(matches!(request.scope, CopyScope::Collections(items) if items.len() == 2));
    assert_eq!(s.banner, None, "a successful paste clears the banner");
}

#[test]
fn paste_with_an_empty_clipboard_does_nothing() {
    let target = PasteTarget::Connection { session: sid(B) };
    let s = act(state(false), SidebarAction::Paste(target));
    assert_eq!((s.pending_paste, s.banner), (None, None));
}

#[test]
fn a_blocked_paste_sets_a_banner_and_stores_no_request() {
    let s = act(orders_and_users(), SidebarAction::Copy);
    let target = PasteTarget::Connection { session: sid(B) };
    let s = act(s, SidebarAction::Paste(target));
    assert_eq!(s.pending_paste, None);
    assert_eq!(
        s.banner,
        Some(Banner::PasteBlocked(PasteBlock::CollectionsOntoConnection))
    );
}

#[test]
fn modified_clicks_select_without_opening_or_expanding() {
    let s = select(state(false), coll(A, "shop", "orders"), SelectMode::Toggle);
    assert_eq!(s.selection.nodes(), [coll(A, "shop", "orders")]);
    assert_eq!(s.sidebar.selected, None, "nothing was opened");
    let s = select(s, db(A, "logs"), SelectMode::Extend);
    assert!(!s.sidebar.databases[1].expanded, "nothing was expanded");
    assert!(s.selection.is_selected(&db(A, "logs")));
}

#[test]
fn a_plain_click_selects_the_node_it_opens() {
    let s = state(false);
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: sid(A),
            action: Box::new(SidebarAction::Open(Namespace::new("shop", "items"))),
        }),
    );
    assert_eq!(s.selection.nodes(), [coll(A, "shop", "items")]);
    let (s, _) = update(
        s,
        Input::Sidebar(SidebarAction::InSession {
            session: sid(B),
            action: Box::new(SidebarAction::Toggle("crm".into())),
        }),
    );
    assert_eq!(
        s.selection.nodes(),
        [db(B, "crm")],
        "other session replaces it"
    );
    assert_eq!(s.active, Some(sid(A)), "expanding keeps the focus");
}

#[test]
fn shift_click_extends_over_the_rows_of_that_session() {
    let s = select(state(false), coll(A, "shop", "orders"), SelectMode::Single);
    let s = select(s, coll(A, "shop", "items"), SelectMode::Extend);
    assert_eq!(s.selection.nodes().len(), 3);
}

#[test]
fn closing_a_session_clears_its_selection() {
    let s = select(state(false), db(B, "crm"), SelectMode::Single);
    let (s, _) = update(s, Input::Disconnect(sid(B)));
    assert!(s.selection.is_empty());
    let s = select(state(false), db(A, "shop"), SelectMode::Single);
    let (s, _) = update(s, Input::Disconnect(sid(B)));
    assert!(!s.selection.is_empty(), "another session's close keeps it");
}

#[test]
fn closing_either_session_of_a_pending_paste_drops_it() {
    let s = act(orders_and_users(), SidebarAction::Copy);
    let target = PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    };
    let s = act(s, SidebarAction::Paste(target));
    assert!(s.pending_paste.is_some());
    let (s, _) = update(s, Input::Disconnect(sid(B)));
    assert_eq!(s.pending_paste, None);
}

#[test]
fn selecting_under_a_parked_root_does_not_change_the_active_session() {
    let s = act(
        state(false),
        SidebarAction::InSession {
            session: sid(B),
            action: Box::new(SidebarAction::Select {
                node: coll(B, "crm", "contacts"),
                mode: SelectMode::Single,
            }),
        },
    );
    assert_eq!(s.active, Some(sid(A)));
    assert_eq!(s.selection.nodes(), [coll(B, "crm", "contacts")]);
}

/// `state(false)` with the parked session `B` in `status`.
fn with_b_status(status: ConnStatus) -> AppState {
    let mut s = state(false);
    s.parked.get_mut(&sid(B)).unwrap().status = status;
    s
}

fn database_of_b() -> PasteTarget {
    PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    }
}

fn copied_from_a(s: AppState) -> AppState {
    AppState {
        clipboard: CopyClipboard::collections(sid(A), "shop", names(&["orders"])).ok(),
        ..s
    }
}

#[test]
fn a_failed_or_connecting_target_is_closed_for_pasting() {
    for status in [ConnStatus::Failed, ConnStatus::Connecting] {
        let s = copied_from_a(with_b_status(status));
        assert_eq!(
            s.paste_entry(&database_of_b()),
            PasteEntry::Disabled("copy.paste_blocked.target_closed"),
            "{status:?}"
        );
        let s = act(s, SidebarAction::Paste(database_of_b()));
        assert_eq!(s.pending_paste, None, "{status:?}");
        assert_eq!(
            s.banner,
            Some(Banner::PasteBlocked(PasteBlock::TargetClosed))
        );
    }
}

#[test]
fn a_source_that_failed_after_copy_is_closed_for_pasting() {
    let clipboard = CopyClipboard::collections(sid(B), "crm", names(&["contacts"])).ok();
    let s = AppState {
        clipboard,
        ..with_b_status(ConnStatus::Failed)
    };
    let onto_a = PasteTarget::Database {
        session: sid(A),
        db: "shop".into(),
    };
    assert_eq!(
        s.paste_entry(&onto_a),
        PasteEntry::Disabled("copy.paste_blocked.source_closed")
    );
    let s = act(s, SidebarAction::Paste(onto_a));
    assert_eq!(s.pending_paste, None);
    assert_eq!(
        s.banner,
        Some(Banner::PasteBlocked(PasteBlock::SourceClosed))
    );
}

#[test]
fn a_connected_parked_session_is_still_open_for_pasting() {
    let s = copied_from_a(with_b_status(ConnStatus::Connected));
    assert_eq!(s.paste_entry(&database_of_b()), PasteEntry::Enabled);
}
