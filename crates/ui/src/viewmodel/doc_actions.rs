//! Document actions from row context menus and the delete confirmation step.
use super::doc_copy::DocCopy;
use super::page_key::PageKey;
use bson::{Bson, Document};
use curlybrace_core::crud::id_of;

/// Longest `_id` preview shown in the delete confirmation, in characters.
pub const ID_PREVIEW_MAX_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq)]
pub enum DocAction {
    Edit(Document),
    /// The full document, `_id` included; the consumer strips the `_id`.
    Clone(Document),
    Delete(Bson),
    Insert,
    /// Copy documents of the shown collection to the copy clipboard (nothing is written).
    Copy(DocCopy),
}

/// An entry of the row context menu. Only Copy leaves the connection untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Edit,
    Clone,
    Copy,
    Delete,
    Insert,
}

impl MenuItem {
    pub fn label_key(self) -> &'static str {
        match self {
            MenuItem::Edit => "doc.edit",
            MenuItem::Clone => "doc.clone",
            MenuItem::Copy => "copy.menu.copy_document",
            MenuItem::Delete => "doc.delete",
            MenuItem::Insert => "doc.insert",
        }
    }
}

/// True when a query projection makes the displayed documents partial: any non-empty
/// projection (inclusion or exclusion, `_id: 0` included). `None` or `{}` returns full documents.
pub fn is_projected(projection: Option<&Document>) -> bool {
    projection.is_some_and(|p| !p.is_empty())
}

/// Entries offered for `doc`; Edit, Copy and Delete need an `_id` to address the document.
/// On a `projected` page the documents are partial copies, so Edit and Clone (which would
/// write the partial copy back) are not offered; Delete by `_id` and Insert are unaffected.
pub fn menu_items(doc: &Document, projected: bool) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let has_id = id_of(doc).is_ok();
    if has_id && !projected {
        items.push(MenuItem::Edit);
    }
    if !projected {
        items.push(MenuItem::Clone);
    }
    // Copy works from a projected page too: the clipboard keeps `_id`s, never the partial copies.
    if has_id {
        items.push(MenuItem::Copy);
        items.push(MenuItem::Delete);
    }
    items.push(MenuItem::Insert);
    items
}

/// Compact canonical extended JSON of an `_id`, truncated, for the confirmation text.
pub fn id_preview(id: &Bson) -> String {
    let text = id.clone().into_canonical_extjson().to_string();
    if text.chars().count() <= ID_PREVIEW_MAX_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(ID_PREVIEW_MAX_CHARS).collect();
    cut.push('…');
    cut
}

/// Two-step delete: a request parks the id (and the page it came from); only a
/// confirm against that same page emits the action.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DeleteConfirm {
    #[default]
    None,
    Pending {
        id: Bson,
        page: PageKey,
    },
}

impl DeleteConfirm {
    /// Asks for confirmation; documents without `_id` leave the state unchanged.
    pub fn requested(self, doc: &Document, page: PageKey) -> Self {
        match id_of(doc) {
            Ok(id) => DeleteConfirm::Pending {
                id: id.clone(),
                page,
            },
            Err(_) => self,
        }
    }

    /// The pending id, only while `current` is still the page it was requested on.
    pub fn pending_id(&self, current: &PageKey) -> Option<&Bson> {
        match self {
            DeleteConfirm::Pending { id, page } if page == current => Some(id),
            _ => None,
        }
    }

    /// Drops a confirmation that belongs to a page other than `current`.
    pub fn validated(self, current: &PageKey) -> Self {
        match self {
            DeleteConfirm::Pending { page, .. } if page != *current => DeleteConfirm::None,
            other => other,
        }
    }

    /// Emits `Delete(id)` once and clears; refuses when the page changed meanwhile.
    pub fn confirmed(self, current: &PageKey) -> (Self, Option<DocAction>) {
        match self {
            DeleteConfirm::Pending { id, page } if page == *current => {
                (DeleteConfirm::None, Some(DocAction::Delete(id)))
            }
            _ => (DeleteConfirm::None, None),
        }
    }

