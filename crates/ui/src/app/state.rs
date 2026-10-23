//! Application state. Immutable by convention: the reducer consumes a state and returns the next.
use super::connection_io::ImportError;
use super::dialog_banner::DialogBannerWatch;
use super::persistence::{PersistError, PersistGuard, Recovery};
use super::session::SessionView;
use crate::viewmodel::compare::CompareState;
use crate::viewmodel::{EditorState, FindBarState, PageKey, SidebarState, TreeState, ViewMode};
use crate::views::connections::ConnectionsState;
use bson::Bson;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Namespace, Page, Query, Topology};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConnStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    /// The last attempt to open the session failed (its root stays in the tree until closed).
    Failed,
}

/// Which top-level workspace is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Workspace {
    #[default]
    Browse,
    Compare,
    /// The connection manager, shown while sessions stay open in the tree.
    Connections,
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
    /// The file chosen for import was refused. Says nothing about the saved list.
    ImportFailed(ImportError),
    /// Writing the export file failed. Says nothing about the saved list.
    ExportFailed(PersistError),
    /// What startup found wrong with the saved connection list, and whether it was backed up.
    Recovery(Recovery),
    /// A data action was attempted without a live connection.
    NotConnected,
    HostForgotten,
    /// Edit/Clone were refused because the page is shown with a projection (partial documents).
    ProjectionReadOnly,
    /// Queueing fields was refused: the ticked fields (with the ones already queued for the
    /// document) overlap, so one update cannot write them.
    FieldsConflict,
    /// Queueing fields was refused: a whole-category change of the tab already covers them.
    FieldsCovered,
    /// The user confirmed a production write whose connection changed (another session, a
    /// reconnect or a lost connection) in the meantime, so nothing was sent.
    WriteNotSentConnectionChanged,
    /// The user confirmed a production write while another write was still in flight.
    WriteNotSentBusy,
}

impl Banner {
    /// A failed data command of a session (not a connect, not a local operation).
    pub fn is_data_failure(&self) -> bool {
        matches!(
            self,
            Banner::Failure {
                context: "find"
                    | "insert"
                    | "replace"
                    | "delete"
                    | "clone"
                    | "list_databases"
                    | "list_collections",
                ..
            }
        )
    }

    /// A failure raised from one session's events: a data failure or a failed connect.
    pub fn is_session_failure(&self) -> bool {
        self.is_data_failure()
            || matches!(
                self,
                Banner::Failure {
                    context: "connect",
                    ..
                }
            )
    }
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

/// The whole UI state.
///
/// The fields describing the ACTIVE session are `status`, `failure`, `topology`,
/// `latest_requested_epoch`, `sidebar`, `loading`, `find`, `issued_query`, `page_query`,
/// `loading_page`, `page`, `page_key`, `tree`, `view_mode`, `editor`, `pending`,
/// `ssh_endpoint` and `production`. They are parked per session and must be added to `SessionView` (and to
/// `take_view`/`restore_view`) together; `restore_view` destructures it exhaustively so a
/// missed field fails to compile.
#[derive(Debug, Default)]
pub struct AppState {
    pub workspace: Workspace,
    /// Compare sessions have their own connections; this survives browsing connect/disconnect.
    pub compare: CompareState,
    pub conn: ConnectionsState,
    pub status: ConnStatus,
    /// Open sessions in opening order; the active one is among them.
    pub open: Vec<Uuid>,
    /// The session the browsing fields below show (the Compare source default).
    pub active: Option<Uuid>,
    /// The views of the open sessions that are not active.
    pub parked: BTreeMap<Uuid, SessionView>,
    /// Counts connect/disconnect requests sent to the runtime; it mirrors the runtime's own
    /// request counter (bumped once for EVERY Connect/Disconnect command sent, starting at 0,
    /// and the UI is its only sender).
    pub epoch_counter: u64,
    /// Why the active session's last attempt to open failed.
    pub failure: Option<CoreError>,
    /// How the active session's server is deployed, once known.
    pub topology: Option<Topology>,
    /// The epoch of the latest connect requested for the ACTIVE session (parked with its view,
    /// see `epoch_counter` for the global request counter). An outcome stamped with an older
    /// epoch belongs to a request the user has since replaced.
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
    /// A write to a production connection waiting for the user's confirmation. Not part of a
    /// session view: the parked command names its session and is re-validated on confirm.
    pub write_warning: Option<super::write_guard::ParkedWrite>,
    pub persist: PersistGuard,
    pub banner: Option<Banner>,
    /// What the edit dialog may repeat of `banner` (only what is new since it opened).
    pub dialog_banner_watch: DialogBannerWatch,
    /// SSH endpoint of the connection being (or last) opened.
    pub ssh_endpoint: Option<HostEndpoint>,
    /// Whether the active session was opened on a production connection (snapshotted at
    /// connect). Read it through `session_production`, which also counts the live saved flag.
    pub production: bool,
    /// Endpoint awaiting the "forget host" confirmation.
    pub forget_confirm: Option<HostEndpoint>,
    /// The session whose failure raised the current banner (a close of it clears the banner).
    pub banner_session: Option<Uuid>,
    /// The host of the session whose connect failed with a changed host key, captured when the
    /// banner was raised. Only offered while that banner is showing.
    pub forget_target: Option<HostEndpoint>,
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

    /// The colour tag and production flag of the active connection; empty for the embedded
    /// session, which has no saved connection.
    pub fn active_badge(&self) -> crate::viewmodel::ConnectionBadge {
        self.active
            .map(|id| self.session_badge(id))
            .unwrap_or_default()
    }

    /// The badge of the open session `id`: the colour follows the saved connection, production
    /// is `session_production` (opened-with snapshot OR the live saved flag).
    pub fn session_badge(&self, id: Uuid) -> crate::viewmodel::ConnectionBadge {
        let color = self.connection(id).and_then(|c| c.color);
        crate::viewmodel::ConnectionBadge::new(color, self.session_production(id))
    }

    /// The name of the active connection, or a translated placeholder when none is open (so a
    /// write confirmation always names its target).
    pub fn active_connection_label(&self, i18n: &crate::I18n) -> String {
        self.active
            .and_then(|id| self.open_label(id, i18n))
            .unwrap_or_else(|| i18n.t("doc.connection_unknown"))
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

    /// The runtime session browsing commands address: the open connection, or the embedded
    /// session when none was opened through the manager.
    pub fn session_id(&self) -> Uuid {
        self.active.unwrap_or_default()
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
            write_warning: None,
            forget_confirm: None,
            open: Vec::new(),
            active: None,
            parked: BTreeMap::new(),
            failure: None,
            topology: None,
            production: false,
            workspace: Workspace::Browse,
            ..self
        }
    }

    /// The session a banner is about: the one that raised the current failure banner, when that
    /// banner is a session failure (connect or data). Other banners have no owner, whatever
    /// `banner_session` still holds.
    pub fn banner_owner(&self) -> Option<Uuid> {
        match &self.banner {
            Some(banner) if banner.is_session_failure() => self.banner_session,
            _ => None,
        }
    }

    /// The state with no banner, no banner owner and no forget offer.
    pub fn without_banner(self) -> AppState {
        AppState {
            banner: None,
            banner_session: None,
            forget_target: None,
            ..self
        }
    }

    /// Whether the "forget host" action applies to the current banner.
    pub fn forgettable_host(&self) -> Option<&HostEndpoint> {
        // The endpoint was captured when the failure was raised, so it stays that of the session
        // whose key changed whichever session is active now.
        match (&self.banner, &self.forget_target) {
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
