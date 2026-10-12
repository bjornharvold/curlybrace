//! SCRAM authentication against `mongo:7 --auth`, directly and through the
//! real runtime. Set `MONGOSTUDIO_IT=1` to run.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bson::doc;
use mongostudio_core::client::{self, SERVER_SELECTION_TIMEOUT};
use mongostudio_core::connection::{AuthMechanism, ConnectionConfig};
use mongostudio_core::error::CoreError;
use mongostudio_core::mongo_store::MongoStore;
use mongostudio_core::runtime::{Command, Event, RuntimeConfig, spawn_runtime_with_config};
use mongostudio_core::secrets::{MemorySecretStore, SecretKind, SecretStore, secret_key};
use mongostudio_core::store::{DocumentStore, Namespace, Query};
use support::{Container, it_enabled, next_event, no_repaint, wait_ready};

const USER: &str = "root";
/// Every character that is special in a URI, plus a space and non-ASCII.
const PASSWORD: &str = "p@ss:w/rd%2F?#x yé世";
const BOUND: Duration = Duration::from_secs(SERVER_SELECTION_TIMEOUT.as_secs() + 3);

fn auth_container() -> Container {
    Container::start_mongo(
        &[
            format!("MONGO_INITDB_ROOT_USERNAME={USER}"),
            format!("MONGO_INITDB_ROOT_PASSWORD={PASSWORD}"),
        ],
        &["--auth"],
    )
}

fn scram_config(container: &Container) -> ConnectionConfig {
    let mut cfg = container.config();
    cfg.auth = AuthMechanism::ScramSha256;
    cfg.username = Some(USER.into());
    cfg.auth_source = Some("admin".into());
    cfg
}

#[tokio::test]
#[ignore = "needs docker; set MONGOSTUDIO_IT=1"]
async fn scram_sha256_succeeds_and_failures_are_typed_and_bounded() {
    if !it_enabled("scram_sha256_succeeds_and_failures_are_typed_and_bounded") {
        return;
    }
    let container = auth_container();
    let cfg = scram_config(&container);
    let client = wait_ready(&cfg, Some(PASSWORD)).await;
    // Authenticated: an admin-only command works.
    client
        .database("admin")
        .run_command(doc! { "listDatabases": 1 })
        .await
        .unwrap();

    let started = Instant::now();
    let wrong = client::connect(&cfg, Some("not-the-password"), None).await;
    let Err(err) = wrong else {
        panic!("wrong password must fail");
    };
    assert_eq!(err, CoreError::AuthFailed);
    let text = err.to_string();
    assert!(
        started.elapsed() < BOUND,
        "wrong password took {:?}",
        started.elapsed()
    );
    assert!(!text.contains("not-the-password") && !text.contains(PASSWORD));

    // No credentials on an --auth server: `ping` alone would succeed, so connect must
    // refuse up front with a clear authentication error.
    let anonymous = client::connect(&container.config(), None, None).await;
    assert!(
        matches!(anonymous, Err(CoreError::AuthFailed)),
        "{anonymous:?}"
    );

    // A store built on a client that skipped the check still classifies server rejections.
    let raw = mongodb::Client::with_uri_str(container.config().uri)
        .await
        .unwrap();
    let store = MongoStore::new(raw);
    assert!(matches!(
        store.list_databases().await,
        Err(CoreError::Unauthorized)
    ));
}

fn runtime_with(
    cfg: &ConnectionConfig,
    password: &str,
) -> mongostudio_core::runtime::RuntimeHandle {
    let secrets = MemorySecretStore::new();
    secrets
        .set(&secret_key(cfg.id, SecretKind::Password), password)
        .unwrap();
    let config = RuntimeConfig {
        command_timeout: Duration::from_secs(20),
        ..RuntimeConfig::default()
    };
    spawn_runtime_with_config(Some(Arc::new(secrets)), None, no_repaint(), config)
}

#[tokio::test]
#[ignore = "needs docker; set MONGOSTUDIO_IT=1"]
async fn real_runtime_connects_lists_and_finds_with_a_keychain_password() {
    if !it_enabled("real_runtime_connects_lists_and_finds_with_a_keychain_password") {
        return;
    }
    let container = auth_container();
    let cfg = scram_config(&container);
    wait_ready(&cfg, Some(PASSWORD)).await;
    let mut rt = runtime_with(&cfg, PASSWORD);
    let wait = Duration::from_secs(20);

    rt.tx.send(Command::Connect(cfg.clone())).unwrap();
    let Event::Connected { epoch } = next_event(&mut rt.rx, wait, "connect").await else {
        panic!("expected Connected");
    };
    assert!(epoch >= 1);

    rt.tx.send(Command::ListDatabases).unwrap();
    match next_event(&mut rt.rx, wait, "databases").await {
        Event::Databases(names) => assert!(names.contains(&"admin".to_string()), "{names:?}"),
        other => panic!("expected Databases, got {other:?}"),
    }

    let ns = Namespace::new("admin", "system.version");
    let query = Query::default();
    rt.tx
        .send(Command::Find {
            ns: ns.clone(),
            query: query.clone(),
        })
        .unwrap();
    match next_event(&mut rt.rx, wait, "results").await {
        Event::Results {
            ns: got,
            query: echoed,
            page,
        } => {
            assert_eq!((got, echoed), (ns, query));
            assert!(!page.docs.is_empty());
        }
        other => panic!("expected Results, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs docker; set MONGOSTUDIO_IT=1"]
async fn real_runtime_reports_connect_failed_for_a_wrong_password_in_time() {
    if !it_enabled("real_runtime_reports_connect_failed_for_a_wrong_password_in_time") {
        return;
    }
    let container = auth_container();
    let cfg = scram_config(&container);
    wait_ready(&cfg, Some(PASSWORD)).await;
    let mut rt = runtime_with(&cfg, "wrong-password");

    let started = Instant::now();
    rt.tx.send(Command::Connect(cfg)).unwrap();
    match next_event(&mut rt.rx, BOUND, "connect failure").await {
        Event::ConnectFailed {
            error: CoreError::AuthFailed,
            ..
        } => {}
        other => panic!("expected ConnectFailed, got {other:?}"),
    }
    assert!(started.elapsed() < BOUND);
}

#[tokio::test]
#[ignore = "needs no docker, but is gated like the rest; set MONGOSTUDIO_IT=1"]
async fn unreachable_port_fails_within_the_bound() {
    if !it_enabled("unreachable_port_fails_within_the_bound") {
        return;
    }
    // Nothing listens on a just-released ephemeral port.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let cfg = ConnectionConfig::new("it", &format!("mongodb://127.0.0.1:{port}"));
    let started = Instant::now();
    let r = client::connect(&cfg, None, None).await;
    assert!(matches!(r, Err(CoreError::Database(_))), "{r:?}");
    assert!(started.elapsed() < BOUND, "took {:?}", started.elapsed());
}
