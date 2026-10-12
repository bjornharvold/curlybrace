//! Application state. Immutable by convention: the reducer consumes a state and returns the next.
use super::persistence::{PersistError, PersistGuard, Recovery};
use crate::viewmodel::{EditorState, FindBarState, PageKey, SidebarState, TreeState, ViewMode};
use crate::views::connections::ConnectionsState;
use bson::Bson;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Namespace, Page, Query};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

/// A dismissible message. Holds keys and typed errors, never English text.
#[derive(Clone, Debug, PartialEq)]
pub enum Banner {
    /// A runtime command (or local operation) failed; `context` is a `failures.*` suffix.
    Failure {
        context: &'static str,
        error: CoreError,
    },
    Persist(PersistError),
    /// What startup found wrong with the saved connection list, and whether it was backed up.
    Recovery(Recovery),
    /// A data action was attempted without a live connection.
    NotConnected,
    HostForgotten,
    /// Edit/Clone were refused because the page is shown with a projection (partial documents).
    ProjectionReadOnly,
}

/// An SSH endpoint whose recorded host key the user may drop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostEndpoint {
    pub host: String,
    pub port: u16,
}

impl HostEndpoint {
    pub fn of(cfg: &ConnectionConfig) -> Option<Self> {
        cfg.ssh.as_ref().map(|s| Self {
            host: s.host.clone(),
            port: s.port,
        })
    }
}

/// Identity of the editor content a mutation was saved from.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorKey {
    pub original_id: Option<Bson>,
    pub text: String,
}

/// The single mutating command in flight.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    pub ns: Namespace,
    /// Set for editor saves, so a failure only lands in the editor it came from.
    pub editor: Option<EditorKey>,
}

#[derive(Debug, Default)]
pub struct AppState {
    pub conn: ConnectionsState,
    pub status: ConnStatus,
    /// Mirrors the runtime's request counter: bumped once for EVERY Connect/Disconnect command
    /// sent (the runtime does the same, starting at 0, and the UI is its only sender). An
    /// outcome stamped with an older epoch belongs to a request the user has since replaced.
    pub latest_requested_epoch: u64,
    pub sidebar: SidebarState,
    /// Databases whose collection list is in flight.
    pub loading: BTreeSet<String>,
    pub find: FindBarState,
    /// The latest requested query; only used to recognise stale `Results`.
    pub issued_query: Query,
    /// The query that produced the page on screen (paging and re-fetch after edits use it).
    pub page_query: Query,
    /// A find is in flight and no page is shown yet.
    pub loading_page: bool,
    pub page: Option<Page>,
    /// Content key of `page`, computed once when the page arrives.
    pub page_key: Option<PageKey>,
    pub tree: TreeState,
    pub view_mode: ViewMode,
    pub editor: Option<EditorState>,
    /// The mutating command in flight; further mutations are ignored until it resolves.
    pub pending: Option<Pending>,
    pub persist: PersistGuard,
    pub banner: Option<Banner>,
    /// SSH endpoint of the connection being (or last) opened.
    pub ssh_endpoint: Option<HostEndpoint>,
    /// Endpoint awaiting the "forget host" confirmation.
    pub forget_confirm: Option<HostEndpoint>,
}

impl AppState {
    pub fn new(connections: Vec<ConnectionConfig>) -> Self {
        Self {
            conn: ConnectionsState::new(connections),
            ..Self::default()
        }
    }

    pub fn selected_ns(&self) -> Option<&Namespace> {
        self.sidebar.selected.as_ref()
    }

    pub fn connection(&self, id: Uuid) -> Option<&ConnectionConfig> {
        self.conn.connections.iter().find(|c| c.id == id)
    }

    /// The displayed page was fetched with a projection, so its documents are partial.
    pub fn page_is_projected(&self) -> bool {
        self.page.is_some() && crate::viewmodel::is_projected(self.page_query.projection.as_ref())
    }

    /// `(has_prev, has_next)` for the find bar.
    pub fn paging(&self) -> (bool, bool) {
        (
            self.page.is_some() && self.page_query.skip > 0,
            self.page.as_ref().is_some_and(|p| p.has_more),
        )
    }

    /// Drops everything tied to a live session; keeps saved connections, banner and form.
    pub fn session_cleared(self) -> Self {
        Self {
            sidebar: SidebarState::default(),
            loading: BTreeSet::new(),
            find: FindBarState::default(),
            issued_query: Query::default(),
            page_query: Query::default(),
            loading_page: false,
            page: None,
            page_key: None,
            tree: self.tree.results_replaced(),
            editor: None,
            pending: None,
            forget_confirm: None,
            ..self
        }
    }

    /// Whether the "forget host" action applies to the current banner.
    pub fn forgettable_host(&self) -> Option<&HostEndpoint> {
        match (&self.banner, &self.ssh_endpoint) {
            (
                Some(Banner::Failure {
                    context: "connect",
                    error,
                }),
                Some(endpoint),
            ) if *error == CoreError::SshHostKeyChanged => Some(endpoint),
            _ => None,
        }
    }
}
