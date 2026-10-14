//! SCRAM authentication against `mongo:7 --auth`, directly and through the
//! real runtime. Tier 2 (docker): own `cb-it-*` container; run with `scripts/container-tests.sh`.

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use bson::doc;
use curlybrace_core::client::{self, SERVER_SELECTION_TIMEOUT};
use curlybrace_core::connection::{AuthMechanism, ConnectionConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::mongo_store::MongoStore;
use curlybrace_core::runtime::{
    Command, Event, RuntimeConfig, TestSecrets, spawn_runtime_with_config,
};
use curlybrace_core::secrets::{
    MemorySecretStore, SecretKind, SecretStore, SecretString, secret_key,
};
use curlybrace_core::store::{DocumentStore, Namespace, Query};
use support::{Container, docker_enabled, next_event, no_repaint, wait_ready};

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
    let mut cfg = support::config(container);
    cfg.auth = AuthMechanism::ScramSha256;
    cfg.username = Some(USER.into());
    cfg.auth_source = Some("admin".into());
    cfg
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn scram_sha256_succeeds_and_failures_are_typed_and_bounded() {
    if !docker_enabled("scram_sha256_succeeds_and_failures_are_typed_and_bounded") {
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
    let anonymous = client::connect(&support::config(&container), None, None).await;
    assert!(
        matches!(anonymous, Err(CoreError::AuthFailed)),
        "{anonymous:?}"
    );

    // A store built on a client that skipped the check still classifies server rejections.
    let raw = mongodb::Client::with_uri_str(support::config(&container).uri)
        .await
        .unwrap();
    let store = MongoStore::new(raw);
    assert!(matches!(
        store.list_databases().await,
        Err(CoreError::Unauthorized)
    ));
}

fn runtime_with(cfg: &ConnectionConfig, password: &str) -> curlybrace_core::runtime::RuntimeHandle {
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
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn real_runtime_connects_lists_and_finds_with_a_keychain_password() {
    if !docker_enabled("real_runtime_connects_lists_and_finds_with_a_keychain_password") {
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
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn real_runtime_reports_connect_failed_for_a_wrong_password_in_time() {
    if !docker_enabled("real_runtime_reports_connect_failed_for_a_wrong_password_in_time") {
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
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn test_connection_reports_the_version_or_a_typed_auth_failure_and_installs_nothing() {
    if !docker_enabled(
        "test_connection_reports_the_version_or_a_typed_auth_failure_and_installs_nothing",
    ) {
        return;
    }
    let container = auth_container();
    let cfg = scram_config(&container);
    wait_ready(&cfg, Some(PASSWORD)).await;
    // No stored secrets: the typed ones are the only credentials.
    let mut rt = spawn_runtime_with_config(None, None, no_repaint(), RuntimeConfig::default());
    let wait = Duration::from_secs(20);
    let test = |request_id, password: &str| Command::TestConnection {
        request_id,
        cfg: cfg.clone(),
        secrets: TestSecrets {
            password: Some(SecretString::new(password)),
            ssh_secret: None,
        },
    };

    rt.tx.send(test(1, PASSWORD)).unwrap();
    match next_event(&mut rt.rx, wait, "test with the right password").await {
        Event::TestConnection {
            request_id: 1,
            outcome: Ok(report),
        } => {
            assert!(report.authenticated && !report.tls);
            assert!(
                report.server_version.is_some_and(|v| !v.is_empty()),
                "buildInfo reports a version"
            );
        }
        other => panic!("expected a successful test, got {other:?}"),
    }

    rt.tx.send(test(2, "not-the-password")).unwrap();
    match next_event(&mut rt.rx, BOUND, "test with the wrong password").await {
        Event::TestConnection {
            request_id: 2,
            outcome: Err(CoreError::AuthFailed),
        } => {}
        other => panic!("expected AuthFailed, got {other:?}"),
    }

    // A test installs nothing.
    rt.tx.send(Command::ListDatabases).unwrap();
    assert!(matches!(
        next_event(&mut rt.rx, wait, "list after tests").await,
        Event::Failed {
            context: "list_databases",
            ..
        }
    ));
}
