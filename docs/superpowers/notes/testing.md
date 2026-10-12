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
- `support/mod.rs` - driver helpers (`wait_ready`, `plain_mongo`, `next_event`) on top of the shared
  `mongostudio-testkit` crate (see below).

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
matches only the test containers, and guards remove by exact name only. This applies to your own
containers (for example a local dev database) too: never point cleanup at them.

## Shared test support: `crates/testkit`

`mongostudio-testkit` (`publish = false`, dev-only, no dependency on core or the driver) holds
what core and UI tests share: the env gates (each prints `SKIPPED <test>: ...` when it says no),
the RAII `Container` (unique `ms-it-<uuid>` name, `docker rm -f -v` on drop), `SshStack`,
`plain_server()` and the bounded `poll` helper.

Environment variables:

| Variable | Effect |
|----------|--------|
| `MONGOSTUDIO_IT=1` | Enables container/server backed tests (core integration, UI e2e). |
| `MONGOSTUDIO_IT_MONGO_URI` | e.g. `mongodb://127.0.0.1:27017`. UI e2e tests that need only a plain server use it instead of starting a container (CI macOS runners have no docker). Each test uses a unique `msit_*` database and drops it. Tests that need container control (`--auth`, stop/start) still require docker and skip with a printed reason when it is set. Core's integration tests always start their own containers (several assert exact database lists). |
| `MONGOSTUDIO_SNAPSHOTS=1` | Enables the GPU screenshot tests. |
| `UPDATE_SNAPSHOTS=1` | Rewrites the committed PNGs instead of comparing. |

## UI end-to-end tests (click-driven)

    MONGOSTUDIO_IT=1 cargo test -p mongostudio-ui --test e2e_flows -- --include-ignored
    # against an existing server (no docker for the plain-server flows):
    MONGOSTUDIO_IT=1 MONGOSTUDIO_IT_MONGO_URI=mongodb://127.0.0.1:27017 \
      cargo test -p mongostudio-ui --test e2e_flows -- --include-ignored

`crates/ui/tests/e2e_flows.rs` launches the REAL `MongoStudioApp` and the REAL runtime (a
Tokio runtime kept alive by the test fixture), a `MemorySecretStore` (never the Keychain) and a
temp connections file, and drives it with `egui_kittest` through the accessibility tree: widgets are
found by their label (looked up from `en.json` through `I18n`, not hardcoded), clicked,
right-clicked and typed into. Seeding and verification use the `mongodb` driver, so every step
asserts both what the screen shows and what the database holds. Waiting for the async runtime uses
`wait_until(harness, what, timeout, cond)`, which steps frames with short real-time pauses and
FAILS with the app state at the deadline (no fixed sleeps as assertions).

Flows: add a connection through the form and browse it; find bar filter/sort/projection and bad
JSON; Tree/Table/JSON; edit, clone, delete with confirmation; projected pages are read-only for
edit/clone; deleting a saved connection (and its secrets) asks first; wrong password (`--auth`
container); server stopped mid-session and restart. Text boxes are reachable by their caption
because the views link caption and box with `labelled_by`.

Debugging a flow: `support::screen::dump(&harness)` prints one line per accessibility node.

## Screenshot tests

    MONGOSTUDIO_SNAPSHOTS=1 cargo test -p mongostudio-ui --test snapshots -- --include-ignored
    MONGOSTUDIO_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p mongostudio-ui --test snapshots -- --include-ignored

`crates/ui/tests/snapshots.rs` renders fixed in-memory states (no database; the landing page's
sample people) with `egui_kittest` + wgpu (Metal on macOS) at 1000x700 and 1.0 pixels per point and
compares them with the PNGs in `crates/ui/tests/snapshots/`. Update them only deliberately and
review the PNG diff in the pull request. A mismatch leaves `<name>.new.png` and `<name>.diff.png`
next to the baseline (git-ignored). Pixel output can differ between GPUs and drivers: if the
baselines were produced on a different machine than CI, expect to regenerate them there.

## CI

`.github/workflows/ci.yml` has two extra jobs on `macos-14`, both `continue-on-error: true`
(non-blocking) until the first runs show whether they are reliable:

- `ui-e2e`: installs MongoDB with brew, starts `mongod` on a free port with a temp dbpath, sets
  `MONGOSTUDIO_IT_MONGO_URI` and runs `e2e_flows`; the docker-only flows skip with a printed reason.
- `ui-snapshots`: runs the snapshot tests with `MONGOSTUDIO_SNAPSHOTS=1`. Whether the runners expose
  a Metal device is unverified; if not, run snapshots locally.

The default `check` job is unchanged: container and GPU tests are `#[ignore]`d, so
`cargo test --workspace` stays fast and needs neither docker nor a GPU.

## Coverage

    cargo install cargo-llvm-cov --locked && rustup component add llvm-tools-preview
    cargo llvm-cov -p mongostudio-core --features test-support --fail-under-lines 80
    MONGOSTUDIO_IT=1 cargo llvm-cov -p mongostudio-core --features test-support \
      --fail-under-lines 80 -- --include-ignored --skip keyring_round_trip
    cargo llvm-cov -p mongostudio-ui          # reported, no gate

The gate is on lines. Nothing is excluded from the measurement.
