# Testing

## Unit tests (default, offline, fast)

    cargo test --workspace

Container-backed tests are `#[ignore]`d, so this never needs docker or a network.

## Integration tests (docker)

    MONGOSTUDIO_IT=1 cargo test -p mongostudio-core -- --include-ignored --skip keyring_round_trip

`MONGOSTUDIO_IT=1` is required in addition to `--include-ignored`. Without it each
integration test returns early and prints `SKIPPED <test>: set MONGOSTUDIO_IT=1 ...`
(see `--nocapture`), so a skip is never silent. `--skip keyring_round_trip` avoids the
test that touches the real macOS Keychain.

Files in `crates/core/tests`:

- `mongo_crud.rs` - listing, paging (50/page, 100/101/120 docs), sort/projection/filter,
  replace/delete for every `_id` kind, `DocumentService`, exotic BSON round trip.
- `mongo_auth.rs` - SCRAM-SHA-256 against `mongo:7 --auth` (awkward password), wrong
  password and unreachable port bounds, the real runtime Connect/List/Find flow.
- `runtime_failures.rs` - stops the server mid-session; `Find` must yield `Failed`
  within the command timeout, then a fresh `Connect` recovers after restart.
- `ssh_integration.rs` - SSH tunnel + host-key policy against a real sshd.
- `support/mod.rs` - the container guards.

### Containers and cleanup

Containers are driven through the `docker` CLI (no testcontainers dependency) and
are always named `ms-it-*`: `mongo:7` on a random 127.0.0.1 port, and for SSH an
`lscr.io/linuxserver/openssh-server` + `mongo:7` pair on a private `ms-it-net-*`
network. Each is owned by an RAII guard (`Container`, `SshStack`) whose `Drop` runs
`docker rm -f -v` (and `docker network rm`), so cleanup happens on assertion failure
too. Only a hard kill of the test process can leak one: remove leftovers with
`docker ps -a --filter 'name=^ms-it-' -q | xargs docker rm -f`.

**Never touch containers you did not create.** Tests and cleanup must not stop, remove or
reuse containers they did not start; the anchored `name=^ms-it-` filter above deliberately
matches only the test containers.

## Coverage

    cargo install cargo-llvm-cov --locked && rustup component add llvm-tools-preview
    cargo llvm-cov -p mongostudio-core --features test-support --fail-under-lines 80
    MONGOSTUDIO_IT=1 cargo llvm-cov -p mongostudio-core --features test-support \
      --fail-under-lines 80 -- --include-ignored --skip keyring_round_trip
    cargo llvm-cov -p mongostudio-ui          # reported, no gate

The gate is on lines. Nothing is excluded from the measurement.
