# Testing

There are four levels. Only the first one runs on GitHub, and GitHub never needs a MongoDB
database.

| Level | What | Needs | Run |
|-------|------|-------|-----|
| Unit | everything default, offline, fast | nothing | `cargo test --workspace` |
| Tier 1 "local server" | CRUD flows and the click-driven UI flows against a real MongoDB | a MongoDB server already running locally (no docker) | `scripts/integration-tests.sh` |
| Tier 2 "docker" | `--auth`, stopping/restarting the server, the SSH tunnel stack | docker (the tests create their own containers) | `scripts/container-tests.sh` |
| Screenshots | fixed in-memory states rendered with wgpu | a GPU (Metal); no database | `MONGOSTUDIO_SNAPSHOTS=1 cargo test -p mongostudio-ui --test snapshots -- --include-ignored --nocapture` |

Tiers 1 and 2 and the screenshots are `#[ignore]`d and additionally gated by an environment
variable. When a gate is off the test returns early and prints `SKIPPED <test>: set ... to run`
(visible with `--nocapture`), so a skip is never silent. When the gate is on but the needed server is
unreachable the test FAILS with instructions; it never skips.

## Environment variables

| Variable | Effect |
|----------|--------|
| `MONGOSTUDIO_IT=1` | Enables tier 1 and tier 2 (the scripts set it). |
| `MONGOSTUDIO_IT_MONGO_URI` | Tier 1 only. Server to connect to; default `mongodb://127.0.0.1:27017`. |
| `MONGOSTUDIO_SNAPSHOTS=1` | Enables the screenshot tests. |
| `UPDATE_SNAPSHOTS=1` | Rewrites the committed PNGs instead of comparing. |

## Unit tests (default, offline, fast)

    cargo test --workspace

This is what GitHub runs (`check` job: fmt, clippy, unit tests). It never needs docker, a database
or a GPU.

## The GitHub guard

    scripts/check-no-db-in-ci.sh
    scripts/test-check-no-db-in-ci.sh      # tests for the guard, run against fixture trees

The `check` job runs both. The guard scans every file under `.github/workflows` and
`.github/actions` (workflows and composite actions), case-insensitively, and FAILS on the
integration gates (`MONGOSTUDIO_IT`), the runner scripts (`integration-tests`, `container-tests`),
`docker run` / `docker compose`, testcontainers, GitHub `services:` containers, MongoDB
images/installs/binaries (`mongo:`, `mongodb`, `mongod`, `mongosh`, `brew install mongo`) and
`--include-ignored` / `--ignored`. The one exception is the database-free screenshot job, which must
be a single `--test snapshots` target. Issue/PR templates and `dependabot.yml` cannot run anything
and are out of scope (they legitimately mention MongoDB). The script lives outside the scanned
directories, so it does not match itself.

## Tier 1: local server tests

    scripts/integration-tests.sh
    MONGOSTUDIO_IT_MONGO_URI=mongodb://127.0.0.1:27018 scripts/integration-tests.sh

Targets (select one with `cargo test --test <name>`, with `MONGOSTUDIO_IT=1` and
`--include-ignored --nocapture`):

- `mongostudio-core` `local_mongo_crud` - listing, paging (50/page, 100/101/120 docs),
  sort/projection/filter, replace/delete for every `_id` kind, `DocumentService`, exotic BSON.
- `mongostudio-ui` `local_e2e_flows` - the click-driven flows, see below.

The script checks that the server answers on TCP (no `mongosh` needed), sets `MONGOSTUDIO_IT=1` and
runs both targets. **Start the server yourself**, for example
`mongod --dbpath "$(mktemp -d)" --bind_ip 127.0.0.1 --port 27017` or
`docker run -d --rm -p 127.0.0.1:27017:27017 mongo:7`. These tests never start containers.

**These tests WRITE to the server they connect to.** Every test creates and uses only its own
throwaway database named `msit_<id>` (never any other database), drops it when it finishes, and
only ever looks for its own databases and collections in listings, so a server that also holds
other data is not disturbed. A hard kill (SIGKILL) can leak an `msit_*` database. Point the tests
at a throwaway server, never at production. Nothing sweeps other `msit_*` databases automatically;
to clean leftovers by hand:

    mongosh "mongodb://127.0.0.1:27017" --quiet --eval \
      'db.adminCommand({listDatabases:1}).databases.filter(d=>d.name.startsWith("msit_")).forEach(d=>db.getSiblingDB(d.name).dropDatabase())'

### UI click-driven flows

`crates/ui/tests/local_e2e_flows.rs` launches the REAL `MongoStudioApp` and the REAL runtime (a
Tokio runtime kept alive by the fixture), a `MemorySecretStore` (never the Keychain) and a temp
connections file, and drives it with `egui_kittest` through the accessibility tree: widgets are
found by their label (looked up from `en.json` through `I18n`, not hardcoded), clicked,
right-clicked and typed into. Seeding and verification use the `mongodb` driver, so each step
asserts both what the screen shows and what the database holds. Waiting for the async runtime uses
`wait_until(harness, what, timeout, cond)`, which steps frames with short real-time pauses and FAILS
with the app state at the deadline (no fixed sleeps as assertions).

