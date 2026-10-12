//! Document actions from row context menus and the delete confirmation step.
use bson::{Bson, Document};
use mongostudio_core::crud::id_of;

#[derive(Debug, Clone, PartialEq)]
pub enum DocAction {
    Edit(Document),
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

/// Entries offered for `doc`; Delete is withheld when the document has no `_id`.
pub fn menu_items(doc: &Document) -> Vec<MenuItem> {
    let mut items = vec![MenuItem::Edit, MenuItem::Clone];
    if id_of(doc).is_ok() {
        items.push(MenuItem::Delete);
    }
    items.push(MenuItem::Insert);
    items
}

/// Two-step delete: a request parks the id, only a confirm emits the action.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum DeleteConfirm {
    #[default]
    None,
    Pending(Bson),
}

impl DeleteConfirm {
    /// Asks for confirmation; documents without `_id` leave the state unchanged.
    pub fn requested(self, doc: &Document) -> Self {
        match id_of(doc) {
            Ok(id) => DeleteConfirm::Pending(id.clone()),
            Err(_) => self,
        }
    }

    /// Emits `Delete(id)` once and clears.
    pub fn confirmed(self) -> (Self, Option<DocAction>) {
        match self {
            DeleteConfirm::Pending(id) => (DeleteConfirm::None, Some(DocAction::Delete(id))),
            DeleteConfirm::None => (DeleteConfirm::None, None),
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
) -> (DeleteConfirm, Option<DocAction>) {
    match item {
        MenuItem::Edit => (confirm, Some(DocAction::Edit(doc.clone()))),
        MenuItem::Clone => (confirm, Some(DocAction::Clone(doc.clone()))),
        MenuItem::Insert => (confirm, Some(DocAction::Insert)),
        MenuItem::Delete => (confirm.requested(doc), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;
    use mongostudio_core::test_support::mock_objects::mock_doc_with_id;

    #[test]
    fn delete_request_parks_without_emitting() {
        let d = mock_doc_with_id(5_i32);
        let (state, action) = choose(DeleteConfirm::None, MenuItem::Delete, &d);
        assert_eq!(state, DeleteConfirm::Pending(Bson::Int32(5)));
        assert_eq!(action, None);
    }

    #[test]
    fn confirm_emits_delete_exactly_once() {
        let pending = DeleteConfirm::Pending(Bson::Int32(5));
        let (state, first) = pending.confirmed();
        assert_eq!(first, Some(DocAction::Delete(Bson::Int32(5))));
        let (state, second) = state.confirmed();
        assert_eq!((state, second), (DeleteConfirm::None, None));
    }

    #[test]
    fn cancel_and_new_page_clear_pending() {
        let pending = DeleteConfirm::Pending(Bson::Int32(1));
        assert_eq!(pending.clone().cancelled(), DeleteConfirm::None);
        assert_eq!(pending.results_replaced(), DeleteConfirm::None);
    }

    #[test]
    fn doc_without_id_cannot_be_deleted() {
        let d = doc! { "name": "x" };
        assert!(!menu_items(&d).contains(&MenuItem::Delete));
        let (state, action) = choose(DeleteConfirm::None, MenuItem::Delete, &d);
        assert_eq!((state, action), (DeleteConfirm::None, None));
        let (state, _) = choose(DeleteConfirm::Pending(Bson::Int32(9)), MenuItem::Delete, &d);
        assert_eq!(state, DeleteConfirm::Pending(Bson::Int32(9)), "unchanged");
    }

    #[test]
    fn other_items_emit_their_action() {
        let d = mock_doc_with_id(1_i32);
        assert_eq!(
            choose(DeleteConfirm::None, MenuItem::Edit, &d).1,
            Some(DocAction::Edit(d.clone()))
        );
        assert_eq!(
            choose(DeleteConfirm::None, MenuItem::Clone, &d).1,
            Some(DocAction::Clone(d.clone()))
        );
        assert_eq!(
            choose(DeleteConfirm::None, MenuItem::Insert, &d).1,
            Some(DocAction::Insert)
        );
        assert_eq!(menu_items(&d).len(), 4);
    }
}
