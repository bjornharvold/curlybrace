//! Fixed in-memory app states for the screenshot tests. No database, no network.
use super::db::sample_people;
use bson::Document;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::spawn_runtime_with_store;
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::store::{Namespace, Page};
use curlybrace_core::test_support::InMemoryStore;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::host_key::HostKeyRequest;
use curlybrace_ui::app::state::{AppState, ConnStatus};
use curlybrace_ui::viewmodel::{SidebarState, ViewMode, page_key};
use eframe::egui;
use egui_kittest::{Harness, SnapshotOptions};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// Fixed window so every snapshot has the same size on every machine.
pub const SHOT_SIZE: (f32, f32) = (1000.0, 700.0);

/// The harness plus the Tokio runtime the app's (idle) runtime task lives on.
pub struct Shot {
    pub harness: Harness<'static, CurlybraceApp>,
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
}

/// Failed pixels tolerated per snapshot: absorbs text anti-aliasing differences between GPUs and
/// drivers (a handful of pixels) while any changed label or layout differs by far more (a one
/// word label change measured over a thousand).
pub const PIXEL_THRESHOLD: usize = 100;

/// The real app on an in-memory store whose connections file and known_hosts live in a fresh
/// temp dir, never in the developer's real config directory.
pub fn build_app(
    connections: Vec<ConnectionConfig>,
    requests: Option<Receiver<HostKeyRequest>>,
) -> (CurlybraceApp, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let handle = spawn_runtime_with_store(Arc::new(InMemoryStore::new()), Arc::new(|| {}));
    let mut app = CurlybraceApp::new(
        handle,
        I18n::english(),
        connections,
        Arc::new(MemorySecretStore::new()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")))
    .with_known_hosts_path(Some(dir.path().join("known_hosts")));
    if let Some(rx) = requests {
        app = app.with_host_key_requests(rx);
    }
    (app, dir)
}

impl Shot {
    /// Builds the real app on an in-memory store, lets `setup` shape its state, renders with wgpu.
    pub fn new(connections: Vec<ConnectionConfig>, setup: impl FnOnce(&mut AppState)) -> Self {
        Self::build(connections, setup, None)
    }

    /// Like [`Shot::new`] with SSH host-key questions arriving on `requests`.
    pub fn with_host_key_requests(
        connections: Vec<ConnectionConfig>,
        setup: impl FnOnce(&mut AppState),
        requests: Receiver<HostKeyRequest>,
    ) -> Self {
        Self::build(connections, setup, Some(requests))
    }

    fn build(
        connections: Vec<ConnectionConfig>,
        setup: impl FnOnce(&mut AppState),
        requests: Option<Receiver<HostKeyRequest>>,
    ) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .expect("tokio runtime");
        let _enter = rt.enter();
        let (mut app, dir) = build_app(connections, requests);
        setup(app.state_mut());
        let harness = Harness::builder()
            .with_size(egui::vec2(SHOT_SIZE.0, SHOT_SIZE.1))
            .with_pixels_per_point(1.0)
            .wgpu()
            .build_state(|ctx, app: &mut CurlybraceApp| app.run_frame(ctx), app);
        drop(_enter);
        Self {
            harness,
            _rt: rt,
            _dir: dir,
        }
    }

    /// Settles animations, then compares with `tests/snapshots/<name>.png`.
    #[track_caller]
    pub fn snapshot(&mut self, name: &str) {
        self.harness.run();
        let options = SnapshotOptions::new().failed_pixel_count_threshold(PIXEL_THRESHOLD);
        self.harness.snapshot_options(name, &options);
    }
}

/// A connected browser on `crm.customers` showing the sample people in `mode`.
pub fn browsing(mode: ViewMode) -> impl FnOnce(&mut AppState) {
    move |state| {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let ns = Namespace::new("crm", "customers");
        state.status = ConnStatus::Connected;
        state.sidebar = SidebarState::default()
            .with_databases(names(&["billing", "crm", "sandbox"]))
            .toggled("crm")
            .with_collections("crm", names(&["customers", "invoices", "notes"]))
            .selecting(ns);
        install_page(state, sample_people());
        state.view_mode = mode;
    }
}

/// Puts `docs` on screen as the current page (keeping the key in sync).
pub fn install_page(state: &mut AppState, docs: Vec<Document>) {
    let page = Page {
        docs,
        skip: 0,
        has_more: false,
    };
    state.page_key = Some(page_key(&page));
    state.page = Some(page);
}
