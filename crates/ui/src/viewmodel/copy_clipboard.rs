//! The copy clipboard and the rules for pasting it onto a connection, database or collection.
//!
//! The clipboard holds references only: names, and for documents either the hand-picked `_id`s or
//! the filter of the query they came from. No document value is ever kept here. `paste_onto` is
//! pure: it turns a clipboard and a paste target into a `CopyRequest`, or says why it cannot.
use bson::{Bson, Document, doc};
use curlybrace_core::copy::names::{
    NameProblem, is_excluded_collection, is_excluded_database, is_same_namespace, unique_copy_name,
    validate_database_name, validate_namespace,
};
use curlybrace_core::copy::policy::ConflictPolicy;
use curlybrace_core::copy::protocol::{CopyItem, CopyRequest, CopyScope};
use curlybrace_core::runtime::SessionId;
use curlybrace_core::store::Namespace;

/// Most hand-picked `_id`s the clipboard holds; a bigger pick is copied as a query instead.
pub const MAX_CLIPBOARD_IDS: usize = 10_000;

/// Key of the Copy menu entry for one database; the count is always 1.
pub const MENU_COPY_DATABASE: &str = "copy.menu.copy_database";
/// Key of the Copy menu entry for one collection.
pub const MENU_COPY_COLLECTION: &str = "copy.menu.copy_collection";
/// Key of the Copy menu entry for several collections; the count is the number of them.
pub const MENU_COPY_COLLECTIONS: &str = "copy.menu.copy_collections";
/// Key of the Copy menu entry for one document.
pub const MENU_COPY_DOCUMENT: &str = "copy.menu.copy_document";
/// Key of the Copy menu entry for several documents; the count is the number of them.
pub const MENU_COPY_DOCUMENTS: &str = "copy.menu.copy_documents";
/// Key of the Copy menu entry for the whole query result; the count is 0 (unknown).
pub const MENU_COPY_QUERY_RESULT: &str = "copy.menu.copy_query_result";

/// Which documents of the source collection the clipboard stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum DocSelection {
    /// Hand-picked documents, by `_id` (any BSON type).
    Ids(Vec<Bson>),
    /// Every document the query matched, with the filter and sort as they were when copied.
    Query {
        filter: Document,
        sort: Option<Document>,
    },
}

impl DocSelection {
    /// A hand-picked selection; refused when empty or above [`MAX_CLIPBOARD_IDS`].
    pub fn ids(ids: Vec<Bson>) -> Result<DocSelection, PasteBlock> {
        if ids.is_empty() {
            return Err(PasteBlock::EmptySelection);
        }
        if ids.len() > MAX_CLIPBOARD_IDS {
            return Err(PasteBlock::TooManyDocuments);
        }
        Ok(DocSelection::Ids(ids))
    }

    /// The filter that selects these documents in the source collection.
    fn filter(&self) -> Document {
        match self {
            DocSelection::Ids(ids) => doc! { "_id": { "$in": ids.clone() } },
            DocSelection::Query { filter, .. } => filter.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CopyClipboard {
    Collections {
        session: SessionId,
        db: String,
        names: Vec<String>,
    },
    Database {
        session: SessionId,
        db: String,
    },
    Documents {
        session: SessionId,
        ns: Namespace,
        selection: DocSelection,
    },
}

/// Where a paste lands.
#[derive(Clone, Debug, PartialEq)]
pub enum PasteTarget {
    Connection { session: SessionId },
    Database { session: SessionId, db: String },
    Collection { session: SessionId, ns: Namespace },
}

/// What the paste rules ask about the open connections.
pub struct PasteContext<'a> {
    /// Whether the session is open.
    pub open: &'a dyn Fn(SessionId) -> bool,
    /// Whether the session is read-only.
    pub read_only: &'a dyn Fn(SessionId) -> bool,
    /// Names taken on a session: `existing_names(session, "")` lists its databases, and
    /// `existing_names(session, db)` lists the collections of `db`.
    pub existing_names: &'a dyn Fn(SessionId, &str) -> Vec<String>,
}

/// Why a clipboard cannot be pasted (or created).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasteBlock {
    /// Nothing has been copied yet.
    EmptyClipboard,
    ReadOnlyTarget,
    SourceClosed,
    TargetClosed,
    DatabaseOntoDatabase,
    DocumentsOntoDatabase,
    CollectionsOntoCollection,
    CollectionsOntoConnection,
    DatabaseOntoCollection,
    SameNamespaceDocuments,
    ExcludedDatabase,
    TooManyDocuments,
    /// No document is selected, so there is nothing to copy.
    EmptySelection,
    /// The documents come from a `system.*` collection, which is server-owned.
    SystemSource,
    /// A kept or suggested target name is not valid. The dialog lets the user edit names, so this
    /// is only the safety net.
    InvalidName(NameProblem),
}

impl CopyClipboard {
    /// A whole-database clipboard; server-owned databases cannot be copied.
    pub fn database(session: SessionId, db: impl Into<String>) -> Result<Self, PasteBlock> {
        let db = db.into();
        reject_excluded(&db)?;
        Ok(CopyClipboard::Database { session, db })
    }

