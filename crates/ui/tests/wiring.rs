//! End-to-end wiring without a window: the REAL runtime over an in-memory store, driven by the
//! app's real frame loop (`run_frame` pumps events through the reducer and sends its commands).
use bson::{Bson, Document, doc};
use curlybrace_core::connection::ConnectionConfig;
use curlybrace_core::error::Result;
use curlybrace_core::runtime::{Connector, Event, RuntimeConfig, spawn_runtime_with_connector};
use curlybrace_core::secrets::MemorySecretStore;
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use curlybrace_core::test_support::InMemoryStore;
use curlybrace_core::tunnel::{HostKeyDecision, HostKeyPrompt};
use curlybrace_ui::I18n;
use curlybrace_ui::app::CurlybraceApp;
use curlybrace_ui::app::host_key::{HostKeyRequest, InteractivePrompt};
use curlybrace_ui::app::reducer::{Input, apply_event, update};
use curlybrace_ui::app::state::{AppState, Banner, ConnStatus};
use curlybrace_ui::viewmodel::{DocAction, EditorAction, FindAction, SidebarAction};
use eframe::egui;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

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

fn ns() -> Namespace {
    Namespace::new("testdb", "people")
}

fn seeded() -> Arc<dyn DocumentStore> {
    let docs = vec![
        doc! {"_id": 1_i32, "name": "alice"},
        doc! {"_id": 2_i32, "name": "bob"},
        doc! {"_id": 3_i32, "name": "alice"},
    ];
    Arc::new(InMemoryStore::new().with_docs(&ns(), docs))
}

struct Harness {
    app: CurlybraceApp,
    ctx: egui::Context,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn frame(&mut self) {
        let Self { app, ctx, .. } = self;
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
    }

