//! What the Copy and Paste menu entries show and do, as pure decisions over the selection, the
//! clipboard and the paste rules.
use super::copy_clipboard::{CopyClipboard, PasteBlock, PasteContext, PasteTarget};
use super::sidebar_selection::{NodeRef, SidebarSelection, clipboard_from_selection};

/// Key of the Paste menu entry.
pub const PASTE_KEY: &str = "copy.paste";
/// Why Paste is disabled while nothing was copied.
pub const EMPTY_CLIPBOARD_KEY: &str = "copy.paste_blocked.empty_clipboard";

/// The Paste entry for one target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteEntry {
    Enabled,
    /// Shown disabled; the key says why.
    Disabled(&'static str),
}

/// Why documents of a `system.*` collection cannot be copied: shared by the paste rules and by the
/// disabled Copy entries of the document views.
pub const SYSTEM_SOURCE_BLOCKED_KEY: &str = "copy.paste_blocked.system_source";

/// The i18n key that explains `block`.
pub fn paste_block_key(block: PasteBlock) -> &'static str {
    match block {
        PasteBlock::EmptyClipboard => EMPTY_CLIPBOARD_KEY,
        PasteBlock::ReadOnlyTarget => "copy.paste_blocked.read_only_target",
        PasteBlock::SourceClosed => "copy.paste_blocked.source_closed",
        PasteBlock::TargetClosed => "copy.paste_blocked.target_closed",
        PasteBlock::DatabaseOntoDatabase => "copy.paste_blocked.database_onto_database",
        PasteBlock::DocumentsOntoDatabase => "copy.paste_blocked.documents_onto_database",
        PasteBlock::CollectionsOntoCollection => "copy.paste_blocked.collections_onto_collection",
        PasteBlock::CollectionsOntoConnection => "copy.paste_blocked.collections_onto_connection",
        PasteBlock::DatabaseOntoCollection => "copy.paste_blocked.database_onto_collection",
        PasteBlock::SameNamespaceDocuments => "copy.paste_blocked.same_namespace_documents",
        PasteBlock::ExcludedDatabase => "copy.paste_blocked.excluded_database",
        PasteBlock::TooManyDocuments => "copy.paste_blocked.too_many_documents",
        PasteBlock::EmptySelection => "copy.paste_blocked.empty_selection",
        PasteBlock::SystemSource => SYSTEM_SOURCE_BLOCKED_KEY,
        PasteBlock::InvalidName(_) => "copy.paste_blocked.invalid_name",
    }
}

/// The Copy entry for `selection`: the label key and count of what it would copy, or `None`
/// (entry disabled) when nothing in it can be copied.
pub fn copy_entry(selection: &SidebarSelection) -> Option<(&'static str, u64)> {
    clipboard_from_selection(selection).map(|clipboard| clipboard.menu_label_key())
}

/// Whether `clipboard` can be pasted onto `target`, and if not, why.
pub fn paste_entry(
    clipboard: Option<&CopyClipboard>,
    target: &PasteTarget,
    ctx: &PasteContext,
) -> PasteEntry {
    let Some(clipboard) = clipboard else {
        return PasteEntry::Disabled(EMPTY_CLIPBOARD_KEY);
    };
    match clipboard.paste_onto(target, ctx) {
        Ok(_) => PasteEntry::Enabled,
        Err(block) => PasteEntry::Disabled(paste_block_key(block)),
    }
}

/// Where Cmd/Ctrl+V pastes when `node` is the focused row: onto a database; onto a collection when
/// the clipboard holds documents, else onto that collection's database.
pub fn paste_target_for(node: &NodeRef, clipboard: Option<&CopyClipboard>) -> PasteTarget {
    match (node, clipboard) {
        (NodeRef::Database { session, db }, _) => PasteTarget::Database {
            session: *session,
            db: db.clone(),
        },
        (NodeRef::Collection { session, ns }, Some(CopyClipboard::Documents { .. })) => {
            PasteTarget::Collection {
                session: *session,
                ns: ns.clone(),
            }
        }
        (NodeRef::Collection { session, ns }, _) => PasteTarget::Database {
            session: *session,
            db: ns.db.clone(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::Bson;
    use curlybrace_core::runtime::SessionId;
    use curlybrace_core::store::Namespace;

    fn sid() -> SessionId {
        SessionId::from_u128(1)
    }

    fn collection() -> NodeRef {
        NodeRef::Collection {
            session: sid(),
            ns: Namespace::new("shop", "orders"),
        }
    }

    fn database_target() -> PasteTarget {
        PasteTarget::Database {
            session: sid(),
            db: "shop".into(),
        }
    }

    #[test]
    fn a_focused_database_pastes_onto_itself() {
        let node = NodeRef::Database {
            session: sid(),
            db: "shop".into(),
        };
        assert_eq!(paste_target_for(&node, None), database_target());
    }

    #[test]
    fn a_focused_collection_pastes_onto_its_database_unless_documents_are_copied() {
        let collections = CopyClipboard::collections(sid(), "crm", vec!["c".into()]).unwrap();
        assert_eq!(
            paste_target_for(&collection(), Some(&collections)),
            database_target()
        );
        let documents = CopyClipboard::Documents {
            session: sid(),
            ns: Namespace::new("crm", "c"),
            selection: crate::viewmodel::DocSelection::Ids(vec![Bson::Int32(1)]),
        };
        assert_eq!(
            paste_target_for(&collection(), Some(&documents)),
            PasteTarget::Collection {
                session: sid(),
                ns: Namespace::new("shop", "orders"),
            }
        );
    }
}