    /// A clipboard of collections of one database; `system.*` collections are dropped.
    pub fn collections(
        session: SessionId,
        db: impl Into<String>,
        names: Vec<String>,
    ) -> Result<Self, PasteBlock> {
        let db = db.into();
        reject_excluded(&db)?;
        let names = names
            .into_iter()
            .filter(|n| !is_excluded_collection(n))
            .collect();
        Ok(CopyClipboard::Collections { session, db, names })
    }

    fn source_session(&self) -> SessionId {
        match self {
            CopyClipboard::Collections { session, .. }
            | CopyClipboard::Database { session, .. }
            | CopyClipboard::Documents { session, .. } => *session,
        }
    }

    /// The copy this clipboard asks for when pasted onto `target`, or why that is not possible.
    /// The first rule that applies wins: closed source, closed target, read-only target, the
    /// shape of the paste, a collection pasted onto itself, excluded databases, too many ids.
    pub fn paste_onto(
        &self,
        target: &PasteTarget,
        ctx: &PasteContext,
    ) -> Result<CopyRequest, PasteBlock> {
        let source = self.source_session();
        let target_session = target.session();
        if !(ctx.open)(source) {
            return Err(PasteBlock::SourceClosed);
        }
        if !(ctx.open)(target_session) {
            return Err(PasteBlock::TargetClosed);
        }
        if (ctx.read_only)(target_session) {
            return Err(PasteBlock::ReadOnlyTarget);
        }
        let scope = self.scope_for(target, ctx)?;
        Ok(CopyRequest {
            source,
            target: target_session,
            scope,
            policy: ConflictPolicy::default(),
            with_indexes_and_options: false,
        })
    }

    fn scope_for(&self, target: &PasteTarget, ctx: &PasteContext) -> Result<CopyScope, PasteBlock> {
        match (self, target) {
            (CopyClipboard::Database { .. }, PasteTarget::Database { .. }) => {
                Err(PasteBlock::DatabaseOntoDatabase)
            }
            (CopyClipboard::Database { .. }, PasteTarget::Collection { .. }) => {
                Err(PasteBlock::DatabaseOntoCollection)
            }
            (CopyClipboard::Documents { .. }, PasteTarget::Connection { .. })
            | (CopyClipboard::Documents { .. }, PasteTarget::Database { .. }) => {
                Err(PasteBlock::DocumentsOntoDatabase)
            }
            (CopyClipboard::Collections { .. }, PasteTarget::Collection { .. }) => {
                Err(PasteBlock::CollectionsOntoCollection)
            }
            (CopyClipboard::Collections { .. }, PasteTarget::Connection { .. }) => {
                Err(PasteBlock::CollectionsOntoConnection)
            }
            (CopyClipboard::Database { session, db }, PasteTarget::Connection { session: to }) => {
                database_scope(*session, db, *to, ctx)
            }
            (
                CopyClipboard::Collections { session, db, names },
                PasteTarget::Database {
                    session: to,
                    db: target_db,
                },
            ) => collections_scope(*session, db, names, *to, target_db, ctx),
            (
                CopyClipboard::Documents {
                    session,
                    ns,
                    selection,
                },
                PasteTarget::Collection {
                    session: to,
                    ns: target_ns,
                },
            ) => documents_scope(*session, ns, selection, *to, target_ns),
        }
    }

