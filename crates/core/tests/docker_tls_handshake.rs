//! A TLS client against a plain-text server is a failed handshake, not an unreachable server;
//! a closed port stays unreachable.
//! Tier 2 (docker): own `cb-it-*` container. Run with `scripts/container-tests.sh`.

mod support;

use std::time::Duration;

use curlybrace_core::client::{self, SERVER_SELECTION_TIMEOUT};
use curlybrace_core::error::CoreError;
use support::{Container, config, docker_enabled, wait_ready};

const BOUND: Duration = Duration::from_secs(SERVER_SELECTION_TIMEOUT.as_secs() + 3);

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn tls_against_a_plain_text_server_is_a_handshake_failure() {
    if !docker_enabled("tls_against_a_plain_text_server_is_a_handshake_failure") {
        return;
    }
    let container = Container::start_mongo(&[], &[]);
    let plain = config(&container);
    wait_ready(&plain, None).await;

    let mut tls = plain.clone();
    tls.tls.enabled = true;
    tls.tls.allow_invalid_certs = true;
    let outcome = tokio::time::timeout(BOUND, client::connect(&tls, None, None))
        .await
        .expect("the handshake failure is reported within the selection bound");
    let Err(err) = outcome else {
        panic!("a TLS client must not connect to a plain-text server");
    };
    assert_eq!(err, CoreError::TlsHandshake, "{err:?}");
}

#[tokio::test]
#[ignore = "needs docker; set CURLYBRACE_IT=1"]
async fn a_closed_port_is_still_unreachable() {
    if !docker_enabled("a_closed_port_is_still_unreachable") {
        return;
    }
    let container = Container::start_mongo_restartable(&[], &[]);
    let cfg = config(&container);
    wait_ready(&cfg, None).await;
    container.stop();

    for tls in [false, true] {
        let mut cfg = cfg.clone();
        cfg.tls.enabled = tls;
        cfg.tls.allow_invalid_certs = tls;
        let outcome = tokio::time::timeout(BOUND, client::connect(&cfg, None, None))
            .await
            .expect("the failure is reported within the selection bound");
        let Err(err) = outcome else {
            panic!("a stopped server must not connect");
        };
        assert_eq!(err, CoreError::Unreachable, "tls {tls}: {err:?}");
    }
}
