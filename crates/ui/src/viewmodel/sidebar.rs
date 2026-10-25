//! Sidebar tree state: databases, lazily loaded collections, selection.
//! Every transition consumes the state and returns a new value.
use super::copy_clipboard::PasteTarget;
use super::sidebar_selection::{NodeRef, SelectMode};
use curlybrace_core::store::Namespace;
use std::collections::HashSet;
use uuid::Uuid;

/// One database row. `collections == None` means "not loaded yet".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbNode {
    pub name: String,
    pub expanded: bool,
    pub collections: Option<Vec<String>>,
}

impl DbNode {
    fn new(name: String) -> Self {
        Self {
            name,
            expanded: false,
            collections: None,
        }
    }
}

/// What the user asked for in the sidebar; the caller (reducer) performs it.
///
/// - `LoadCollections(db)`: mark the database expanded (`toggled`) AND fetch its collections.
/// - `Toggle(db)`: toggle expansion only, no fetch. While a load is in flight the database is
///   expanded with `collections == None`, and a click emits `Toggle` (the reducer should dedupe
///   in-flight loads and may collapse).
/// - `Open(ns)`: open the collection. It does NOT update the selection; the reducer must call
///   `SidebarState::selecting`.
/// - `Refresh`: reload the database list.
/// - `CompareTo(ns)`: start a comparison with `ns` as the source side.
/// - `CompareDatabaseTo(db)`: start a comparison with database `db` as the source side.
/// - `Select { node, mode }`: a modified click (Cmd/Ctrl or Shift) or a focus move changed the
///   selection. It never opens or expands the node. A plain click is `Open`/`Toggle`/
///   `LoadCollections`, which also make the node the single selection.
/// - `Copy`: store the selection as the copy clipboard.
/// - `Paste(target)`: paste the clipboard onto `target`.
/// - `InSession { session, action }`: `action` happened under the root of `session`, which may
///   not be the active one. Expanding keeps the view where it is; anything else focuses it first.
#[derive(Debug, Clone, PartialEq)]
pub enum SidebarAction {
    LoadCollections(String),
    Toggle(String),
    Open(Namespace),
    Refresh,
    CompareTo(Namespace),
    CompareDatabaseTo(String),
    Select {
        node: NodeRef,
        mode: SelectMode,
    },
    Copy,
    Paste(PasteTarget),
    InSession {
        session: Uuid,
        action: Box<SidebarAction>,
    },
}

/// Decision for the "Compare to..." entry of a collection row's context menu.
pub fn compare_to_action(db: &str, collection: &str) -> SidebarAction {
    SidebarAction::CompareTo(Namespace::new(db, collection))
}

/// Decision for the "Compare database to..." entry of a database header's context menu.
pub fn compare_database_to_action(db: &str) -> SidebarAction {
    SidebarAction::CompareDatabaseTo(db.to_string())
}

/// Decision for a click on a database header.
pub fn db_click_action(node: &DbNode) -> SidebarAction {
    if !node.expanded && node.collections.is_none() {
        SidebarAction::LoadCollections(node.name.clone())
    } else {
        SidebarAction::Toggle(node.name.clone())
    }
}