    /// An i18n key and a count for the Copy menu entry of this clipboard. The count is the number
    /// of collections or documents; 0 means the whole query result, whose size is not known.
    pub fn menu_label_key(&self) -> (&'static str, u64) {
        match self {
            CopyClipboard::Database { .. } => (MENU_COPY_DATABASE, 1),
            CopyClipboard::Collections { names, .. } => match names.len() {
                1 => (MENU_COPY_COLLECTION, 1),
                n => (MENU_COPY_COLLECTIONS, n as u64),
            },
            CopyClipboard::Documents { selection, .. } => match selection {
                DocSelection::Ids(ids) if ids.len() == 1 => (MENU_COPY_DOCUMENT, 1),
                DocSelection::Ids(ids) => (MENU_COPY_DOCUMENTS, ids.len() as u64),
                DocSelection::Query { .. } => (MENU_COPY_QUERY_RESULT, 0),
            },
        }
    }
}

impl PasteTarget {
    fn session(&self) -> SessionId {
        match self {
            PasteTarget::Connection { session }
            | PasteTarget::Database { session, .. }
            | PasteTarget::Collection { session, .. } => *session,
        }
    }
}

fn reject_excluded(db: &str) -> Result<(), PasteBlock> {
    if is_excluded_database(db) {
        Err(PasteBlock::ExcludedDatabase)
    } else {
        Ok(())
    }
}

fn database_scope(
    from: SessionId,
    db: &str,
    to: SessionId,
    ctx: &PasteContext,
) -> Result<CopyScope, PasteBlock> {
    reject_excluded(db)?;
    let target_db = if from == to {
        unique_copy_name(db, &(ctx.existing_names)(to, ""))
    } else {
        db.to_string()
    };
    validate_database_name(&target_db).map_err(PasteBlock::InvalidName)?;
    Ok(CopyScope::Database {
        source_db: db.to_string(),
        target_db,
    })
}

fn collections_scope(
    from: SessionId,
    db: &str,
    names: &[String],
    to: SessionId,
    target_db: &str,
    ctx: &PasteContext,
) -> Result<CopyScope, PasteBlock> {
    reject_excluded(db)?;
    reject_excluded(target_db)?;
    // Only a paste within one connection needs new names: next to the source every name does, in
    // another database only the names the target already holds. Across connections the conflict
    // policy decides, so the names are kept.
    let same_session = from == to;
    let next_to_source = same_session && db == target_db;
    let mut taken = match same_session {
        true => (ctx.existing_names)(to, target_db),
        false => Vec::new(),
    };
    let mut items = Vec::with_capacity(names.len());
    for name in names {
        let target_name = if next_to_source || (same_session && taken.contains(name)) {
            let fresh = unique_copy_name(name, &taken);
            taken.push(fresh.clone());
            fresh
        } else {
            name.clone()
        };
        let target = Namespace::new(target_db, target_name);
        validate_namespace(&target).map_err(PasteBlock::InvalidName)?;
        items.push(CopyItem {
            source: Namespace::new(db, name.clone()),
            target,
            filter: Document::new(),
        });
    }
    Ok(CopyScope::Collections(items))
}

fn documents_scope(
    from: SessionId,
    ns: &Namespace,
    selection: &DocSelection,
    to: SessionId,
    target_ns: &Namespace,
) -> Result<CopyScope, PasteBlock> {
    if from == to && is_same_namespace(ns, target_ns) {
        return Err(PasteBlock::SameNamespaceDocuments);
    }
    reject_excluded(&ns.db)?;
    reject_excluded(&target_ns.db)?;
    if is_excluded_collection(&ns.collection) {
        return Err(PasteBlock::SystemSource);
    }
    match selection {
        DocSelection::Ids(ids) if ids.is_empty() => return Err(PasteBlock::EmptySelection),
        DocSelection::Ids(ids) if ids.len() > MAX_CLIPBOARD_IDS => {
            return Err(PasteBlock::TooManyDocuments);
        }
        _ => {}
    }
    validate_namespace(target_ns).map_err(PasteBlock::InvalidName)?;
    Ok(CopyScope::Collections(vec![CopyItem {
        source: ns.clone(),
        target: target_ns.clone(),
        filter: selection.filter(),
    }]))
}

#[cfg(test)]
#[path = "copy_clipboard_tests.rs"]
mod tests;
