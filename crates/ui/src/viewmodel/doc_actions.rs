//! Document actions from row context menus and the delete confirmation step.
use super::page_key::PageKey;
use bson::{Bson, Document};
use mongostudio_core::crud::id_of;

/// Longest `_id` preview shown in the delete confirmation, in characters.
pub const ID_PREVIEW_MAX_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq)]
pub enum DocAction {
    Edit(Document),
    /// The full document, `_id` included; the consumer strips the `_id`.
    Clone(Document),
    Delete(Bson),
    Insert,
}

/// An entry of the row context menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItem {
    Edit,
    Clone,
    Delete,
    Insert,
}

impl MenuItem {
    pub fn label_key(self) -> &'static str {
        match self {
            MenuItem::Edit => "doc.edit",
            MenuItem::Clone => "doc.clone",
            MenuItem::Delete => "doc.delete",
            MenuItem::Insert => "doc.insert",
        }
    }
}

/// Entries offered for `doc`; Edit and Delete need an `_id` to address the document.
pub fn menu_items(doc: &Document) -> Vec<MenuItem> {
    let mut items = Vec::new();
    let has_id = id_of(doc).is_ok();
    if has_id {
        items.push(MenuItem::Edit);
    }
    items.push(MenuItem::Clone);
    if has_id {
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
    use mongostudio_core::test_support::mock_objects::mock_doc_with_id;

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
        let items = menu_items(&d);
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
            assert_eq!(menu_items(&d).len(), 4, "{id:?}");
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
}
