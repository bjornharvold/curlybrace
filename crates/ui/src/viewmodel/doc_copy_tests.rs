use super::*;
use crate::viewmodel::{
    DocAction, MenuItem, PasteContext, PasteTarget, TreeState, menu_items, page_key,
};
use bson::{Bson, doc};
use curlybrace_core::copy::protocol::{CopyRequest, CopyScope};
use curlybrace_core::store::{Namespace, Page, Query};
use uuid::Uuid;

fn page_of(ids: &[i32]) -> Page {
    Page {
        docs: ids
            .iter()
            .map(|id| doc! { "_id": *id, "name": "n" })
            .collect(),
        skip: 50,
        has_more: true,
    }
}

fn session() -> Uuid {
    Uuid::from_u128(1)
}

fn orders() -> Namespace {
    Namespace::new("shop", "orders")
}

fn query() -> Query {
    Query {
        filter: doc! { "status": "open" },
        sort: Some(doc! { "n": -1 }),
        projection: Some(doc! { "name": 1 }),
        skip: 50,
        limit: 50,
    }
}

fn paste(clip: &CopyClipboard) -> CopyRequest {
    let open = |_| true;
    let closed = |_| false;
    let names = |_, _: &str| Vec::<String>::new();
    let ctx = PasteContext {
        open: &open,
        read_only: &closed,
        existing_names: &names,
    };
    let target = PasteTarget::Collection {
        session: Uuid::from_u128(2),
        ns: Namespace::new("crm", "copies"),
    };
    clip.paste_onto(&target, &ctx).unwrap()
}

#[test]
fn selecting_rows_builds_an_ids_selection() {
    let page = page_of(&[1, 2, 3]);
    let pick = DocPick::default()
        .row_toggled(&Bson::Int32(1), &page)
        .row_toggled(&Bson::Int32(3), &page);
    assert_eq!(
        pick.copy(),
        Some(DocCopy::Ids(vec![Bson::Int32(1), Bson::Int32(3)]))
    );
    let unticked = pick.row_toggled(&Bson::Int32(1), &page);
    assert_eq!(unticked.copy(), Some(DocCopy::Ids(vec![Bson::Int32(3)])));
}

#[test]
fn an_empty_pick_copies_nothing() {
    assert_eq!(DocPick::default().copy(), None);
}

#[test]
fn select_all_result_builds_a_query_selection_not_the_visible_page() {
    let pick = DocPick::default().all_selected();
    assert_eq!(pick.copy(), Some(DocCopy::Result));
    let clip = clipboard_for(DocCopy::Result, session(), &orders(), &query()).unwrap();
    assert_eq!(
        clip,
        CopyClipboard::Documents {
            session: session(),
            ns: orders(),
            selection: DocSelection::Query {
                filter: doc! { "status": "open" },
                sort: Some(doc! { "n": -1 }),
            },
        }
    );
}

#[test]
fn unticking_a_row_after_select_all_keeps_the_rest_of_the_page() {
    let page = page_of(&[1, 2, 3]);
    let pick = DocPick::default()
        .all_selected()
        .row_toggled(&Bson::Int32(2), &page);
    assert_eq!(
        pick.copy(),
        Some(DocCopy::Ids(vec![Bson::Int32(1), Bson::Int32(3)]))
    );
}

#[test]
fn copy_is_offered_with_a_projection_unlike_edit() {
    let projected = doc! { "_id": 1_i32, "name": "n" };
    let items = menu_items(&projected, true);
    assert!(items.contains(&MenuItem::Copy));
    assert!(!items.contains(&MenuItem::Edit));
    // The clipboard keeps the filter, never the projected page: full documents are copied.
    let clip = clipboard_for(
        DocCopy::Ids(vec![Bson::Int32(1)]),
        session(),
        &orders(),
        &query(),
    )
    .unwrap();
    let CopyScope::Collections(items) = paste(&clip).scope else {
        panic!("collections scope");
    };
    assert_eq!(items[0].filter, doc! { "_id": { "$in": [1_i32] } });
}

#[test]
fn a_document_without_an_id_cannot_be_copied_by_hand() {
    let items = menu_items(&doc! { "name": "n" }, true);
    assert!(!items.contains(&MenuItem::Copy));
}

#[test]
fn over_10000_selected_rows_is_refused() {
    let many: Vec<Bson> = (0..10_001).map(Bson::Int32).collect();
    assert_eq!(
        clipboard_for(DocCopy::Ids(many), session(), &orders(), &query()),
        Err(PasteBlock::TooManyDocuments)
    );
}

#[test]
fn an_empty_selection_and_a_system_collection_are_refused() {
    assert_eq!(
        clipboard_for(DocCopy::Ids(vec![]), session(), &orders(), &query()),
        Err(PasteBlock::EmptySelection)
    );
    let system = Namespace::new("shop", "system.views");
    assert!(copy_blocked(&system));
    assert!(!copy_blocked(&orders()));
    assert_eq!(
        clipboard_for(DocCopy::Result, session(), &system, &query()),
        Err(PasteBlock::SystemSource)
    );
}

#[test]
fn a_menu_copy_of_a_picked_row_copies_the_whole_pick_else_just_that_row() {
    let page = page_of(&[1, 2, 3]);
    let key = page_key(&page);
    let mut state = TreeState::default();
    state.synced_to(&key);
    state.pick = DocPick::default()
        .row_toggled(&Bson::Int32(1), &page)
        .row_toggled(&Bson::Int32(2), &page);
    let (state, picked) = state.menu_chosen(MenuItem::Copy, &page.docs[1], &key);
    let both = vec![Bson::Int32(1), Bson::Int32(2)];
    assert_eq!(picked, Some(DocAction::Copy(DocCopy::Ids(both))));
    let (_, other) = state.menu_chosen(MenuItem::Copy, &page.docs[2], &key);
    assert_eq!(
        other,
        Some(DocAction::Copy(DocCopy::Ids(vec![Bson::Int32(3)])))
    );
}

#[test]
fn a_new_page_drops_the_pick() {
    let page = page_of(&[1, 2]);
    let mut state = TreeState::default();
    state.synced_to(&page_key(&page));
    state.pick = DocPick::default().all_selected();
    state.synced_to(&page_key(&page_of(&[7])));
    assert!(state.pick.is_empty());
    state.pick = DocPick::default().all_selected();
    assert!(state.results_replaced().pick.is_empty());
}

#[test]
fn the_menu_label_names_the_count_or_the_whole_result() {
    let page = page_of(&[1, 2]);
    let one = DocPick::default().row_toggled(&Bson::Int32(1), &page);
    assert_eq!(one.menu_label_key(), ("copy.menu.copy_document", 1));
    let two = one.row_toggled(&Bson::Int32(2), &page);
    assert_eq!(two.menu_label_key(), ("copy.menu.copy_documents", 2));
    assert_eq!(
        DocPick::default().all_selected().menu_label_key(),
        ("copy.menu.copy_query_result", 0)
    );
}