    pub fn cancelled(self) -> Self {
        DeleteConfirm::None
    }

    /// A new result page invalidates whatever was pending.
    pub fn results_replaced(self) -> Self {
        DeleteConfirm::None
    }
}

/// Applies a menu choice on `doc`: immediate actions are emitted, Delete only parks.
pub fn choose(
    confirm: DeleteConfirm,
    item: MenuItem,
    doc: &Document,
    page: PageKey,
) -> (DeleteConfirm, Option<DocAction>) {
    match item {
        MenuItem::Edit if id_of(doc).is_ok() => (confirm, Some(DocAction::Edit(doc.clone()))),
        MenuItem::Edit => (confirm, None),
        MenuItem::Clone => (confirm, Some(DocAction::Clone(doc.clone()))),
        MenuItem::Copy => {
            let copy = id_of(doc).ok().map(|id| DocCopy::Ids(vec![id.clone()]));
            (confirm, copy.map(DocAction::Copy))
        }
        MenuItem::Insert => (confirm, Some(DocAction::Insert)),
        MenuItem::Delete => (confirm.requested(doc, page), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewmodel::editor::EditorState;
    use bson::spec::BinarySubtype;
    use bson::{Binary, doc, oid::ObjectId};
    use curlybrace_core::test_support::mock_objects::mock_doc_with_id;

    fn key(n: usize) -> PageKey {
        PageKey {
            len: n,
            skip: 0,
            fingerprint: 7,
        }
    }

    fn pending(id: Bson) -> DeleteConfirm {
        DeleteConfirm::Pending { id, page: key(1) }
    }

    #[test]
    fn delete_request_parks_without_emitting() {
        let d = mock_doc_with_id(5_i32);
        let (state, action) = choose(DeleteConfirm::None, MenuItem::Delete, &d, key(1));
        assert_eq!(state, pending(Bson::Int32(5)));
        assert_eq!(action, None);
    }

    #[test]
    fn confirm_emits_delete_exactly_once() {
        let (state, first) = pending(Bson::Int32(5)).confirmed(&key(1));
        assert_eq!(first, Some(DocAction::Delete(Bson::Int32(5))));
        let (state, second) = state.confirmed(&key(1));
        assert_eq!((state, second), (DeleteConfirm::None, None));
    }

    #[test]
    fn confirm_is_refused_when_the_page_changed() {
        let p = pending(Bson::Int32(5));
        assert_eq!(p.pending_id(&key(1)), Some(&Bson::Int32(5)));
        assert_eq!(p.pending_id(&key(2)), None);
        assert_eq!(p.clone().validated(&key(2)), DeleteConfirm::None);
        assert_eq!(p.clone().validated(&key(1)), p);
        assert_eq!(p.confirmed(&key(2)), (DeleteConfirm::None, None));
    }

    #[test]
    fn cancel_and_new_page_clear_pending() {
        let p = pending(Bson::Int32(1));
        assert_eq!(p.clone().cancelled(), DeleteConfirm::None);
        assert_eq!(p.results_replaced(), DeleteConfirm::None);
    }

    #[test]
    fn doc_without_id_cannot_be_deleted_or_edited() {
        let d = doc! { "name": "x" };
        let items = menu_items(&d, false);
        assert!(!items.contains(&MenuItem::Delete) && !items.contains(&MenuItem::Edit));
        assert!(items.contains(&MenuItem::Clone));
        let (state, action) = choose(DeleteConfirm::None, MenuItem::Delete, &d, key(1));
        assert_eq!((state, action), (DeleteConfirm::None, None));
        assert_eq!(
            choose(DeleteConfirm::None, MenuItem::Edit, &d, key(1)).1,
            None
        );
        let (state, _) = choose(pending(Bson::Int32(9)), MenuItem::Delete, &d, key(1));
        assert_eq!(state, pending(Bson::Int32(9)), "unchanged");
    }

    #[test]
    fn clone_emits_the_full_document_even_without_id() {
        let d = doc! { "name": "x" };
        let (_, action) = choose(DeleteConfirm::None, MenuItem::Clone, &d, key(1));
        assert_eq!(action, Some(DocAction::Clone(d)));
    }

    fn id_kinds() -> Vec<Bson> {
        vec![
            Bson::ObjectId(ObjectId::new()),
            Bson::String("user-1".into()),
            Bson::Int32(7),
            Bson::Int64(1 << 40),
            Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes: (0u8..16).collect(),
            }),
            Bson::Document(doc! { "a": 1_i32, "b": "x" }),
        ]
    }

    #[test]
    fn every_id_kind_flows_through_menu_confirm_and_editor_exactly() {
        for id in id_kinds() {
            let d = doc! { "_id": id.clone(), "v": 1_i32 };
            assert_eq!(menu_items(&d, false).len(), 5, "{id:?}");
            let copied = choose(DeleteConfirm::None, MenuItem::Copy, &d, key(1)).1;
            let by_id = DocAction::Copy(DocCopy::Ids(vec![id.clone()]));
            assert_eq!(copied, Some(by_id), "{id:?}");
            let edit = choose(DeleteConfirm::None, MenuItem::Edit, &d, key(1)).1;
            assert_eq!(edit, Some(DocAction::Edit(d.clone())), "{id:?}");
            let cloned = choose(DeleteConfirm::None, MenuItem::Clone, &d, key(1)).1;
            assert_eq!(cloned, Some(DocAction::Clone(d.clone())), "{id:?}");
            let (parked, none) = choose(DeleteConfirm::None, MenuItem::Delete, &d, key(1));
            assert_eq!(none, None);
            let (after, deleted) = parked.confirmed(&key(1));
            assert_eq!(deleted, Some(DocAction::Delete(id.clone())), "{id:?}");
            assert_eq!(after.confirmed(&key(1)).1, None, "once only");
            assert_eq!(EditorState::for_edit(&d).original_id, Some(id), "editor");
        }
    }

    #[test]
    fn id_preview_shows_compact_extended_json() {
        let compound = Bson::Document(doc! { "a": 1_i32, "b": "x" });
        assert_eq!(id_preview(&compound), r#"{"a":{"$numberInt":"1"},"b":"x"}"#);
        assert_eq!(id_preview(&Bson::String("k".into())), "\"k\"");
        let oid = ObjectId::new();
        assert!(id_preview(&Bson::ObjectId(oid)).contains(&oid.to_hex()));
    }

    #[test]
    fn long_id_preview_is_truncated() {
        let long = Bson::String("é".repeat(500));
        let text = id_preview(&long);
        assert_eq!(text.chars().count(), ID_PREVIEW_MAX_CHARS + 1);
        assert!(text.ends_with('…'));
    }

    #[test]
    fn projected_pages_offer_neither_edit_nor_clone_but_keep_copy_delete_and_insert() {
        let d = doc! { "_id": 1_i32, "v": 1_i32 };
        assert_eq!(
            menu_items(&d, true),
            vec![MenuItem::Copy, MenuItem::Delete, MenuItem::Insert]
        );
        assert_eq!(
            menu_items(&d, false),
            vec![
                MenuItem::Edit,
                MenuItem::Clone,
                MenuItem::Copy,
                MenuItem::Delete,
                MenuItem::Insert
            ]
        );
    }

    #[test]
    fn any_non_empty_projection_counts_as_projected() {
        assert!(is_projected(Some(&doc! { "name": 1 })));
        assert!(is_projected(Some(&doc! { "_id": 0 })));
        assert!(is_projected(Some(&doc! { "secret": 0, "n": 0 })));
        assert!(!is_projected(Some(&doc! {})));
        assert!(!is_projected(None));
    }
}
