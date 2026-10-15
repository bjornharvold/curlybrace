//! Tier 2 (docker): UI failure paths that need control of the server: wrong password (an
//! `--auth` container) and a server stopped mid-session. Each test starts its own `cb-it-*`
//! container and removes it on drop. Run with `scripts/container-tests.sh`.
mod support;

use curlybrace_core::connection::{AuthMechanism, ConnectionConfig};
use curlybrace_core::error::CoreError;
use curlybrace_core::secrets::{SecretKind, secret_key};
use curlybrace_testkit::{Container, MongoServer, docker_enabled};
use curlybrace_ui::app::state::{Banner, ConnStatus};
use egui_kittest::kittest::Queryable;
use support::actions::*;
use support::db::people;
use support::fixture::{Fixture, LOCAL_CONNECTION_NAME, Setup};
use support::wait::wait_until;

const PEOPLE: &str = "people";
const NAMES: [&str; 3] = ["Ada Lovelace", "Grace Hopper", "Alan Turing"];

/// The name of the enclosing test function (for the printed skip reason).
macro_rules! test_name {
    () => {{
        fn here() {}
        std::any::type_name_of_val(&here).trim_end_matches("::here")
    }};
}

/// The saved connection's name in the secured fixture (`auth_fixture`).
const SECURED_NAME: &str = "secured";

/// The accessible name of a connection's own disconnect button in the sidebar.
fn disconnect_label(f: &Fixture, name: &str) -> String {
    f.labels
        .t_with("sidebar.disconnect_named", &[("name", name)])
}

const ROOT: &str = "root";
const ROOT_PASSWORD: &str = "it-root-pw-1";

fn auth_fixture(test: &str, saved_password: &str) -> Option<Fixture> {
    if !docker_enabled(test) {
        return None;
    }
    let container = Container::start_mongo(
        &[
            format!("MONGO_INITDB_ROOT_USERNAME={ROOT}"),
            format!("MONGO_INITDB_ROOT_PASSWORD={ROOT_PASSWORD}"),
        ],
        &["--auth"],
    );
    container.wait_tcp();
    let uri = container.uri();
    let mut cfg = ConnectionConfig::new(SECURED_NAME, &uri);
    cfg.auth = AuthMechanism::ScramSha256;
    cfg.username = Some(ROOT.into());
    cfg.auth_source = Some("admin".into());
    let ready_uri = format!(
        "mongodb://{ROOT}:{ROOT_PASSWORD}@{}/?authSource=admin",
        &uri["mongodb://".len()..]
    );
    let secrets = vec![(
        secret_key(cfg.id, SecretKind::Password),
        saved_password.to_string(),
    )];
    let setup = Setup {
        ready_uri,
        saved: vec![cfg],
        secrets,
    };
    Some(Fixture::start(MongoServer::Owned(container), setup))
}

#[test]
#[ignore = "needs docker: set CURLYBRACE_IT=1"]
fn wrong_password_shows_the_auth_error_banner_in_time() {
    let Some(mut f) = auth_fixture(test_name!(), "definitely-wrong") else {
        return;
    };
    click_key(&mut f, "connections.connect");
    let headline = f.labels.i18n().failure("connect");
    let detail = f.labels.i18n().error(&CoreError::AuthFailed);
    wait_until(&mut f.harness, "auth failure banner", FAILURE_BOUND, |h| {
        h.query_all_by_label(&headline).next().is_some()
    });
    assert!(
        f.harness.query_all_by_label(&detail).next().is_some(),
        "localized auth error"
    );
    let state = f.harness.state().state();
    assert_eq!(state.status, ConnStatus::Failed);
    assert!(matches!(
        state.banner,
        Some(Banner::Failure {
            context: "connect",
            ..
        })
    ));
    assert!(
        f.harness
            .query_all_by_label(&f.labels.t("connections.connect"))
            .next()
            .is_some(),
        "back on the connections screen"
    );
}

#[test]
#[ignore = "needs docker: set CURLYBRACE_IT=1"]
fn the_right_password_connects_to_the_same_secured_server() {
    let Some(mut f) = auth_fixture(test_name!(), ROOT_PASSWORD) else {
        return;
    };
    click_key(&mut f, "connections.connect");
    wait_until(&mut f.harness, "connected", FAILURE_BOUND, |h| {
        h.state().state().status == ConnStatus::Connected
    });
    let disconnect = disconnect_label(&f, SECURED_NAME);
    wait_shows(&mut f, &disconnect);
    assert!(f.harness.state().state().banner.is_none());
}

#[test]
#[ignore = "needs docker: set CURLYBRACE_IT=1"]
fn a_stopped_server_fails_in_time_and_reconnects_after_restart() {
    let Some(mut f) = Fixture::docker_plain(test_name!()) else {
        return;
    };
    f.db.insert(PEOPLE, people());
    browse(&mut f, PEOPLE);
    let container = f.server.container().expect("owned container");

    container.stop();
    click_key(&mut f, "findbar.run");
    let headline = f.labels.i18n().failure("find");
    wait_until(&mut f.harness, "find failure banner", FAILURE_BOUND, |h| {
        h.query_all_by_label(&headline).next().is_some()
    });
    assert_names_on_screen(&f, &["Ada Lovelace"]);

    f.server.container().unwrap().start();
    support::fixture::wait_ready_blocking(&f);
    click_key(&mut f, "banner.dismiss");
    let disconnect = disconnect_label(&f, LOCAL_CONNECTION_NAME);
    f.click_text(&disconnect);
    let connect = f.labels.t("connections.connect");
    wait_shows(&mut f, &connect);
    browse(&mut f, PEOPLE);
    assert_names_on_screen(&f, &NAMES);
    assert!(f.harness.state().state().banner.is_none());
}
