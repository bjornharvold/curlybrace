//! Dragging sidebar selections onto a target: what a drag carries, and that a drop takes the same
//! path as the Paste menu entry. The pointer part is egui's; this is the decision behind it.
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, ConnStatus};
use crate::viewmodel::copy_menu::paste_block_key;
use crate::viewmodel::{
    CopyClipboard, DropVerdict, NodeRef, PasteBlock, PasteTarget, SelectMode, SidebarAction,
    SidebarState,
};
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

/// `A` (active): `shop` expanded with orders, users, items; `logs` collapsed with a loaded
/// `audit`. `B` (parked): `crm` with contacts.
fn state() -> AppState {
    let mut state = AppState {
        open: vec![sid(A), sid(B)],
        active: Some(sid(A)),
        status: ConnStatus::Connected,
        sidebar: SidebarState::default()
            .with_databases(names(&["shop", "logs"]))
            .toggled("shop")
            .with_collections("shop", names(&["orders", "users", "items"]))
            .with_collections("logs", names(&["audit"])),
        ..AppState::default()
    };
    state.parked.insert(
        sid(B),
        SessionView {
            status: ConnStatus::Connected,
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

fn select(state: AppState, node: NodeRef, mode: SelectMode) -> AppState {
    let (next, _) = update(state, Input::Sidebar(SidebarAction::Select { node, mode }));
    next
}

fn orders_and_users() -> AppState {
    let s = select(state(), coll(A, "shop", "orders"), SelectMode::Single);
    select(s, coll(A, "shop", "users"), SelectMode::Toggle)
}

fn collections(db: &str, list: &[&str]) -> CopyClipboard {
    CopyClipboard::collections(sid(A), db, names(list)).unwrap()
}

fn onto_crm() -> PasteTarget {
    PasteTarget::Database {
        session: sid(B),
        db: "crm".into(),
    }
}

#[test]
fn drag_of_unselected_row_drags_only_that_row() {
    let s = orders_and_users();
    let payload = s.drag_payload(&coll(A, "shop", "items"));
    assert_eq!(payload, Some(collections("shop", &["items"])));
    let payload = s.drag_payload(&db(A, "logs"));
    assert_eq!(
        payload,
        Some(CopyClipboard::database(sid(A), "logs").unwrap())
    );
}

#[test]
fn drag_of_selected_row_drags_the_whole_selection() {
    let s = orders_and_users();
    for dragged in ["orders", "users"] {
        let payload = s.drag_payload(&coll(A, "shop", dragged));
        assert_eq!(payload, Some(collections("shop", &["orders", "users"])));
    }
}

#[test]
fn drag_carries_only_rows_that_are_visible() {
    // `audit` is selected but its database is collapsed: it was not on screen to be dragged.
    let s = select(state(), coll(A, "shop", "orders"), SelectMode::Single);
    let s = select(s, coll(A, "logs", "audit"), SelectMode::Toggle);
    assert_eq!(
        s.drag_payload(&coll(A, "shop", "orders")),
        Some(collections("shop", &["orders"]))
    );
}

#[test]
fn a_drag_that_cannot_be_copied_carries_nothing() {
    let s = state();
    assert_eq!(s.drag_payload(&db(A, "admin")), None);
    let two_databases = select(
        select(s, db(A, "shop"), SelectMode::Single),
        db(A, "logs"),
        SelectMode::Toggle,
    );
    assert_eq!(two_databases.drag_payload(&db(A, "shop")), None);
}

#[test]
fn drop_runs_the_same_paste_path_as_the_menu() {
    let payload = collections("shop", &["orders", "users"]);
    let (dropped, commands) = update(
        state(),
        Input::Sidebar(SidebarAction::Drop {
            clipboard: Box::new(payload.clone()),
            target: onto_crm(),
        }),
    );
    assert!(commands.is_empty());
    let copied = AppState {
        clipboard: Some(payload.clone()),
        ..state()
    };
    let (pasted, _) = update(copied, Input::Sidebar(SidebarAction::Paste(onto_crm())));
    assert!(dropped.pending_paste.is_some());
    assert_eq!(dropped.pending_paste, pasted.pending_paste);
    assert_eq!(dropped.banner, pasted.banner);
    // The drop is what puts the payload on the clipboard.
    assert_eq!(dropped.clipboard, Some(payload));
}

#[test]
fn a_session_tagged_drop_pastes_without_changing_the_focus() {
    let payload = CopyClipboard::database(sid(A), "shop").unwrap();
    let action = SidebarAction::InSession {
        session: sid(B),
        action: Box::new(SidebarAction::Drop {
            clipboard: Box::new(payload),
            target: PasteTarget::Connection { session: sid(B) },
        }),
    };
    let (next, _) = update(state(), Input::Sidebar(action));
    assert!(next.pending_paste.is_some());
    assert_eq!(next.active, Some(sid(A)));
}

#[test]
fn blocked_drop_reports_the_reason_and_sends_nothing() {
    let earlier = collections("shop", &["items"]);
    let before = AppState {
        clipboard: Some(earlier.clone()),
        ..state()
    };
    let payload = collections("shop", &["orders"]);
    let target = PasteTarget::Collection {
        session: sid(B),
        ns: Namespace::new("crm", "contacts"),
    };
    let key = paste_block_key(PasteBlock::CollectionsOntoCollection);
    assert_eq!(
        before.drop_verdict(&payload, &target),
        DropVerdict::Blocked(key)
    );
    let (after, commands) = update(
        before,
        Input::Sidebar(SidebarAction::Drop {
            clipboard: Box::new(payload),
            target,
        }),
    );
    assert!(commands.is_empty());
    assert_eq!(after.pending_paste, None);
    assert!(after.banner.is_none());
    assert_eq!(
        after.clipboard,
        Some(earlier),
        "a blocked drop keeps the clipboard"
    );
}

#[test]
fn a_valid_target_is_accepted() {
    let payload = collections("shop", &["orders"]);
    assert_eq!(
        state().drop_verdict(&payload, &onto_crm()),
        DropVerdict::Accept
    );
    let database = CopyClipboard::database(sid(A), "shop").unwrap();
    let connection = PasteTarget::Connection { session: sid(B) };
    assert_eq!(
        state().drop_verdict(&database, &connection),
        DropVerdict::Accept
    );
}

// ---- the verdict cache of a drag ----

use crate::viewmodel::DropVerdictCache;
use std::sync::Arc;

fn drop_cached(
    state: &AppState,
    cache: &mut DropVerdictCache,
    payload: &CopyClipboard,
    target: &PasteTarget,
) -> DropVerdict {
    state.drop_verdict_cached(cache, payload, target)
}

#[test]
fn the_cached_verdict_equals_the_computed_one_for_every_target() {
    let payload = Arc::new(collections("shop", &["orders"]));
    let targets = [
        onto_crm(),
        PasteTarget::Connection { session: sid(B) },
        PasteTarget::Collection {
            session: sid(B),
            ns: Namespace::new("crm", "contacts"),
        },
    ];
    let s = state();
    let mut cache = DropVerdictCache::new(payload.clone());
    for _ in 0..2 {
        for target in &targets {
            assert_eq!(
                drop_cached(&s, &mut cache, &payload, target),
                s.drop_verdict(&payload, target),
                "{target:?}"
            );
        }
    }
}

#[test]
fn the_cache_answers_from_memory_while_the_context_is_unchanged() {
    let payload = Arc::new(collections("shop", &["orders"]));
    let s = state();
    let key = s.drop_context_key(&payload, &onto_crm());
    let mut cache = DropVerdictCache::new(payload);
    let mut computed = 0;
    for _ in 0..3 {
        cache.verdict(&onto_crm(), key, || {
            computed += 1;
            DropVerdict::Accept
        });
    }
    assert_eq!(computed, 1);
}

#[test]
fn the_cache_recomputes_when_the_connection_status_changes() {
    let payload = Arc::new(collections("shop", &["orders"]));
    let mut cache = DropVerdictCache::new(payload.clone());
    let connected = state();
    assert_eq!(
        drop_cached(&connected, &mut cache, &payload, &onto_crm()),
        DropVerdict::Accept
    );
    let mut lost = state();
    lost.parked.get_mut(&sid(B)).unwrap().status = ConnStatus::Failed;
    let verdict = drop_cached(&lost, &mut cache, &payload, &onto_crm());
    assert_eq!(verdict, lost.drop_verdict(&payload, &onto_crm()));
    assert_ne!(verdict, DropVerdict::Accept, "the target is not open");
    // And back: the connected context is recomputed, not served stale.
    assert_eq!(
        drop_cached(&connected, &mut cache, &payload, &onto_crm()),
        DropVerdict::Accept
    );
}

#[test]
fn the_cache_recomputes_when_the_target_collections_change() {
    let payload = Arc::new(collections("shop", &["orders"]));
    let mut cache = DropVerdictCache::new(payload.clone());
    let before = state();
    drop_cached(&before, &mut cache, &payload, &onto_crm());
    let mut after = state();
    after.parked.get_mut(&sid(B)).unwrap().sidebar = SidebarState::default()
        .with_databases(names(&["crm"]))
        .toggled("crm")
        .with_collections("crm", names(&["contacts", "orders"]));
    assert_ne!(
        before.drop_context_key(&payload, &onto_crm()),
        after.drop_context_key(&payload, &onto_crm())
    );
}

#[test]
fn a_cache_belongs_to_one_drag_payload() {
    let payload = Arc::new(collections("shop", &["orders"]));
    let cache = DropVerdictCache::new(payload.clone());
    assert!(cache.is_for(&payload));
    assert!(!cache.is_for(&Arc::new(collections("shop", &["orders"]))));
}