    /// Runs frames until `done` holds; fails after a generous deadline.
    fn until(&mut self, what: &str, done: impl Fn(&CurlybraceApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.frame();
            if done(&self.app) {
                return;
            }
            let s = self.app.state();
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: banner={:?} editor={:?} pending={}",
                s.banner,
                s.editor,
                s.pending.is_some()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn names(app: &CurlybraceApp) -> Vec<String> {
    let page = app.state().page.as_ref();
    let docs = page.map(|p| p.docs.clone()).unwrap_or_default();
    docs.iter()
        .map(|d| d.get_str("name").unwrap_or_default().to_string())
        .collect()
}

fn harness() -> (Harness, uuid::Uuid) {
    harness_with(None)
}

fn harness_with(requests: Option<Receiver<HostKeyRequest>>) -> (Harness, uuid::Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(Fixed(seeded())),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let cfg = ConnectionConfig::new("local", "mongodb://localhost:27017");
    let id = cfg.id;
    let app = CurlybraceApp::new(
        handle,
        I18n::english(),
        vec![cfg],
        Arc::new(MemorySecretStore::new()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")));
    let app = match requests {
        Some(rx) => app.with_host_key_requests(rx),
        None => app,
    };
    let harness = Harness {
        app,
        ctx: egui::Context::default(),
        _dir: dir,
    };
    (harness, id)
}

fn harness_two() -> (Harness, [uuid::Uuid; 2]) {
    let dir = tempfile::tempdir().unwrap();
    let handle = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(Fixed(seeded())),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let a = ConnectionConfig::new("alpha", "mongodb://localhost:27017");
    let b = ConnectionConfig::new("beta", "mongodb://localhost:27018");
    let ids = [a.id, b.id];
    let app = CurlybraceApp::new(
        handle,
        I18n::english(),
        vec![a, b],
        Arc::new(MemorySecretStore::new()),
    )
    .with_config_path(Ok(dir.path().join("connections.json")));
    let harness = Harness {
        app,
        ctx: egui::Context::default(),
        _dir: dir,
    };
    (harness, ids)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_open_connections_each_stamp_only_their_own_last_connected() {
    let (mut h, [a, b]) = harness_two();
    let both = |app: &CurlybraceApp| {
        [a, b].map(|id| app.state().status_of(id) == Some(ConnStatus::Connected))
    };
    h.app.dispatch(Input::Connect(a));
    h.until("first connected", |app| both(app)[0]);
    let file = h._dir.path().join("connections.json");
    let stamped = |id| {
        let loaded = curlybrace_ui::app::persistence::load_connections(&file).unwrap();
        loaded
            .connections
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.last_connected)
    };
    assert!(stamped(a).is_some());
    assert!(stamped(b).is_none(), "the other one was never connected");
    h.app.dispatch(Input::Connect(b));
    h.until("both connected", |app| both(app) == [true, true]);
    assert!(stamped(a).is_some() && stamped(b).is_some());
    assert_eq!(h.app.state().open, vec![a, b]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connecting_stamps_last_connected_in_the_saved_file() {
    let (mut h, id) = harness();
    h.app.dispatch(Input::Connect(id));
    h.until("connected", |a| a.state().status == ConnStatus::Connected);
    let file = h._dir.path().join("connections.json");
    let loaded = curlybrace_ui::app::persistence::load_connections(&file).unwrap();
    let saved = loaded.connections.iter().find(|c| c.id == id).unwrap();
    assert!(
        saved.last_connected.is_some(),
        "the connect was stamped on disk"
    );
    assert_eq!(
        h.app.state().connection(id).unwrap().last_connected,
        saved.last_connected
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn browse_edit_and_delete_through_the_real_runtime() {
    let (mut h, id) = harness();

    // connect -> list databases
    h.app.dispatch(Input::Connect(id));
    h.until("databases", |a| {
        a.state().status == ConnStatus::Connected && !a.state().sidebar.databases.is_empty()
    });
    assert_eq!(h.app.state().sidebar.databases[0].name, "testdb");

    // expand -> collections, open -> first page
    h.app
        .dispatch(Input::Sidebar(SidebarAction::LoadCollections(
            "testdb".into(),
        )));
    h.until("collections", |a| {
        a.state().sidebar.databases[0].collections.is_some()
    });
    h.app.dispatch(Input::Sidebar(SidebarAction::Open(ns())));
    h.until("first page", |a| names(a).len() == 3);

    // find
    let by_name = Query {
        filter: doc! {"name": "bob"},
        ..Query::default()
    };
    h.app.dispatch(Input::Find(FindAction::Run(by_name)));
    h.until("filtered page", |a| names(a) == ["bob"]);

    // edit -> save -> Mutated -> re-find shows the changed value
    let bob: Document = h.app.state().page.as_ref().unwrap().docs[0].clone();
    h.app.dispatch(Input::Doc(DocAction::Edit(bob)));
    let editor = h.app.state().editor.clone().expect("editor opened");
    let text = editor.text.replace("bob", "bobby");
    // What the user typed into the modal.
    h.app.state_mut().editor.as_mut().unwrap().text = text.clone();
    h.app.dispatch(Input::Editor(EditorAction::Save {
        original_id: editor.original_id,
        text,
    }));
    assert!(h.app.state().pending.is_some());
    // The current query (name == "bob") is re-run, so the renamed document drops out of it.
    h.until("re-fetched edit", |a| {
        a.state().editor.is_none() && a.state().pending.is_none() && names(a).is_empty()
    });
    h.app.dispatch(Input::Find(FindAction::Clear));
    h.until("cleared query", |a| names(a) == ["alice", "bobby", "alice"]);

    // delete (the confirmation step is the view's; the reducer then accepts the id)
    let id2 = Bson::Int32(2);
    h.app.dispatch(Input::Doc(DocAction::Delete(id2)));
    h.until("re-fetched delete", |a| {
        names(a) == ["alice", "alice"] && a.state().pending.is_none()
    });
    assert!(h.app.state().banner.is_none());

    // a data action after disconnecting is refused locally
    h.app.dispatch(Input::Disconnect(id));
    h.app.dispatch(Input::Find(FindAction::Clear));
    assert_eq!(h.app.state().banner, Some(Banner::NotConnected));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_and_clone_add_documents_and_failures_keep_the_page() {
    let (mut h, id) = harness();
    h.app.dispatch(Input::Connect(id));
    h.until("connected", |a| a.state().status == ConnStatus::Connected);
    h.app.dispatch(Input::Sidebar(SidebarAction::Open(ns())));
    h.until("page", |a| names(a).len() == 3);

    h.app.dispatch(Input::Doc(DocAction::Insert));
    h.app.dispatch(Input::Editor(EditorAction::Save {
        original_id: None,
        text: r#"{"_id": 4, "name": "dora"}"#.into(),
    }));
    h.until("insert", |a| names(a).len() == 4);

    let source = h.app.state().page.as_ref().unwrap().docs[0].clone();
    h.app.dispatch(Input::Doc(DocAction::Clone(source)));
    h.until("clone", |a| names(a).len() == 5);

    // The in-memory store rejects operator filters: the find fails, the page stays.
    let bad = Query {
        filter: doc! {"name": {"$regex": "a"}},
        ..Query::default()
    };
    h.app.dispatch(Input::Find(FindAction::Run(bad)));
    h.until("find failure", |a| a.state().banner.is_some());
    assert_eq!(names(&h.app).len(), 5, "previous page stays visible");
    assert!(matches!(
        h.app.state().banner,
        Some(Banner::Failure {
            context: "find",
            ..
        })
    ));
}

fn asking_harness() -> (Harness, uuid::Uuid, Arc<InteractivePrompt>) {
    let (prompt, requests) = InteractivePrompt::channel(Arc::new(|| {}), Duration::from_secs(10));
    let (h, id) = harness_with(Some(requests));
    (h, id, prompt)
}

fn ask(prompt: &Arc<InteractivePrompt>) -> std::thread::JoinHandle<HostKeyDecision> {
    let prompt = prompt.clone();
    std::thread::spawn(move || prompt.decide("bastion", "SHA256:abc"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_key_question_is_shown_and_answered_exactly_once() {
    let (mut h, _id, prompt) = asking_harness();
    let asker = ask(&prompt);
    h.until("question", |a| a.pending_host_key().is_some());
    assert_eq!(
        h.app.pending_host_key(),
        Some(("bastion", "SHA256:abc", None))
    );
    h.app.answer_host_key(HostKeyDecision::TrustAndRemember);
    assert_eq!(asker.join().unwrap(), HostKeyDecision::TrustAndRemember);
    assert!(h.app.pending_host_key().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_the_last_session_rejects_a_pending_question() {
    let (mut h, id, prompt) = asking_harness();
    h.app.dispatch(Input::Connect(id));
    let asker = ask(&prompt);
    h.until("question", |a| a.pending_host_key().is_some());
    h.app.dispatch(Input::Disconnect(id));
    assert_eq!(asker.join().unwrap(), HostKeyDecision::Reject);
    assert!(h.app.pending_host_key().is_none());
}

type Opened = Result<Arc<dyn DocumentStore>>;

/// Connector whose outcome per connection name is released by the test.
struct Gated(
    std::sync::Mutex<std::collections::HashMap<String, tokio::sync::oneshot::Receiver<Opened>>>,
);

#[async_trait::async_trait]
impl Connector for Gated {
    async fn connect(
        &self,
        cfg: &ConnectionConfig,
        _password: Option<&str>,
        _tunnel: Option<SocketAddr>,
    ) -> Opened {
        let gate = self.0.lock().unwrap().remove(&cfg.name).expect("gate");
        gate.await
            .unwrap_or_else(|_| Err(curlybrace_core::error::CoreError::Database("gone".into())))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_first_connect_failing_later_only_fails_its_own_root() {
    use tokio::sync::oneshot;
    // One repaint per delivered event: lets the test wait for an exact number of answers.
    let delivered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let repaint: Arc<dyn Fn() + Send + Sync> = {
        let delivered = delivered.clone();
        Arc::new(move || {
            delivered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        })
    };
    let (tx_a, rx_a) = oneshot::channel::<Opened>();
    let (tx_b, rx_b) = oneshot::channel::<Opened>();
    let gates = std::collections::HashMap::from([("a".to_string(), rx_a), ("b".to_string(), rx_b)]);
    let handle = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(Gated(std::sync::Mutex::new(gates))),
        repaint,
        RuntimeConfig::default(),
    );
    let (a, b) = (
        ConnectionConfig::new("a", "mongodb://localhost:27017"),
        ConnectionConfig::new("b", "mongodb://localhost:27017"),
    );
    let (id_a, id_b) = (a.id, b.id);
    let app = CurlybraceApp::new(
        handle,
        I18n::english(),
        vec![a, b],
        Arc::new(MemorySecretStore::new()),
    );
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness {
        app: app.with_config_path(Ok(dir.path().join("connections.json"))),
        ctx: egui::Context::default(),
        _dir: dir,
    };

    h.app.dispatch(Input::Connect(id_a));
    h.app.dispatch(Input::Connect(id_b));
    tx_b.send(Ok(seeded())).ok();
    h.until("B connected and listed", |a| {
        a.state().status == ConnStatus::Connected && !a.state().sidebar.databases.is_empty()
    });
    // A (a separate session, still open in the background) now fails with a genuine error: only
    // its own root is marked failed, the active session is not disturbed.
    tx_a.send(Err(curlybrace_core::error::CoreError::Database(
        "late".into(),
    )))
    .ok();
    // Exactly three answers exist: B's Connected, the ListDatabases reply and A's failure.
    // Wait for all of them (not for a quiet period), then look at the final state.
    let deadline = Instant::now() + Duration::from_secs(10);
    while delivered.load(std::sync::atomic::Ordering::SeqCst) < 3 {
        assert!(Instant::now() < deadline, "A's answer never arrived");
        std::thread::sleep(Duration::from_millis(2));
    }
    h.frame();
    let state = h.app.state();
    assert_eq!(
        (state.active, state.status),
        (Some(id_b), ConnStatus::Connected)
    );
    assert!(!state.sidebar.databases.is_empty());
    assert_eq!(state.status_of(id_a), Some(ConnStatus::Failed));
}

fn step(
    state: AppState,
    input: Input,
    tx: &tokio::sync::mpsc::UnboundedSender<curlybrace_core::runtime::Command>,
) -> AppState {
    let (state, commands) = update(state, input);
    for command in commands {
        tx.send(command).unwrap();
    }
    state
}

async fn recv(h: &mut curlybrace_core::runtime::RuntimeHandle) -> Event {
    tokio::time::timeout(Duration::from_secs(10), h.rx.recv())
        .await
        .expect("event in time")
        .expect("channel open")
}

/// The UI's mirrored request counter must equal the epoch the real runtime stamps, including
/// for a connect that fails `validate()` and for a superseded one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mirrored_request_counter_matches_the_epochs_the_runtime_stamps() {
    use tokio::sync::oneshot;
    let (tx_a, rx_a) = oneshot::channel::<Opened>();
    let (tx_c, rx_c) = oneshot::channel::<Opened>();
    let gates = std::collections::HashMap::from([("a".to_string(), rx_a), ("c".to_string(), rx_c)]);
    let mut handle = spawn_runtime_with_connector(
        None,
        None,
        Arc::new(Gated(std::sync::Mutex::new(gates))),
        Arc::new(|| {}),
        RuntimeConfig::default(),
    );
    let bad = ConnectionConfig::new("", "mongodb://localhost:27017");
    let a = ConnectionConfig::new("a", "mongodb://localhost:27017");
    let c = ConnectionConfig::new("c", "mongodb://localhost:27017");
    let (id_bad, id_a, id_c) = (bad.id, a.id, c.id);
    let tx = handle.tx.clone();
    let mut state = AppState::new(vec![bad, a, c]);

    // Connect #1 fails validate(): it still consumes an epoch in the runtime.
    state = step(state, Input::Connect(id_bad), &tx);
    assert_eq!(state.latest_requested_epoch, 1);
    let failed = recv(&mut handle).await;
    match &failed {
        Event::ConnectFailed { epoch, .. } => assert_eq!(*epoch, state.latest_requested_epoch),
        other => panic!("unexpected {other:?}"),
    }
    // The UI sees the failure, so no session is open when the next connect is requested.
    state = apply_event(state, failed);
    state = update(state, Input::DismissBanner).0;

    // Connect #2 (slow), then connect #3 for c (fast): several sessions are open at once, so
    // nothing is superseded and no Disconnect is sent for a.
    state = step(state, Input::Connect(id_a), &tx);
    state = step(state, Input::Connect(id_c), &tx);
    assert_eq!(state.epoch_counter, 3);
    assert_eq!(state.latest_requested_epoch, 3);
    tx_c.send(Ok(seeded())).ok();
    assert_eq!(
        recv(&mut handle).await,
        Event::Connected {
            session: id_c,
            epoch: 3,
            topology: None,
        }
    );
    tx_a.send(Ok(seeded())).ok();
    assert_eq!(
        recv(&mut handle).await,
        Event::Connected {
            session: id_a,
            epoch: 2,
            topology: None,
        }
    );

    // Disconnect #4 takes the next number whichever session it names.
    state = step(state, Input::Disconnect(id_c), &tx);
    assert_eq!(state.epoch_counter, 4);
    assert_eq!(
        recv(&mut handle).await,
        Event::Disconnected {
            session: id_c,
            epoch: state.epoch_counter
        }
    );
}
