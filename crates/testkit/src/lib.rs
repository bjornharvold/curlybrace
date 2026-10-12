//! Shared test support: env gates and `ms-it-*` docker containers.
//!
//! Containers are driven through the `docker` CLI and owned by RAII guards whose `Drop` runs
//! `docker rm -f -v` on the exact container name, so they are removed even when an assertion
//! panics. Every container is named `ms-it-*`; containers this code did not create are never
//! stopped, removed, executed in or reused. This crate does not depend on the driver or on
//! `mongostudio-core`: callers receive a URI/host/port and build their own config.
//!
//! Gates print why a test was skipped (never silent):
//! * `MONGOSTUDIO_IT=1` enables container/server backed tests ([`it_enabled`]).
//! * `MONGOSTUDIO_IT_MONGO_URI` points plain-server tests at an existing server instead of a
//!   container ([`plain_server`]); tests that need container control skip then
//!   ([`it_enabled_with_docker`]).
//! * `MONGOSTUDIO_SNAPSHOTS=1` enables the GPU screenshot tests ([`snapshots_enabled`]).

mod container;
mod gate;
mod poll;
mod ssh;

pub use container::{Container, MONGO_IMAGE, MongoServer, docker, free_port, plain_server};
pub use gate::{
    ENV_EXTERNAL_URI, ENV_IT, ENV_SNAPSHOTS, enabled_by, external_uri, it_enabled,
    it_enabled_with_docker, snapshots_enabled,
};
pub use poll::poll;
pub use ssh::{SSH_PASSWORD, SSH_USER, SshStack};
