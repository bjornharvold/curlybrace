//! Shared harness for the connection manager wiring tests: the real runtime with a fake
//! connector, an in-memory keychain kept by the test, and a config file in a temp dir.
#![allow(dead_code)]
use bson::doc;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::Result;
use curlybrace_core::runtime::{Connector, RuntimeConfig, spawn_runtime_with_connector};
use curlybrace_core::secrets::{MemorySecretStore, SecretKind, SecretStore, secret_key};
use curlybrace_core::store::{DocumentStore, Namespace};
use curlybrace_core::test_support::InMemoryStore;
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::file_picker::FilePicker;
use curlybrace_ui::app::persistence::load_connections;
use eframe::egui;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Fixed(Arc<dyn DocumentStore>);

#[async_trait::async_trait]
impl Connector for Fixed {
    async fn connect(
        &self,
        _cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Result<Arc<dyn DocumentStore>> {
        Ok(self.0.clone())
    }
}

/// Answers every open and save dialog with one fixed path.
pub struct FixedPath(pub PathBuf);

impl FilePicker for FixedPath {
    fn save_file(&self, _name: &str, _extension: &str) -> Option<PathBuf> {
        Some(self.0.clone())
    }

    fn open_file(&self, _extension: &str) -> Option<PathBuf> {
        Some(self.0.clone())
    }
}

pub fn people() -> Namespace {
    Namespace::new("testdb", "people")
}

fn seeded() -> Arc<dyn DocumentStore> {
    let docs = vec![doc! {"_id": 1_i32, "name": "alice"}];
    Arc::new(InMemoryStore::new().with_docs(&people(), docs))
}

pub struct Harness {
    pub app: CurlybraceApp,
    pub secrets: Arc<MemorySecretStore>,
    pub dir: tempfile::TempDir,
    ctx: egui::Context,
}

impl Harness {
    pub fn new(connections: Vec<ConnectionConfig>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let secrets = Arc::new(MemorySecretStore::new());
        Self::on(dir, secrets, connections)
    }

    /// A fresh app (a "restart") over the given directory and keychain.
    pub fn on(
        dir: tempfile::TempDir,
        secrets: Arc<MemorySecretStore>,
        connections: Vec<ConnectionConfig>,
    ) -> Self {
        let store: Arc<dyn SecretStore> = secrets.clone();
        let handle = spawn_runtime_with_connector(
            Some(store.clone()),
            None,
            Arc::new(Fixed(seeded())),
            Arc::new(|| {}),
            RuntimeConfig::default(),
        );
        let app = CurlybraceApp::new(handle, I18n::english(), connections, store)
            .with_config_path(Ok(dir.path().join("connections.json")));
        Self {
            app,
            secrets,
            dir,
            ctx: egui::Context::default(),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.dir.path().join("connections.json")
    }

    pub fn saved_text(&self) -> String {
        std::fs::read_to_string(self.config_file()).unwrap_or_default()
    }

    pub fn saved(&self) -> Vec<ConnectionConfig> {
        load_connections(&self.config_file())
            .expect("the config file loads")
            .connections
    }

    pub fn store_password(&self, id: Uuid, value: &str) {
        self.secrets
            .set(&secret_key(id, SecretKind::Password), value)
            .expect("memory store accepts the secret");
    }

    pub fn password_of(&self, id: Uuid) -> Option<String> {
        self.secrets
            .get(&secret_key(id, SecretKind::Password))
            .expect("memory store answers")
    }

    pub fn frame(&mut self) {
        let Self { app, ctx, .. } = self;
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
    }

    pub fn until(&mut self, what: &str, done: impl Fn(&CurlybraceApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame();
            if done(&self.app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: banner={:?}",
                self.app.state().banner
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

pub fn names(list: &[ConnectionConfig]) -> Vec<String> {
    list.iter().map(|c| c.name.clone()).collect()
}

pub fn exists(path: &Path) -> bool {
    path.exists()
}
