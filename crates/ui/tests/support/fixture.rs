//! The real app + real runtime + a real server, assembled for one test.
use super::db::TestDb;
use super::screen::Labels;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::runtime::{RuntimeConfig, spawn_runtime_with_config};
use mongostudio_core::secrets::{MemorySecretStore, SecretStore};
use mongostudio_testkit::{MongoServer, it_enabled, it_enabled_with_docker, plain_server};
use mongostudio_ui::app::MongoStudioApp;
use mongostudio_ui::app::host_key::InteractivePrompt;
use mongostudio_ui::app::runtime_config;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::runtime::Runtime;
use uuid::Uuid;

pub const WINDOW: (f32, f32) = (1000.0, 700.0);
const READY_LIMIT: Duration = Duration::from_secs(90);

pub struct Fixture {
    pub harness: Harness<'static, MongoStudioApp>,
    pub labels: Labels,
    pub secrets: Arc<MemorySecretStore>,
    pub config_path: PathBuf,
    pub db: TestDb,
    pub server: MongoServer,
    _dir: TempDir,
    rt: Runtime,
}

/// Builds the app on `rt`: the real runtime (real driver connector), an in-memory secret store
/// (never the Keychain) and a connections file inside `dir`.
fn launch(
    rt: &Runtime,
    dir: &TempDir,
    secrets: Arc<MemorySecretStore>,
    saved: Vec<ConnectionConfig>,
) -> (Harness<'static, MongoStudioApp>, PathBuf) {
    let _enter = rt.enter();
    let config_path = dir.path().join("connections.json");
    let known_hosts = dir.path().join("known_hosts");
    let (prompt, requests) = InteractivePrompt::channel(Arc::new(|| {}), Duration::from_secs(5));
    let config: RuntimeConfig = runtime_config(Some(known_hosts.clone()), prompt);
    let store: Arc<dyn SecretStore> = secrets;
    let handle = spawn_runtime_with_config(Some(store.clone()), None, Arc::new(|| {}), config);
    let app = MongoStudioApp::new(handle, mongostudio_ui::I18n::english(), saved, store)
        .with_config_path(Ok(config_path.clone()))
        .with_known_hosts_path(Some(known_hosts))
        .with_host_key_requests(requests);
    let harness = Harness::builder()
        .with_size(eframe::egui::vec2(WINDOW.0, WINDOW.1))
        .build_state(|ctx, app: &mut MongoStudioApp| app.run_frame(ctx), app);
    (harness, config_path)
}

pub fn wait_ready(rt: &Runtime, uri: &str) -> mongodb::Client {
    let deadline = Instant::now() + READY_LIMIT;
    loop {
        let attempt = rt.block_on(async {
            let client = mongodb::Client::with_uri_str(uri).await?;
            client
                .database("admin")
                .run_command(bson::doc! {"ping": 1})
                .await?;
            Ok::<_, mongodb::error::Error>(client)
        });
        match attempt {
            Ok(client) => return client,
            Err(e) if Instant::now() > deadline => panic!("mongod never became ready: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// What the app starts with and how to tell the server is up.
pub struct Setup {
    /// A URI (with credentials, if the server needs them) the driver can ping.
    pub ready_uri: String,
    pub saved: Vec<ConnectionConfig>,
    /// `(secret key, value)` pairs placed in the in-memory secret store before launch.
    pub secrets: Vec<(String, String)>,
}

impl Setup {
    pub fn saved_connection(server: &MongoServer, saved: bool) -> Self {
        let uri = server.uri();
        let saved = if saved {
            vec![ConnectionConfig::new("local", &uri)]
        } else {
            Vec::new()
        };
        Self {
            ready_uri: uri,
            saved,
            secrets: Vec::new(),
        }
    }
}

impl Fixture {
    /// A plain server (external via `MONGOSTUDIO_IT_MONGO_URI`, else a container) with one saved
    /// connection to it. `None`, after printing why, when the integration gate is off.
    pub fn plain(test: &str) -> Option<Self> {
        it_enabled(test).then(|| {
            let server = plain_server();
            let setup = Setup::saved_connection(&server, true);
            Self::start(server, setup)
        })
    }

    /// Like [`Fixture::plain`] but needs docker container control (stop/start): skips when an
    /// external server is configured.
    pub fn plain_with_docker(test: &str) -> Option<Self> {
        it_enabled_with_docker(test).then(|| {
            let server = plain_server();
            let setup = Setup::saved_connection(&server, true);
            Self::start(server, setup)
        })
    }

    /// No saved connections (the "add one through the form" flow).
    pub fn plain_empty(test: &str) -> Option<Self> {
        it_enabled(test).then(|| {
            let server = plain_server();
            let setup = Setup::saved_connection(&server, false);
            Self::start(server, setup)
        })
    }

    /// Waits for the server, then launches the app with `setup`.
    pub fn start(server: MongoServer, setup: Setup) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let client = wait_ready(&rt, &setup.ready_uri);
        let db = TestDb::new(client, rt.handle().clone());
        let dir = tempfile::tempdir().expect("temp dir");
        let secrets = Arc::new(MemorySecretStore::new());
        for (key, value) in &setup.secrets {
            secrets.set(key, value).expect("seed secret");
        }
        let (harness, config_path) = launch(&rt, &dir, secrets.clone(), setup.saved);
        Self {
            harness,
            labels: Labels::new(),
            secrets,
            config_path,
            db,
            server,
            _dir: dir,
            rt,
        }
    }

    /// The saved connection's id (the first one).
    pub fn saved_id(&self) -> Uuid {
        self.harness.state().state().conn.connections[0].id
    }

    pub fn runtime(&self) -> &Runtime {
        &self.rt
    }

    /// Clicks the label `text` (exactly one node must match).
    pub fn click_text(&mut self, text: &str) {
        self.harness.get_by_label(text).click();
        super::wait::settle(&mut self.harness);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // A container is removed (with its volumes) by its own guard; only a shared external
        // server needs this test's database dropped.
        if matches!(self.server, MongoServer::External(_)) {
            self.db.drop_database();
        }
    }
}

/// Blocks until the fixture's server answers a ping again (after a container restart).
pub fn wait_ready_blocking(f: &Fixture) {
    wait_ready(f.runtime(), &f.server.uri());
}
