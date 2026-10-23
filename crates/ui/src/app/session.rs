//! The view of every open session: the active one lives in `AppState` itself, the others are
//! parked here until the user switches to them.
use super::reducer::Out;
use super::state::{AppState, ConnStatus, HostEndpoint, Pending};
use crate::I18n;
use crate::viewmodel::{ConnectionBadge, connection_list};
use crate::viewmodel::{EditorState, FindBarState, PageKey, SidebarState, TreeState, ViewMode};
use curlybrace_core::connection::{ConnectionColor, ConnectionConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::store::{Page, Query, Topology};
use std::collections::BTreeSet;
use uuid::Uuid;

/// Everything one open session shows; the active session's copy lives in `AppState` itself.
#[derive(Debug, Default)]
pub struct SessionView {
    pub status: ConnStatus,
    pub failure: Option<CoreError>,
    pub latest_requested_epoch: u64,
    pub topology: Option<Topology>,
    pub sidebar: SidebarState,
    pub loading: BTreeSet<String>,
    pub find: FindBarState,
    pub issued_query: Query,
    pub page_query: Query,
    pub loading_page: bool,
    pub page: Option<Page>,
    pub page_key: Option<PageKey>,
    pub tree: TreeState,
    pub view_mode: ViewMode,
    pub editor: Option<EditorState>,
    pub pending: Option<Pending>,
    pub ssh_endpoint: Option<HostEndpoint>,
}

/// One entry of the connection tree: an open session and what to show for it.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub id: Uuid,
    pub name: String,
    pub status: ConnStatus,
    pub topology: Option<Topology>,
    pub failure: Option<CoreError>,
    pub active: bool,
    pub color: Option<ConnectionColor>,
    pub production: bool,
}

impl Root {
    /// The colour tag and production flag to draw beside the name.
    pub fn badge(&self) -> ConnectionBadge {
        ConnectionBadge::new(self.color, self.production)
    }
}

/// The name to show for the open session `id`: see [`AppState::open_label`].
pub fn session_display_name(state: &AppState, id: Uuid, i18n: &I18n) -> Option<String> {
    state.open_label(id, i18n)
}

/// The first four hex characters of a connection id.
fn short_id(id: Uuid) -> String {
    id.simple().to_string().chars().take(4).collect()
}

/// What tells a connection apart from same-named ones: its servers joined with the language's
/// list `separator` (shown to the user), or its short id without any.
fn distinguishing_servers(cfg: &ConnectionConfig, separator: &str) -> String {
    let servers = connection_list::server_hosts(cfg).join(separator);
    if servers.is_empty() {
        short_id(cfg.id)
    } else {
        servers
    }
}

impl AppState {
    /// Moves the active view out (leaving defaults); local mutation only, the public API stays
    /// immutable.
    fn take_view(&mut self) -> SessionView {
        SessionView {
            status: std::mem::take(&mut self.status),
            failure: self.failure.take(),
            latest_requested_epoch: std::mem::take(&mut self.latest_requested_epoch),
            topology: self.topology.take(),
            sidebar: std::mem::take(&mut self.sidebar),
            loading: std::mem::take(&mut self.loading),
            find: std::mem::take(&mut self.find),
            issued_query: std::mem::take(&mut self.issued_query),
            page_query: std::mem::take(&mut self.page_query),
            loading_page: std::mem::take(&mut self.loading_page),
            page: self.page.take(),
            page_key: self.page_key.take(),
            tree: std::mem::take(&mut self.tree),
            view_mode: std::mem::take(&mut self.view_mode),
            editor: self.editor.take(),
            pending: self.pending.take(),
            ssh_endpoint: self.ssh_endpoint.take(),
        }
    }