/// Decision for a click on a collection row; names are passed through verbatim.
pub fn collection_click_action(db: &str, collection: &str) -> SidebarAction {
    SidebarAction::Open(Namespace::new(db, collection))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarState {
    pub databases: Vec<DbNode>,
    pub selected: Option<Namespace>,
}

/// Removes duplicates, keeping the first occurrence order.
fn dedupe(names: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .into_iter()
        .filter(|n| seen.insert(n.clone()))
        .collect()
}

impl SidebarState {
    /// Replaces the database list. Nodes that still exist keep their expanded state and loaded
    /// collections; vanished ones (and a selection inside them) are dropped.
    pub fn with_databases(self, names: Vec<String>) -> Self {
        let Self {
            databases,
            selected,
        } = self;
        let mut old: std::collections::HashMap<String, DbNode> =
            databases.into_iter().map(|n| (n.name.clone(), n)).collect();
        let databases: Vec<DbNode> = dedupe(names)
            .into_iter()
            .map(|n| old.remove(&n).unwrap_or_else(|| DbNode::new(n)))
            .collect();
        let selected = selected.filter(|ns| databases.iter().any(|d| d.name == ns.db));
        Self {
            databases,
            selected,
        }
    }

    /// Stores the loaded collections of `db`; a no-op for an unknown database.
    pub fn with_collections(self, db: &str, names: Vec<String>) -> Self {
        let names = dedupe(names);
        self.map_db(db, |node| DbNode {
            collections: Some(names),
            ..node
        })
    }

    pub fn toggled(self, db: &str) -> Self {
        self.map_db(db, |node| DbNode {
            expanded: !node.expanded,
            ..node
        })
    }

    pub fn selecting(self, ns: Namespace) -> Self {
        Self {
            selected: Some(ns),
            ..self
        }
    }

    fn map_db(self, db: &str, f: impl FnOnce(DbNode) -> DbNode) -> Self {
        let mut f = Some(f);
        let databases = self
            .databases
            .into_iter()
            .map(|node| match f.take() {
                Some(f) if node.name == db => f(node),
                other => {
                    f = other;
                    node
                }
            })
            .collect();
        Self { databases, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_database_to_action_names_the_database() {
        assert_eq!(
            compare_database_to_action("my db"),
            SidebarAction::CompareDatabaseTo("my db".to_string())
        );
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn state() -> SidebarState {
        SidebarState::default().with_databases(names(&["app", "my db", "a.b.c", "данные"]))
    }

    fn node(expanded: bool, collections: Option<Vec<String>>) -> DbNode {
        DbNode {
            name: "my db.x".into(),
            expanded,
            collections,
        }
    }

    #[test]
    fn db_click_collapsed_unloaded_loads() {
        assert_eq!(
            db_click_action(&node(false, None)),
            SidebarAction::LoadCollections("my db.x".into())
        );
    }

    #[test]
    fn db_click_otherwise_toggles() {
        let toggle = SidebarAction::Toggle("my db.x".into());
        assert_eq!(db_click_action(&node(false, Some(vec![]))), toggle);
        assert_eq!(db_click_action(&node(false, Some(names(&["a"])))), toggle);
        assert_eq!(db_click_action(&node(true, Some(names(&["a"])))), toggle);
        assert_eq!(db_click_action(&node(true, Some(vec![]))), toggle);
        assert_eq!(db_click_action(&node(true, None)), toggle, "load in flight");
    }

    #[test]
    fn collection_click_opens_verbatim_names() {
        for (db, c) in [("my db", "a.b.c"), ("данные", "коллекция"), ("a.b", " x ")]
        {
            assert_eq!(
                collection_click_action(db, c),
                SidebarAction::Open(Namespace::new(db, c))
            );
        }
    }

    #[test]
    fn with_collections_on_unknown_db_is_a_noop() {
        let s = state();
        assert_eq!(s.clone().with_collections("nope", names(&["x"])), s);
    }

    #[test]
    fn toggled_twice_returns_original() {
        let s = state();
        let once = s.clone().toggled("app");
        assert!(once.databases[0].expanded);
        assert_eq!(once.toggled("app"), s);
    }

    #[test]
    fn awkward_names_are_kept_verbatim_and_selectable() {
        let s = state()
            .with_collections("my db", names(&["a.b", "коллекция", "with space"]))
            .selecting(Namespace::new("my db", "коллекция"));
        assert_eq!(s.databases[1].name, "my db");
        assert_eq!(s.databases[2].name, "a.b.c");
        assert_eq!(s.databases[3].name, "данные");
        assert_eq!(
            s.databases[1].collections,
            Some(names(&["a.b", "коллекция", "with space"]))
        );
        assert_eq!(s.selected, Some(Namespace::new("my db", "коллекция")));
    }

    #[test]
    fn refresh_keeps_state_of_surviving_databases_and_drops_vanished() {
        let s = state()
            .toggled("app")
            .with_collections("app", names(&["users"]))
            .toggled("my db")
            .selecting(Namespace::new("my db", "c"))
            .with_databases(names(&["app", "new"]));
        let app = &s.databases[0];
        assert!(app.expanded);
        assert_eq!(app.collections, Some(names(&["users"])));
        assert_eq!(s.databases[1], DbNode::new("new".into()));
        assert_eq!(s.databases.len(), 2);
        assert_eq!(s.selected, None, "selection in a vanished db is dropped");
    }

    #[test]
    fn refresh_keeps_selection_in_surviving_database() {
        let ns = Namespace::new("app", "users");
        let s = state()
            .selecting(ns.clone())
            .with_databases(names(&["app"]));
        assert_eq!(s.selected, Some(ns));
    }

    #[test]
    fn duplicates_are_deduped_keeping_first_order() {
        let s = SidebarState::default()
            .with_databases(names(&["b", "a", "b", "a", "c"]))
            .with_collections("a", names(&["x", "y", "x"]));
        let order: Vec<_> = s.databases.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(order, ["b", "a", "c"]);
        assert_eq!(s.databases[1].collections, Some(names(&["x", "y"])));
    }
}
