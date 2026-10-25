//! Copying documents from the documents pane and pasting them onto a collection in the sidebar.
use super::reducer::{Input, update};
use super::session::SessionView;
use super::state::{AppState, Banner, ConnStatus};
use crate::viewmodel::{
    CopyClipboard, DocAction, DocCopy, DocSelection, PasteBlock, PasteEntry, PasteTarget,
    SidebarAction, SidebarState,
};
use bson::{Bson, doc};
use curlybrace_core::copy::protocol::CopyScope;
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

fn sid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

const A: u128 = 1;
const B: u128 = 2;

fn open_collection(collection: &str, query: Query) -> AppState {
    let sidebar = SidebarState {
        selected: Some(Namespace::new("shop", collection)),
        ..SidebarState::default()
    };
    let mut state = AppState {
        open: vec![sid(A), sid(B)],
        active: Some(sid(A)),
        status: ConnStatus::Connected,
        sidebar,
        page: Some(Page {
            docs: vec![doc! { "_id": 1_i32 }, doc! { "_id": 2_i32 }],
            skip: query.skip,
            has_more: true,
        }),
        page_query: query,
        ..AppState::default()
    };
    state.parked.insert(
        sid(B),
        SessionView {
            status: ConnStatus::Connected,
            ..SessionView::default()
        },
    );
    state
}

fn copy(state: AppState, what: DocCopy) -> AppState {
    let (next, commands) = update(state, Input::Doc(DocAction::Copy(what)));
    assert!(commands.is_empty(), "{commands:?}");
    next
}

fn onto(session: u128, db: &str, collection: &str) -> PasteTarget {
    PasteTarget::Collection {
        session: sid(session),
        ns: Namespace::new(db, collection),
    }
}

#[test]
fn copying_rows_stores_the_ids_and_the_source() {
    let s = copy(
        open_collection("orders", Query::default()),
        DocCopy::Ids(vec![Bson::Int32(2)]),
    );
    assert_eq!(
        s.clipboard,
        Some(CopyClipboard::Documents {
            session: sid(A),
            ns: Namespace::new("shop", "orders"),
            selection: DocSelection::Ids(vec![Bson::Int32(2)]),
        })
    );
    assert_eq!(s.copy_entry(), None, "the sidebar selection is unrelated");
}

#[test]
fn copying_the_result_stores_filter_and_sort_of_the_shown_query_not_the_page() {
    let query = Query {
        filter: doc! { "status": "open" },
        sort: Some(doc! { "n": 1 }),
        projection: Some(doc! { "n": 1 }),
        skip: 50,
        limit: 50,
    };
    let s = copy(open_collection("orders", query), DocCopy::Result);
    assert_eq!(
        s.clipboard,
        Some(CopyClipboard::Documents {
            session: sid(A),
            ns: Namespace::new("shop", "orders"),
            selection: DocSelection::Query {
                filter: doc! { "status": "open" },
                sort: Some(doc! { "n": 1 }),
            },
        })
    );
}

#[test]
fn documents_paste_uses_the_collection_dialog_and_filter() {
    let s = copy(
        open_collection("orders", Query::default()),
        DocCopy::Ids(vec![Bson::Int32(1), Bson::Int32(2)]),
    );
    let target = onto(B, "crm", "archive");
    assert_eq!(s.paste_entry(&target), PasteEntry::Enabled);
    let (s, commands) = update(s, Input::Sidebar(SidebarAction::Paste(target)));
    assert!(commands.is_empty(), "nothing is sent before the dialog");
    let request = s.pending_paste.expect("the paste dialog request");
    assert_eq!((request.source, request.target), (sid(A), sid(B)));
    let CopyScope::Collections(items) = request.scope else {
        panic!("a collections scope");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source, Namespace::new("shop", "orders"));
    assert_eq!(items[0].target, Namespace::new("crm", "archive"));
    assert_eq!(items[0].filter, doc! { "_id": { "$in": [1_i32, 2_i32] } });
}

#[test]
fn paste_documents_onto_own_collection_is_disabled() {
    let s = copy(
        open_collection("orders", Query::default()),
        DocCopy::Ids(vec![Bson::Int32(1)]),
    );
    let own = onto(A, "shop", "orders");
    assert_eq!(
        s.paste_entry(&own),
        PasteEntry::Disabled("copy.paste_blocked.same_namespace_documents")
    );
    let (s, _) = update(s, Input::Sidebar(SidebarAction::Paste(own)));
    assert_eq!(s.pending_paste, None);
    assert_eq!(
        s.banner,
        Some(Banner::PasteBlocked(PasteBlock::SameNamespaceDocuments))
    );
}

#[test]
fn a_refused_copy_keeps_the_old_clipboard_and_says_why() {
    let before = copy(
        open_collection("orders", Query::default()),
        DocCopy::Ids(vec![Bson::Int32(1)]),
    );
    let many: Vec<Bson> = (0..10_001).map(Bson::Int32).collect();
    for (what, block) in [
        (DocCopy::Ids(many), PasteBlock::TooManyDocuments),
        (DocCopy::Ids(vec![]), PasteBlock::EmptySelection),
    ] {
        let s = copy(
            AppState {
                banner: None,
                ..clone_state(&before)
            },
            what,
        );
        assert_eq!(s.clipboard, before.clipboard);
        assert_eq!(s.banner, Some(Banner::PasteBlocked(block)));
    }
}

#[test]
fn documents_of_a_system_collection_are_not_copied() {
    let s = copy(
        open_collection("system.views", Query::default()),
        DocCopy::Result,
    );
    assert_eq!(s.clipboard, None);
    assert_eq!(
        s.banner,
        Some(Banner::PasteBlocked(PasteBlock::SystemSource))
    );
}

#[test]
fn nothing_is_copied_without_a_shown_collection() {
    let s = copy(AppState::default(), DocCopy::Result);
    assert_eq!((s.clipboard, s.banner), (None, None));
}

fn clone_state(state: &AppState) -> AppState {
    AppState {
        open: state.open.clone(),
        active: state.active,
        status: state.status,
        sidebar: state.sidebar.clone(),
        page: state.page.clone(),
        page_query: state.page_query.clone(),
        clipboard: state.clipboard.clone(),
        ..AppState::default()
    }
}
