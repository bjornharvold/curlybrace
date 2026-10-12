//! Application state. Immutable by convention: the reducer consumes a state and returns the next.
use super::persistence::PersistError;
use crate::viewmodel::{EditorState, FindBarState, PageKey, SidebarState, TreeState, ViewMode};
use crate::views::connections::ConnectionsState;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::error::CoreError;
use mongostudio_core::store::{Namespace, Page, Query};
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
    /// Entries of the saved connection list that were invalid and skipped on load.
    SkippedEntries(usize),
    /// A data action was attempted without a live connection.
    NotConnected,
    HostForgotten,
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

#[derive(Debug, Default)]
pub struct AppState {
    pub conn: ConnectionsState,
    pub status: ConnStatus,
    /// Connect/Disconnect requests whose event has not arrived yet. Only the last one counts.
    pub epoch_pending: u32,
    pub sidebar: SidebarState,
    /// Databases whose collection list is in flight.
    pub loading: BTreeSet<String>,
    pub find: FindBarState,
    /// The query behind the page on screen (and the one results are matched against).
    pub last_query: Query,
    pub page: Option<Page>,
    /// Content key of `page`, computed once when the page arrives.
    pub page_key: Option<PageKey>,
    pub tree: TreeState,
    pub view_mode: ViewMode,
    pub editor: Option<EditorState>,
    /// A mutating command is in flight; further mutations are ignored.
    pub pending_mutation: bool,
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

    /// `(has_prev, has_next)` for the find bar.
    pub fn paging(&self) -> (bool, bool) {
        (
            self.last_query.skip > 0,
            self.page.as_ref().is_some_and(|p| p.has_more),
        )
    }

    /// Drops everything tied to a live session; keeps saved connections, banner and form.
    pub fn session_cleared(self) -> Self {
        Self {
            sidebar: SidebarState::default(),
            loading: BTreeSet::new(),
            find: FindBarState::default(),
            last_query: Query::default(),
            page: None,
            page_key: None,
            tree: self.tree.results_replaced(),
            editor: None,
            pending_mutation: false,
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
            ) if is_host_key_changed(error) => Some(endpoint),
            _ => None,
        }
    }
}

/// True for the core error raised when a recorded SSH host key no longer matches.
pub fn is_host_key_changed(error: &CoreError) -> bool {
    matches!(error, CoreError::Ssh(m) if m.contains(crate::i18n::HOST_KEY_CHANGED_MARKER))
}
