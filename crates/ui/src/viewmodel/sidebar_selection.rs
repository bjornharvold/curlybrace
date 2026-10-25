//! The sidebar's multi-selection: databases and collections of ONE session, plus the pure rule
//! that turns a selection into a copy clipboard. Every transition consumes the selection.
use super::copy_clipboard::CopyClipboard;
use super::sidebar::SidebarState;
use curlybrace_core::runtime::SessionId;
use curlybrace_core::store::Namespace;

/// A selectable sidebar row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeRef {
    Database { session: SessionId, db: String },
    Collection { session: SessionId, ns: Namespace },
}

impl NodeRef {
    pub fn session(&self) -> SessionId {
        match self {
            NodeRef::Database { session, .. } | NodeRef::Collection { session, .. } => *session,
        }
    }
}

/// How a click changes the selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Plain click: this node alone.
    Single,
    /// Cmd/Ctrl-click: add or remove this node.
    Toggle,
    /// Shift-click: the range from the anchor to this node, in visible order.
    Extend,
}

/// The selected nodes (in selection order) and the anchor a shift-click extends from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarSelection {
    nodes: Vec<NodeRef>,
    anchor: Option<NodeRef>,
}

impl SidebarSelection {
    /// Selects `node` alone.
    pub fn single(self, node: NodeRef) -> Self {
        let _ = self;
        Self {
            nodes: vec![node.clone()],
            anchor: Some(node),
        }
    }

    /// Adds `node`, or removes it when selected. A node of another session starts a new selection.
    pub fn toggled(self, node: NodeRef) -> Self {
        if self.session().is_some_and(|s| s != node.session()) {
            return Self::default().single(node);
        }
        let nodes = match self.nodes.iter().position(|n| *n == node) {
            Some(at) => without_index(self.nodes, at),
            None => [self.nodes, vec![node.clone()]].concat(),
        };
        Self {
            nodes,
            anchor: Some(node),
        }
    }

    /// Selects the range between the anchor and `node` in `visible` order. Without an anchor in
    /// the same session and in `visible`, it selects `node` alone.
    pub fn extended_to(self, node: NodeRef, visible: &[NodeRef]) -> Self {
        let same_session = self.session().is_none_or(|s| s == node.session());
        let range = self
            .anchor
            .as_ref()
            .filter(|_| same_session)
            .and_then(|anchor| {
                let from = visible.iter().position(|n| n == anchor)?;
                let to = visible.iter().position(|n| *n == node)?;
                Some(visible[from.min(to)..=from.max(to)].to_vec())
            });
        match range {
            Some(nodes) => Self {
                nodes,
                anchor: self.anchor,
            },
            None => self.single(node),
        }
    }

    /// Applies a click of `mode`.
    pub fn select(self, node: NodeRef, mode: SelectMode, visible: &[NodeRef]) -> Self {
        match mode {
            SelectMode::Single => self.single(node),
            SelectMode::Toggle => self.toggled(node),
            SelectMode::Extend => self.extended_to(node, visible),
        }
    }

    pub fn nodes(&self) -> &[NodeRef] {
        &self.nodes
    }

    pub fn is_selected(&self, node: &NodeRef) -> bool {
        self.nodes.contains(node)
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The session the selection belongs to; none while empty.
    pub fn session(&self) -> Option<SessionId> {
        self.nodes.first().map(NodeRef::session)
    }

    /// Nothing selected.
    pub fn clear(self) -> Self {
        Self::default()
    }

    /// The selection without `session` (cleared when it belongs to it).
    pub fn without_session(self, session: SessionId) -> Self {
        if self.session() == Some(session) {
            Self::default()
        } else {
            self
        }
    }
}

/// The rows of `state` in display order for `session`: each database, then its collections when
/// expanded and loaded. This is the order a shift-click range runs in.
pub fn visible_nodes(session: SessionId, state: &SidebarState) -> Vec<NodeRef> {
    let mut out = Vec::new();
    for node in &state.databases {
        let db = node.name.clone();
        out.push(NodeRef::Database {
            session,
            db: db.clone(),
        });
        let loaded = node.collections.iter().flatten().filter(|_| node.expanded);
        out.extend(loaded.map(|c| NodeRef::Collection {
            session,
            ns: Namespace::new(&db, c),
        }));
    }
    out
}

/// The clipboard a Copy of `selection` stores, or `None` when nothing in it can be copied.
///
/// Collections that sit in a selected database are covered by it. What is left decides:
/// - one database and nothing else: that database;
/// - collections of ONE database (with or without other databases selected): those collections;
/// - collections of several databases, or several databases and no collection: not copyable.
///
/// Server-owned databases and `system.*` collections are dropped by the clipboard constructors.
pub fn clipboard_from_selection(selection: &SidebarSelection) -> Option<CopyClipboard> {
    let session = selection.session()?;
    let databases: Vec<&str> = selection
        .nodes()
        .iter()
        .filter_map(|n| match n {
            NodeRef::Database { db, .. } => Some(db.as_str()),
            NodeRef::Collection { .. } => None,
        })
        .collect();
    let uncovered: Vec<&Namespace> = selection
        .nodes()
        .iter()
        .filter_map(|n| match n {
            NodeRef::Collection { ns, .. } if !databases.contains(&ns.db.as_str()) => Some(ns),
            _ => None,
        })
        .collect();
    match (databases.as_slice(), uncovered.as_slice()) {
        ([db], []) => CopyClipboard::database(session, *db).ok(),
        (_, []) => None,
        (_, [first, ..]) if uncovered.iter().all(|ns| ns.db == first.db) => {
            let names = uncovered.iter().map(|ns| ns.collection.clone()).collect();
            let clipboard = CopyClipboard::collections(session, first.db.clone(), names).ok()?;
            Some(clipboard).filter(|c| c.menu_label_key().1 > 0)
        }
        _ => None,
    }
}

fn without_index(mut nodes: Vec<NodeRef>, at: usize) -> Vec<NodeRef> {
    nodes.remove(at);
    nodes
}

#[cfg(test)]
#[path = "sidebar_selection_tests.rs"]
mod tests;