    /// Destructures exhaustively: a field added to `SessionView` but not restored here fails to
    /// compile.
    fn restore_view(&mut self, v: SessionView) {
        let SessionView {
            status,
            failure,
            latest_requested_epoch,
            topology,
            sidebar,
            loading,
            find,
            issued_query,
            page_query,
            loading_page,
            page,
            page_key,
            tree,
            view_mode,
            editor,
            pending,
            ssh_endpoint,
        } = v;
        self.status = status;
        self.failure = failure;
        self.latest_requested_epoch = latest_requested_epoch;
        self.topology = topology;
        self.sidebar = sidebar;
        self.loading = loading;
        self.find = find;
        self.issued_query = issued_query;
        self.page_query = page_query;
        self.loading_page = loading_page;
        self.page = page;
        self.page_key = page_key;
        self.tree = tree;
        self.view_mode = view_mode;
        self.editor = editor;
        self.pending = pending;
        self.ssh_endpoint = ssh_endpoint;
    }

    /// Makes `id` the active session, parking the previous one. Unknown ids change nothing.
    pub fn activated(self, id: Uuid) -> AppState {
        if self.active == Some(id) || !self.open.contains(&id) {
            return self;
        }
        let mut next = self;
        if let Some(current) = next.active {
            let view = next.take_view();
            next.parked.insert(current, view);
        }
        let incoming = next.parked.remove(&id).unwrap_or_default();
        next.restore_view(incoming);
        next.active = Some(id);
        next
    }

    /// Runs `f` as if `id` were the active session, then puts the previous active session back.
    /// The commands `f` builds carry `id` because `active` is `id` while it runs.
    pub fn with_session(self, id: Uuid, f: impl FnOnce(AppState) -> Out) -> Out {
        if self.active == Some(id) {
            return f(self);
        }
        if !self.open.contains(&id) {
            return (self, Vec::new());
        }
        let previous = self.active;
        let (state, commands) = f(self.activated(id));
        let state = match previous.filter(|p| state.open.contains(p)) {
            Some(p) => state.activated(p),
            // Nothing was active before: park `id` again so no session is foregrounded by a
            // background answer.
            None if previous.is_none() => state.deactivated(),
            None => state,
        };
        (state, commands)
    }

    /// Parks the active view and leaves no session active.
    fn deactivated(self) -> AppState {
        let mut next = self;
        if let Some(current) = next.active.take() {
            let view = next.take_view();
            next.parked.insert(current, view);
        }
        next
    }

    /// The status of an open session, wherever its view lives.
    pub fn status_of(&self, id: Uuid) -> Option<ConnStatus> {
        if !self.open.contains(&id) {
            return None;
        }
        if self.active == Some(id) {
            return Some(self.status);
        }
        Some(
            self.parked
                .get(&id)
                .map_or(ConnStatus::default(), |v| v.status),
        )
    }

    /// The SSH endpoint an open session connects through, wherever its view lives.
    fn ssh_endpoint_of(&self, id: Uuid) -> Option<&HostEndpoint> {
        if self.active == Some(id) {
            return self.ssh_endpoint.as_ref();
        }
        self.parked.get(&id).and_then(|v| v.ssh_endpoint.as_ref())
    }

    /// The SSH host an open session connects through, wherever its view lives.
    pub fn ssh_host_of(&self, id: Uuid) -> Option<String> {
        self.ssh_endpoint_of(id).map(|e| e.host.clone())
    }

    /// The SSH host of the saved connection `id` (open or not), if it goes through one.
    pub fn ssh_host_of_saved(&self, id: Uuid) -> Option<String> {
        self.connection(id)
            .and_then(HostEndpoint::of)
            .map(|e| e.host)
    }

    /// Whether some open session that is still connecting goes through SSH host `host`.
    pub fn any_connecting_via(&self, host: &str) -> bool {
        self.open.iter().any(|id| {
            self.status_of(*id) == Some(ConnStatus::Connecting)
                && self
                    .ssh_endpoint_of(*id)
                    .is_some_and(|e| e.host.eq_ignore_ascii_case(host))
        })
    }

