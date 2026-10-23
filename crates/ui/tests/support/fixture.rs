//! The real app + real runtime + a real server, assembled for one test.
use super::db::TestDb;
use super::screen::Labels;
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::runtime::{RuntimeConfig, spawn_runtime_with_config};
use curlybrace_core::secrets::{MemorySecretStore, SecretStore};
use curlybrace_testkit::{
    Container, MongoServer, docker_enabled, local_enabled, local_server, poll_with,
    unreachable_message,
};
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::file_picker::FilePicker;
use curlybrace_ui::app::host_key::InteractivePrompt;
use curlybrace_ui::app::runtime_config;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use uuid::Uuid;

pub const WINDOW: (f32, f32) = (1000.0, 700.0);
/// A container needs time to boot; an already-running local server answers at once.
const OWNED_READY_LIMIT: Duration = Duration::from_secs(90);
const LOCAL_READY_LIMIT: Duration = Duration::from_secs(10);

pub struct Fixture {
    pub harness: Harness<'static, CurlybraceApp>,
    pub labels: Labels,
    pub secrets: Arc<MemorySecretStore>,
    pub config_path: PathBuf,
    pub db: TestDb,
    pub server: MongoServer,
    _dir: TempDir,
    rt: Runtime,
}

/// The runtime settings every harness uses: the app's own, except that index history lives in the
/// test's temporary directory, never in the developer's real config directory.
pub fn harness_runtime_config(dir: &Path, prompt: Arc<InteractivePrompt>) -> RuntimeConfig {
    runtime_config(
        Some(dir.join("known_hosts")),
        Some(dir.join("history.sqlite")),
        prompt,
    )
}

/// Builds the app on `rt`: the real runtime (real driver connector), an in-memory secret store
/// (never the Keychain) and a connections file inside `dir`.
fn launch(
    rt: &Runtime,
    dir: &TempDir,
    secrets: Arc<MemorySecretStore>,
    saved: Vec<ConnectionConfig>,
    picker: Option<Arc<dyn FilePicker>>,
) -> (Harness<'static, CurlybraceApp>, PathBuf) {
    let _enter = rt.enter();
    let config_path = dir.path().join("connections.json");
    let known_hosts = dir.path().join("known_hosts");
    let (prompt, requests) = InteractivePrompt::channel(Arc::new(|| {}), Duration::from_secs(5));
    let config = harness_runtime_config(dir.path(), prompt);
    let store: Arc<dyn SecretStore> = secrets;
    let handle = spawn_runtime_with_config(Some(store.clone()), None, Arc::new(|| {}), config);
    let app = CurlybraceApp::new(handle, curlybrace_ui::I18n::english(), saved, store)
        .with_config_path(Ok(config_path.clone()))
        .with_known_hosts_path(Some(known_hosts))
        .with_host_key_requests(requests);
    let app = match picker {
        Some(picker) => app.with_file_picker(picker, Arc::new(|| {})),
        None => app,
    };
    let harness = Harness::builder()
        .with_size(eframe::egui::vec2(WINDOW.0, WINDOW.1))
        .build_state(|ctx, app: &mut CurlybraceApp| app.run_frame(ctx), app);
    (harness, config_path)
}

/// Pings `uri` until it answers. Fails (never skips) with `what` once `limit` has passed.
pub fn wait_ready(rt: &Runtime, uri: &str, limit: Duration) -> mongodb::Client {
    let mut ready = None;
    poll_with(limit, "the MongoDB server", || {
        let attempt = rt.block_on(async {
            let client = mongodb::Client::with_uri_str(uri).await?;
            client
                .database("admin")
                .run_command(bson::doc! {"ping": 1})
                .await?;
            Ok::<_, mongodb::error::Error>(client)
        });
        attempt
            .map(|client| ready = Some(client))
            .map_err(|e| e.to_string())
    });
    ready.expect("poll_with returned without a client")
}

/// What the app starts with and how to tell the server is up.
pub struct Setup {
    /// A URI (with credentials, if the server needs them) the driver can ping.
    pub ready_uri: String,
    pub saved: Vec<ConnectionConfig>,
    /// `(secret key, value)` pairs placed in the in-memory secret store before launch.
    pub secrets: Vec<(String, String)>,
}

