//! Application state. Immutable by convention: the reducer consumes a state and returns the next.
use super::connection_io::ImportError;
use super::dialog_banner::DialogBannerWatch;
use super::explain_settings::ExplainSettings;
use super::persistence::{PersistError, PersistGuard, Recovery};
use super::sampling_settings::SamplingSettings;
use super::session::SessionView;
use super::shell_state::ShellTabs;
use super::tools_state::ToolsState;
use crate::viewmodel::compare::CompareState;
use crate::viewmodel::{
    AggregateState, CollectionTab, CopyClipboard, EditorState, FindBarState, IndexesState, PageKey,
    PasteBlock, SidebarSelection, SidebarState, TreeState, ViewMode,
};
use crate::views::connections::ConnectionsState;
use bson::Bson;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::copy::protocol::CopyRequest;
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Namespace, Page, Query, Topology};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
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
    /// The Shell tab of the active session (Browse stays one switch away).
    Shell,
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
    /// The tool paths file could not be written; the change applies until the app closes.
    ToolsPersist(PersistError),
    /// A connection was deleted or duplicated and its saved pipelines could not be updated on
    /// disk (even after one retry). The list was updated in this window; the file is corrected
    /// by the next successful save.
    PipelinesCleanup,
    /// The file chosen for import was refused. Says nothing about the saved list.
    ImportFailed(ImportError),
    /// Writing the export file failed. Says nothing about the saved list.
    ExportFailed(PersistError),
    /// What startup found wrong with the saved connection list, and whether it was backed up.
    Recovery(Recovery),
    /// A data action was attempted without a live connection.
    NotConnected,
    HostForgotten,
    /// Paste was refused for this reason; nothing was stored or sent.
    PasteBlocked(PasteBlock),
    /// Copy was refused: the selection cannot be copied; the clipboard is unchanged.
    CopyUnavailable,
    /// Edit/Clone were refused because the page is shown with a projection (partial documents).
    ProjectionReadOnly,
    /// A write was refused because the session's connection is read-only; nothing was sent.
    ReadOnly,
    /// Applying a compare was refused because its target connection is read-only.
    CompareApplyReadOnly,
    /// Applying a compare was refused because some of its changes write to the source
    /// connection, which is read-only.
    CompareApplyReadOnlySource,
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
                    | "list_collections"
                    | "list_indexes"
                    | "index_history"
                    | "create_index"
                    | "drop_index"
                    | "set_index_hidden"
                    | "set_index_ttl"
                    // `bulk_cancel` is deliberately absent: its failure raises a banner while
                    // the flow stays open, and a later connect must not withdraw it.
                    | "bulk_count"
                    | "bulk_sample"
                    | "bulk_update"
                    | "bulk_delete",
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
/// `loading_page`, `page`, `page_key`, `verdict`, `guard`, `tree`, `view_mode`, `tab`, `indexes`, `aggregate`, `aggregate_view`, `editor`,
/// `pending`, `ssh_endpoint`, `production` and `read_only`. They are parked per session and must
/// be added to `SessionView` (and to `take_view`/`restore_view`) together; `restore_view` destructures it exhaustively so a
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
    /// The sidebar rows selected, of one session. Not parked: it belongs to the sidebar, which
    /// shows every session's tree, and is cleared when its session closes.
    pub selection: SidebarSelection,
    /// What Copy stored: references to databases or collections, never data.
    pub clipboard: Option<CopyClipboard>,
    /// The copy a Paste asked for, waiting for the paste dialog to take it.
    pub pending_paste: Option<CopyRequest>,
    /// The paste dialog, from the moment it took `pending_paste` until its report is closed.
    pub copy: Option<super::copy_state::CopyDialog>,
    /// Copy run ids handed out so far (previews and copies share the counter).
    pub copy_runs: curlybrace_core::compare::protocol::RunId,
    /// A cancelled preview whose dialog is gone but whose terminal event has not arrived: it
    /// still holds the runtime's one run slot, so the next preview or copy waits for it.
    pub copy_draining: Option<curlybrace_core::compare::protocol::RunId>,
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
    /// How the query bar's current run is served (see `explain_actions`).
    pub verdict: super::explain_actions::VerdictState,
    /// The query bar's index-miss guard for the run asked for (see `guard_actions`). Boxed, like
    /// `shell`: it holds a query and a report, and `AppState` moves by value through every step.
    pub guard: Box<super::guard_actions::GuardState>,
    pub tree: TreeState,
    pub view_mode: ViewMode,
    /// Which tab of the selected collection is shown.
    pub tab: CollectionTab,
    /// The Indexes tab of the selected collection and the session's sampler status.
    pub indexes: IndexesState,
    /// The Aggregate tab's pipeline draft and preview. Boxed, like `guard`: `AppState` moves by
    /// value through every step.
    pub aggregate: Box<AggregateState>,
    /// The Aggregate tab's own view state (the preview's tree and mode, typing not yet sent).
    pub aggregate_view: Box<crate::viewmodel::aggregate_view::AggregateView>,
    /// The databases the usage sampler skips. Global: it belongs to no session, so it is not
    /// parked with one.
    pub sampling: SamplingSettings,
    /// Whether a Run also asks for the index verdict. Global too.
    pub explain: ExplainSettings,
    /// Guard check requests numbered so far. Global: the query bar and the Shell tab share it, so
    /// an answer meant for one never matches a check of the other.
    pub guard_request: u64,
    /// The tool paths (global and per connection) and their version checks. Global too.
    pub tools: ToolsState,
    /// Every connection's saved aggregation pipelines. Global too; changed only through
    /// `persist_pipelines`, so it never runs ahead of the file.
    pub pipelines: curlybrace_core::saved_pipeline::SavedPipelines,
    /// Where `pipelines.json` is; `None` when there is no settings folder (or in tests that do
    /// not set one), so nothing is ever written to the real config directory by accident.
    pub pipelines_path: Option<std::path::PathBuf>,
    /// Whether `pipelines.json` may be replaced (not when a bad one could not be backed up).
    pub pipelines_guard: PersistGuard,
    /// What this window last read from or wrote to `pipelines.json`, to notice another window.
    pub pipelines_seen: super::pipelines_store::DiskStamp,
    /// The `PATH` a `PATH` tool choice is searched in: the process's own plus the usual install
    /// folders in the app; `None` (no search at all) by default, so tests never find a program of
    /// the machine they run on.
    pub tool_search_path: Option<OsString>,
    /// The Shell tabs of the open sessions (see [`ShellTabs`]). Boxed: `AppState` is moved by
    /// value through every reducer step, so it is kept from growing.
    pub shell: Box<ShellTabs>,
    pub editor: Option<EditorState>,
    /// The mutating command in flight; further mutations are ignored until it resolves.
    pub pending: Option<Pending>,
    /// A write to a production connection waiting for the user's confirmation. Not part of a
    /// session view: the parked command names its session and is re-validated on confirm.
    pub write_warning: Option<super::write_guard::ParkedWrite>,
    /// The open administration dialog (drop, rename, duplicate, create view). Not part of a
    /// session view: it names its session and only sends there.
    pub admin: Option<Box<crate::viewmodel::AdminDialog>>,
    /// The open bulk update / delete flow. Not part of a session view either: it names its
    /// session and is closed when that session is closed or reopened.
    pub bulk: Option<super::bulk_state::BulkState>,
    /// Bulk count, preview and sample requests numbered so far. One counter for every flow, so
    /// the answer of a closed flow never matches a later one.
    pub bulk_requests: u64,
    /// Where the runtime keeps undo copies of bulk writes; `None` when undo is off. Set from the
    /// runtime's own configuration at launch, never read from the environment later.
    pub snapshot_dir: Option<std::path::PathBuf>,
    /// The deletion of an undo copy that was asked for and not yet answered: the request id and
    /// the run, so a copy that is still there afterwards can be told.
    pub discarding_copy: Option<(u64, curlybrace_core::bulk::BulkRunId)>,
    /// The list of saved undo copies, while it is open.
    pub copies: Option<super::saved_copies::SavedCopies>,
    pub persist: PersistGuard,
    pub banner: Option<Banner>,
    /// Startup notices waiting for the single banner slot, oldest first: each is shown, once,
    /// as soon as the banner showing now is gone (see `with_queued_recovery_shown`).
    pub pending_recovery: Vec<Recovery>,
    /// What the edit dialog may repeat of `banner` (only what is new since it opened).
    pub dialog_banner_watch: DialogBannerWatch,
    /// SSH endpoint of the connection being (or last) opened.
    pub ssh_endpoint: Option<HostEndpoint>,
    /// Whether the active session was opened on a production connection (snapshotted at
    /// connect). Read it through `session_production`, which also counts the live saved flag.
    pub production: bool,
    /// Whether the active session was opened on a read-only connection (snapshotted at connect).
    /// Read it through `session_read_only`, which also counts the live saved flag.
    pub read_only: bool,
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
        let color = self.connection(id).and_then(|c| c.known_color());
        crate::viewmodel::ConnectionBadge::new(color, self.session_production(id))
            .with_read_only(self.session_read_only(id))
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

    /// Whether a Run also asks the server to explain the query (the global setting).
    pub fn explain_enabled(&self) -> bool {
        self.explain.automatic
    }

    /// Whether a query is checked against the server's plan before it runs (the global setting).
    pub fn guard_enabled(&self) -> bool {
        self.explain.guard
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
            selection: SidebarSelection::default(),
            pending_paste: None,
            copy: None,
            loading: BTreeSet::new(),
            find: FindBarState::default(),
            issued_query: Query::default(),
            page_query: Query::default(),
            loading_page: false,
            page: None,
            page_key: None,
            verdict: Default::default(),
            guard: Default::default(),
            tree: self.tree.results_replaced(),
            tab: CollectionTab::default(),
            indexes: IndexesState::default(),
            aggregate: Box::default(),
            aggregate_view: Box::default(),
            editor: None,
            pending: None,
            write_warning: None,
            admin: None,
            bulk: None,
            shell: Box::default(),
            forget_confirm: None,
            open: Vec::new(),
            active: None,
            parked: BTreeMap::new(),
            failure: None,
            topology: None,
            production: false,
            read_only: false,
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

    /// Puts the oldest queued startup notice in the banner slot when it is free, and only
    /// then; the notice leaves the queue, so it is shown exactly once and the next one follows
    /// when this banner is dismissed.
    pub fn with_queued_recovery_shown(self) -> Self {
        let Some((first, rest)) = self.pending_recovery.split_first() else {
            return self;
        };
        if self.banner.is_some() {
            return self;
        }
        Self {
            banner: Some(Banner::Recovery(first.clone())),
            pending_recovery: rest.to_vec(),
            ..self
        }
    }

    /// Shows `recovery` now if the banner slot is free and nothing is queued ahead of it, else
    /// queues it behind the banner and the earlier notices (none is ever dropped).
    pub fn with_recovery_notice(self, recovery: Recovery) -> Self {
        let pending_recovery = self
            .pending_recovery
            .iter()
            .cloned()
            .chain(std::iter::once(recovery))
            .collect();
        Self {
            pending_recovery,
            ..self
        }
        .with_queued_recovery_shown()
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
