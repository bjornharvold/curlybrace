//! Shared test support: env gates and `cb-it-*` docker containers.
//!
//! Containers are driven through the `docker` CLI and owned by RAII guards whose `Drop` runs
//! `docker rm -f -v` on the exact container name, so they are removed even when an assertion
//! panics. Every container is named `cb-it-*`; containers this code did not create are never
//! stopped, removed, executed in or reused. This crate does not depend on the driver or on
//! `curlybrace-core`: callers receive a URI/host/port and build their own config.
//!
//! Two local tiers, never run on GitHub:
//! * tier 1 "local server": needs only a MongoDB already running ([`local_uri`], default
//!   `mongodb://127.0.0.1:27017`, override `CURLYBRACE_IT_MONGODB_URI`); never starts containers;
//!   writes only to throwaway `cbit_*` databases ([`unique_db_name`]).
//! * tier 2 "docker": creates its own `cb-it-*` containers ([`Container`], [`SshStack`]).
//!
//! Gates print why a test was skipped (never silent):
//! * `CURLYBRACE_IT=1` enables both tiers ([`local_enabled`], [`docker_enabled`]).
//! * `CURLYBRACE_SNAPSHOTS=1` enables the GPU screenshot tests ([`snapshots_enabled`]).

mod container;
mod gate;
mod local;
mod poll;
mod ssh;

pub use container::{Container, MONGO_IMAGE, MongoServer, docker};
pub use gate::{
    DEFAULT_LOCAL_URI, ENV_IT, ENV_LOCAL_URI, ENV_SNAPSHOTS, docker_enabled, enabled_by,
    local_enabled, local_uri, snapshots_enabled, unreachable_message,
};
pub use local::{local_server, unique_db_name};
pub use poll::{poll, poll_with};
pub use ssh::{SSH_PASSWORD, SSH_USER, SshStack};