/// The name of the saved connection `Setup::saved_connection` creates.
pub const LOCAL_CONNECTION_NAME: &str = "local";

impl Setup {
    pub fn saved_connection(server: &MongoServer, saved: bool) -> Self {
        let uri = server.uri();
        let saved = if saved {
            vec![ConnectionConfig::new(LOCAL_CONNECTION_NAME, &uri)]
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
    /// Tier 1: the already-running local server (`CURLYBRACE_IT_MONGODB_URI`, default
    /// `mongodb://127.0.0.1:27017`) with one saved connection to it. `None`, after printing why,
    /// when the integration gate is off. FAILS if the gate is on and no server answers.
    pub fn local(test: &str) -> Option<Self> {
        Self::local_with(test, true)
    }

    /// Tier 1 like [`Self::local`], with `picker` answering every save-file dialog (a test never
    /// opens the native dialog).
    pub fn local_with_picker(test: &str, picker: Arc<dyn FilePicker>) -> Option<Self> {
        local_enabled(test).then(|| {
            let server = local_server();
            let setup = Setup::saved_connection(&server, true);
            Self::start_with_picker(server, setup, Some(picker))
        })
    }

    /// Tier 1 with no saved connections (the "add one through the form" flow).
    pub fn local_empty(test: &str) -> Option<Self> {
        Self::local_with(test, false)
    }

    fn local_with(test: &str, saved: bool) -> Option<Self> {
        local_enabled(test).then(|| {
            let server = local_server();
            let setup = Setup::saved_connection(&server, saved);
            Self::start(server, setup)
        })
    }

    /// Tier 2: a fresh `cb-it-*` container with a fixed port (so it can be stopped and started)
    /// and one saved connection. Needs docker; `None` when the gate is off.
    pub fn docker_plain(test: &str) -> Option<Self> {
        docker_enabled(test).then(|| {
            let container = Container::start_mongo_restartable(&[], &[]);
            container.wait_tcp();
            let server = MongoServer::Owned(container);
            let setup = Setup::saved_connection(&server, true);
            Self::start(server, setup)
        })
    }

    /// Waits for the server, then launches the app with `setup`.
    pub fn start(server: MongoServer, setup: Setup) -> Self {
        Self::start_with_picker(server, setup, None)
    }

    /// [`Self::start`] with a file picker injected into the app.
    pub fn start_with_picker(
        server: MongoServer,
        setup: Setup,
        picker: Option<Arc<dyn FilePicker>>,
    ) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime");
        let client = ready_client(&rt, &server, &setup.ready_uri);
        let db = TestDb::new(client, rt.handle().clone());
        let dir = tempfile::tempdir().expect("temp dir");
        let secrets = Arc::new(MemorySecretStore::new());
        for (key, value) in &setup.secrets {
            secrets.set(key, value).expect("seed secret");
        }
        let (harness, config_path) = launch(&rt, &dir, secrets.clone(), setup.saved, picker);
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
    wait_ready(f.runtime(), &f.server.uri(), OWNED_READY_LIMIT);
}

/// Waits for the server; for the local server an unreachable address fails with instructions.
fn ready_client(rt: &Runtime, server: &MongoServer, ready_uri: &str) -> mongodb::Client {
    match server {
        MongoServer::Owned(_) => wait_ready(rt, ready_uri, OWNED_READY_LIMIT),
        MongoServer::External(uri) => {
            let attempt: Result<mongodb::Client, String> = rt.block_on(async {
                let client = mongodb::Client::with_uri_str(ready_uri)
                    .await
                    .map_err(|e| e.to_string())?;
                let admin = client.database("admin");
                let ping = admin.run_command(bson::doc! {"ping": 1});
                let answered = tokio::time::timeout(LOCAL_READY_LIMIT, ping).await;
                match answered {
                    Ok(Ok(_)) => Ok(client),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err(format!("no answer within {LOCAL_READY_LIMIT:?}")),
                }
            });
            attempt.unwrap_or_else(|e| panic!("{}", unreachable_message(uri, &e)))
        }
    }
}