Flows: add a connection through the form and browse it; find bar filter/sort/projection and bad
JSON; Tree/Table/JSON; edit, clone, delete with confirmation; projected pages are read-only for
edit/clone; deleting a saved connection (and its secrets) asks first. Text boxes are reachable by
their caption because the views link caption and box with `labelled_by`. The sidebar only draws rows
inside its viewport, so the helpers scroll it with the mouse wheel until the wanted row exists (a
developer's server may hold many databases); `tests/sidebar_scroll.rs` proves this without a database. Debugging a flow:
`support::screen::dump(&harness)` prints one line per accessibility node.

## Tier 2: docker tests

    scripts/container-tests.sh
    scripts/container-tests.sh -- --test-threads=2

Targets: `mongostudio-core` `docker_mongo_auth`, `docker_runtime_failures`, `docker_ssh_tunnel`, and
`mongostudio-ui` `docker_e2e_failures` (wrong password against an `--auth` server; server stopped
mid-session, then restarted). The script checks docker is available, sets `MONGOSTUDIO_IT=1`, runs
the targets, then lists `ms-it-*` leftovers and prints the cleanup command. Each test starts several
containers at once, so the script defaults to `--test-threads=4`; lower it on a small machine
(pass `-- --test-threads=N`). Manual only; never in CI.

### Containers and cleanup

Containers are driven through the `docker` CLI (no testcontainers dependency) and are always named
`ms-it-*`: `mongo:7` on a loopback port that docker assigns (`-p 127.0.0.1::27017`, read back with
`docker port`; the stop/start test uses a fixed port because docker may reassign the port on
restart), and for SSH an `lscr.io/linuxserver/openssh-server` + `mongo:7` pair on a private
`ms-it-net-*` network. Each is owned by an RAII guard (`Container`, `SshStack`) whose `Drop` runs
`docker rm -f -v` on the exact name (and `docker network rm`), so cleanup happens on assertion
failure too. Only a hard kill of the test process can leak one:

    docker ps -a --filter 'name=^ms-it-' -q | xargs docker rm -f -v

**Never touch containers you did not create.** Tests, scripts and cleanup must not stop, remove,
exec into or reuse containers they did not start; the anchored `name=^ms-it-` filter deliberately
matches only the test containers, and guards remove by exact name only. This applies to your own
containers (a local dev database, for instance) too. While developing, run tier 1 against a server
you start yourself, not against a database you care about.

## Screenshot tests

    MONGOSTUDIO_SNAPSHOTS=1 cargo test -p mongostudio-ui --test snapshots -- --include-ignored --nocapture
    MONGOSTUDIO_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p mongostudio-ui --test snapshots -- --include-ignored --nocapture

`crates/ui/tests/snapshots.rs` renders fixed in-memory states (no database; the landing page's
sample people) with `egui_kittest` + wgpu (Metal on macOS) at 1000x700 and 1.0 pixels per point and
compares them with the PNGs in `crates/ui/tests/snapshots/`. The app under test uses a temp
connections file and known_hosts, never the developer's. Update the PNGs only deliberately and
review the diff in the pull request. A mismatch leaves `<name>.new.png` and `<name>.diff.png`
next to the baseline (git-ignored).

Up to 100 differing pixels per image are tolerated (`PIXEL_THRESHOLD`) so text anti-aliasing
differences do not fail the test; a one-word label change differs by over 1,300 pixels. Pixel output
can still drift between GPUs and drivers: if the baselines were made on a different machine than the
one that runs them, expect to regenerate them there.

## CI

`.github/workflows/ci.yml` runs:

- `check` (required): the policy guard, `cargo fmt --check`, clippy, `cargo test --workspace`.
- `ui-snapshots` (non-blocking, `continue-on-error`, 30 minute timeout): the screenshot tests on
  `macos-14`; on failure the `*.new.png` and `*.diff.png` files are uploaded as an artifact. Whether
  the runners expose a usable Metal device is not guaranteed, which is why it is not required.

Nothing on GitHub starts a database. Tier 1 and tier 2 are run by hand with the scripts above.

## Coverage

    cargo install cargo-llvm-cov --locked && rustup component add llvm-tools-preview
    cargo llvm-cov -p mongostudio-core --features test-support --fail-under-lines 80
    cargo llvm-cov -p mongostudio-ui          # reported, no gate

The gate is on lines. Nothing is excluded from the measurement. Do not combine `MONGOSTUDIO_IT=1`
with `--include-ignored` over the whole workspace: that also runs the docker tier (and needs both a
local server and docker). Use `scripts/integration-tests.sh` and `scripts/container-tests.sh`
instead.