    /// The name of the one open connecting session whose SSH host is `host`; `None` when no
    /// session or several sessions match (the prompt then cannot say who is asking).
    pub fn connecting_name_for_host(&self, host: &str, i18n: &I18n) -> Option<String> {
        let mut matching = self.open.iter().filter(|id| {
            self.status_of(**id) == Some(ConnStatus::Connecting)
                && self
                    .ssh_endpoint_of(**id)
                    .is_some_and(|e| e.host.eq_ignore_ascii_case(host))
        });
        let only = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        self.open_label(*only, i18n)
    }

    /// THE label of the open session `id`, used wherever a session is named. The saved name,
    /// unless another open session's connection shares it; then "name (servers)", plus the short
    /// id when those servers are identical too. `None` when the saved connection was deleted.
    /// Display only: saved names are never changed.
    pub fn open_label(&self, id: Uuid, i18n: &I18n) -> Option<String> {
        let cfg = self.connection(id)?;
        let twins: Vec<&ConnectionConfig> = self
            .open
            .iter()
            .filter(|other| **other != id)
            .filter_map(|other| self.connection(*other))
            .filter(|other| other.name == cfg.name)
            .collect();
        if twins.is_empty() {
            return Some(cfg.name.clone());
        }
        let separator = i18n.t("connections.list_separator");
        let servers = distinguishing_servers(cfg, &separator);
        let name = cfg.name.as_str();
        let label = if twins
            .iter()
            .any(|t| distinguishing_servers(t, &separator) == servers)
        {
            let params = [("name", name), ("servers", &servers), ("id", &short_id(id))];
            i18n.t_with("sidebar.name_with_servers_id", &params)
        } else {
            i18n.t_with(
                "sidebar.name_with_servers",
                &[("name", name), ("servers", &servers)],
            )
        };
        Some(label)
    }

    /// Opens `id` (keeping its place when already open) with `view` as its whole view and makes it
    /// active; the previously active view is parked.
    pub fn opened_with(self, id: Uuid, view: SessionView) -> AppState {
        let mut next = self;
        if !next.open.contains(&id) {
            next.open.push(id);
        }
        if next.active == Some(id) {
            next.restore_view(view);
            return next;
        }
        next.parked.insert(id, view);
        next.activated(id)
    }

    /// The active session's view after its connect failed: everything cleared except what
    /// identifies the attempt (its epoch and endpoint), plus the error.
    pub fn failed(self, error: CoreError) -> AppState {
        let mut next = self;
        let old = next.take_view();
        next.restore_view(SessionView {
            status: ConnStatus::Failed,
            failure: Some(error),
            latest_requested_epoch: old.latest_requested_epoch,
            ssh_endpoint: old.ssh_endpoint,
            ..SessionView::default()
        });
        next
    }

    pub fn is_open(&self, id: Uuid) -> bool {
        self.open.contains(&id)
    }

    /// Forgets a session and its parked view; dropping the active one leaves no active view.
    pub fn session_dropped(self, id: Uuid) -> AppState {
        let mut next = self;
        next.open.retain(|open| *open != id);
        next.parked.remove(&id);
        if next.active == Some(id) {
            next.take_view();
            next.active = None;
        }
        next
    }

    /// The open sessions in opening order, the active one read from the foreground fields.
    pub fn roots(&self, i18n: &I18n) -> Vec<Root> {
        self.open.iter().map(|id| self.root(*id, i18n)).collect()
    }

    fn root(&self, id: Uuid, i18n: &I18n) -> Root {
        let name = self
            .open_label(id, i18n)
            .unwrap_or_else(|| id.to_string().chars().take(8).collect());
        let active = self.active == Some(id);
        let (status, topology, failure) = if active {
            (self.status, self.topology, self.failure.clone())
        } else {
            let fallback = SessionView::default();
            let v = self.parked.get(&id).unwrap_or(&fallback);
            (v.status, v.topology, v.failure.clone())
        };
        Root {
            id,
            name,
            status,
            topology,
            failure,
            active,
            color: self.connection(id).and_then(|c| c.color),
            production: self.connection(id).is_some_and(|c| c.production),
        }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod session_tests;
