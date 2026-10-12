//! End-to-end wiring without a window: the REAL runtime over an in-memory store, driven by the
//! app's real frame loop (`run_frame` pumps events through the reducer and sends its commands).
use bson::{Bson, Document, doc};
use eframe::egui;
use mongostudio_core::connection::ConnectionConfig;
use mongostudio_core::error::Result;
use mongostudio_core::runtime::{Connector, RuntimeConfig, spawn_runtime_with_connector};
use mongostudio_core::secrets::MemorySecretStore;
use mongostudio_core::store::{DocumentStore, Namespace, Query};
use mongostudio_core::test_support::InMemoryStore;
use mongostudio_core::tunnel::{HostKeyDecision, HostKeyPrompt};
use mongostudio_ui::I18n;
use mongostudio_ui::app::MongoStudioApp;
use mongostudio_ui::app::host_key::{HostKeyRequest, InteractivePrompt};
use mongostudio_ui::app::reducer::Input;
use mongostudio_ui::app::state::{Banner, ConnStatus};
use mongostudio_ui::viewmodel::{DocAction, EditorAction, FindAction, SidebarAction};
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
    app: MongoStudioApp,
    ctx: egui::Context,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn frame(&mut self) {
        let Self { app, ctx, .. } = self;
        let _ = ctx.run(egui::RawInput::default(), |ctx| app.run_frame(ctx));
    }

    /// Runs frames until `done` holds; fails after a generous deadline.
    fn until(&mut self, what: &str, done: impl Fn(&MongoStudioApp) -> bool) {
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

fn names(app: &MongoStudioApp) -> Vec<String> {
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
    let app = MongoStudioApp::new(
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
    h.app.dispatch(Input::Disconnect);
    h.until("disconnected", |a| a.state().epoch_pending == 0);
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
    assert_eq!(h.app.pending_host_key(), Some(("bastion", "SHA256:abc")));
    h.app.answer_host_key(HostKeyDecision::TrustAndRemember);
    assert_eq!(asker.join().unwrap(), HostKeyDecision::TrustAndRemember);
    assert!(h.app.pending_host_key().is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_connect_or_disconnect_rejects_a_pending_question() {
    let (mut h, id, prompt) = asking_harness();
    for input in [Input::Connect(id), Input::Disconnect] {
        let asker = ask(&prompt);
        h.until("question", |a| a.pending_host_key().is_some());
        h.app.dispatch(input);
        assert_eq!(asker.join().unwrap(), HostKeyDecision::Reject);
        assert!(h.app.pending_host_key().is_none());
    }
}
