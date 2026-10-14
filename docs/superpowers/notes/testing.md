# Testing

There are four levels. Only the first one runs on GitHub, and GitHub never needs a MongoDB
database.

| Level | What | Needs | Run |
|-------|------|-------|-----|
| Unit | everything default, offline, fast | nothing | `cargo test --workspace` |
| Tier 1 "local server" | CRUD flows and the click-driven UI flows against a real MongoDB | a MongoDB server already running locally (no docker) | `scripts/integration-tests.sh` |
| Tier 2 "docker" | `--auth`, stopping/restarting the server, the SSH tunnel stack | docker (the tests create their own containers) | `scripts/container-tests.sh` |
| Screenshots | fixed in-memory states rendered with wgpu | a GPU (Metal); no database | `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture` |

Tiers 1 and 2 and the screenshots are `#[ignore]`d and additionally gated by an environment
variable. When a gate is off the test returns early and prints `SKIPPED <test>: set ... to run`
(visible with `--nocapture`), so a skip is never silent. When the gate is on but the needed server is
unreachable the test FAILS with instructions; it never skips.

## Environment variables

| Variable | Effect |
|----------|--------|
| `CURLYBRACE_IT=1` | Enables tier 1 and tier 2 (the scripts set it). |
| `CURLYBRACE_IT_MONGODB_URI` | Tier 1 only. Server to connect to; default `mongodb://127.0.0.1:27017`. |
| `CURLYBRACE_IT_MONGOSH_CONTAINER` | Export tier-1 tests only. Name of YOUR OWN `cb-it-dev-*` container whose `mongosh` runs the generated scripts (`docker exec -i`). Unset = the `mongosh` tests skip with a message. Names not starting with `cb-it-dev-` are refused (panic), and the container's published port 27017 must equal the host:port of `CURLYBRACE_IT_MONGODB_URI`. |
| `CURLYBRACE_SNAPSHOTS=1` | Enables the screenshot tests. |
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
integration gates (`CURLYBRACE_IT`), the runner scripts (`integration-tests`, `container-tests`),
`docker run` / `docker compose`, testcontainers, GitHub `services:` containers, MongoDB
images/installs/binaries (`mongo:`, `mongodb`, `mongod`, `mongosh`, `brew install mongo`) and
`--include-ignored` / `--ignored`. The one exception is the database-free screenshot job, which must
be a single `--test snapshots` target. Issue/PR templates and `dependabot.yml` cannot run anything
and are out of scope (they legitimately mention MongoDB). The script lives outside the scanned
directories, so it does not match itself.

## Tier 1: local server tests

    scripts/integration-tests.sh
    CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27018 scripts/integration-tests.sh

Targets (select one with `cargo test --test <name>`, with `CURLYBRACE_IT=1` and
`--include-ignored --nocapture`):

- `curlybrace-core` `local_mongo_crud` - listing, paging (50/page, 100/101/120 docs),
  sort/projection/filter, replace/delete for every `_id` kind, `DocumentService`, exotic BSON.
- `curlybrace-core` `local_compare_session` and `local_compare_apply` - a runtime compare of two real databases, and the apply
  path: `MongoStore::bulk_write` plus end-to-end apply flows (`_id` and business-key convergence, stale documents,
  `IdPolicy::Generate`, a unique-index violation, a view destination, cancelling 5,000 inserts). They WRITE to
  `cbit_*` databases, so run them against a throwaway server of your own, for example
  `docker run -d --rm --name cb-it-dev-2a -p 127.0.0.1:27119:27017 mongo:7`, then
  `CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27119 cargo test -p curlybrace-core --test local_compare_apply -- --include-ignored --nocapture`
  and `docker rm -f cb-it-dev-2a` afterwards.
- `curlybrace-ui` `local_e2e_flows` - the click-driven flows, see below.
- `curlybrace-ui` `local_compare_flows` - the Compare workspace: single pair, a database-level compare (Add all same-named, a duplicate key and Find duplicate keys, Re-run) and cancelling a multi-pair run. It drives the reducer through `CurlybraceApp::dispatch` against real `cbit_*` databases.
- `curlybrace-ui` `local_compare_apply_flows` - the click-driven APPLY flows of slice 2b (select rows or "Select all in tab", Copy/Delete, Review changes, the typed confirmation, Apply, the result panel): copy a whole collection to an empty target and re-run, delete only ticked extras behind the typed name, replace and converge, a document changed by a second client (stale), a unique-index collision, cancelling a 50,000-document apply, two pairs in one apply. Like `local_compare_apply` it WRITES to `cbit_*` databases, so `scripts/integration-tests.sh` does not run it: run it against your own container (it inserts about 50 MB in one test):

      docker run -d --rm --name cb-it-dev-2b -p 127.0.0.1:27120:27017 mongo:7
      until nc -z 127.0.0.1 27120; do sleep 1; done
      CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27120 \
        cargo test -p curlybrace-ui --test local_compare_apply_flows --locked -- --include-ignored --nocapture
      docker rm -f cb-it-dev-2b

  The gesture helpers (`open_tab`, `tick_rows`, `select_all_in_tab`, `click_action`, `open_review`,
  `type_confirmation`, `apply_enabled`, `confirm_and_wait`, `rerun`) live in `tests/support/compare.rs`.
- `curlybrace-ui` `local_compare_field_flows` - the click-driven FIELD-LEVEL apply flows of slice 2c (open a document's detail, tick fields by path, "Copy fields to target/source", Review changes, Apply, the Finished panel): one field of two, two fields in one apply and the re-run, removing a field, a nested field with an array element by index, a document changed after the compare (stale), a dotted field name (disabled), fields and whole documents in one apply, a field scope that is never exceeded. It WRITES to `cbit_*` databases and skips unless BOTH `CURLYBRACE_IT=1` and an explicit `CURLYBRACE_IT_MONGODB_URI` are set. Use your own container (it needs mongo:7 or later, which stores field names with dots):

      docker run -d --rm --name cb-it-dev-2c -p 127.0.0.1:27121:27017 mongo:7
      until nc -z 127.0.0.1 27121; do sleep 1; done
      CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27121 \
        cargo test -p curlybrace-ui --test local_compare_field_flows --locked -- --include-ignored --nocapture
      docker rm -f cb-it-dev-2c

  Its gesture helpers (`open_detail`, `tick_fields`, `copy_fields`, `apply_ticked_fields`, `field_disabled`, `field_ticked`, `has_field`) are in `tests/support/compare.rs`.

- `curlybrace-core` `local_compare_export` and `local_compare_export_files`, `curlybrace-ui` `local_compare_export_flows` - the slice 3
  EXPORT flows. `local_compare_export` runs the exported mongosh script with the REAL `mongosh` against the server it
  seeded: the script must leave the target equal to an in-process apply, a document of every BSON type must survive
  byte for byte, field by field (documents whose key order JavaScript would change, e.g. a nested `"2"` after `"b"`, are skipped
  and checked as such) (Int32/Int64/Double incl. -0/NaN/Infinity, Decimal128, dates before 1970 and after 9999, ObjectId, BinData
  and UUID, regex with flags, Timestamp, MinKey/MaxKey, Code with scope, Symbol; Undefined and DBPointer are checked as
  skipped), hostile names and values change nothing but the intended collection (a sentinel collection and a sentinel
  database stay), field operations touch only the chosen fields, stale operations are not written, and an unfinished
  script (`quit(1)` first line, or cut at several offsets) changes nothing. `local_compare_export_files` needs no
  `mongosh`: the JSON plan, the CSV report (parsed with a reader written in the test), the report with paths (no
  values), cancelling an export of 40,000 operations (the partial file is removed) and the one-destination-side rule.
  `local_compare_export_flows` drives the REAL app with a fake `FilePicker` (`tests/support/export_flow.rs`, the
  native dialog is never opened): it exports a plan (script and JSON) from the pending list and a report from the
  overview into a temp directory, checks the files and the finished panel, that both collections are unchanged, and
  that the clipboard report is copied exactly once. They WRITE to `cbit_*` databases (the exports themselves write
  none), so use your own container. The container is also the one the generated scripts run in:

      docker run -d --rm --name cb-it-dev-3 -p 127.0.0.1:27122:27017 mongo:7
      until nc -z 127.0.0.1 27122; do sleep 1; done
      export CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27122 \
             CURLYBRACE_IT_MONGOSH_CONTAINER=cb-it-dev-3
      cargo test -p curlybrace-core --test local_compare_export --test local_compare_export_files --locked -- --include-ignored --nocapture
      cargo test -p curlybrace-ui --test local_compare_export_flows --locked -- --include-ignored --nocapture
      docker rm -f cb-it-dev-3

  How a script is run (`tests/support/mongosh.rs`; the 27017 below is INSIDE the container, not the host's):

      docker exec -i cb-it-dev-3 mongosh --quiet "mongodb://127.0.0.1:27017/<cbit_db>?directConnection=true" < script.js

  The helper never execs into another container: the name must start with `cb-it-dev-` (letters, digits, `-`, `_`), the
  default database must be a `cbit_*` name, and the container must publish 27017 on the host:port of
  `CURLYBRACE_IT_MONGODB_URI`. `mongosh` reading a script from stdin is a REPL and would keep going after a failed
  statement and exit 0; the exported script therefore wraps each statement in `try`/`catch` and `quit(2)`, which a
  tier-1 test pins in stdin mode (exit code 2, no later statement). A cut statement is never run (in stdin mode the complete statements
  before the cut do run; the exit status can still be 0), so the tests judge by the database and printed errors, not only by the exit status.

The script checks that the server answers on TCP (no `mongosh` needed), sets `CURLYBRACE_IT=1` and
runs both targets. **Start the server yourself**, for example
`mongod --dbpath "$(mktemp -d)" --bind_ip 127.0.0.1 --port 27017` or
`docker run -d --rm -p 127.0.0.1:27017:27017 mongo:7`. These tests never start containers.

**These tests WRITE to the server they connect to.** Every test creates and uses only its own
throwaway database named `cbit_<id>` (never any other database), drops it when it finishes, and
only ever looks for its own databases and collections in listings, so a server that also holds
other data is not disturbed. A hard kill (SIGKILL) can leak an `cbit_*` database. Point the tests
at a throwaway server, never at production. Nothing sweeps other `cbit_*` databases automatically;
to clean leftovers by hand:

    mongosh "mongodb://127.0.0.1:27017" --quiet --eval \
      'db.adminCommand({listDatabases:1}).databases.filter(d=>d.name.startsWith("cbit_")).forEach(d=>db.getSiblingDB(d.name).dropDatabase())'

### UI click-driven flows

`crates/ui/tests/local_e2e_flows.rs` launches the REAL `CurlybraceApp` and the REAL runtime (a
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

Targets: `curlybrace-core` `docker_mongo_auth`, `docker_runtime_failures`, `docker_ssh_tunnel`, and
`curlybrace-ui` `docker_e2e_failures` (wrong password against an `--auth` server; server stopped
mid-session, then restarted). The script checks docker is available, sets `CURLYBRACE_IT=1`, runs
the targets, then lists `cb-it-*` leftovers and prints the cleanup command. Each test starts several
containers at once, so the script defaults to `--test-threads=4`; lower it on a small machine
(pass `-- --test-threads=N`). Manual only; never in CI.

### Containers and cleanup

Containers are driven through the `docker` CLI (no testcontainers dependency) and are always named
`cb-it-*`: `mongo:7` on a loopback port that docker assigns (`-p 127.0.0.1::27017`, read back with
`docker port`; the stop/start test uses a fixed port because docker may reassign the port on
restart), and for SSH an `lscr.io/linuxserver/openssh-server` + `mongo:7` pair on a private
`cb-it-net-*` network. Each is owned by an RAII guard (`Container`, `SshStack`) whose `Drop` runs
`docker rm -f -v` on the exact name (and `docker network rm`), so cleanup happens on assertion
failure too. Only a hard kill of the test process can leak one:

    docker ps -a --filter 'name=^cb-it-' -q | xargs docker rm -f -v

**Never touch containers you did not create.** Tests, scripts and cleanup must not stop, remove,
exec into or reuse containers they did not start; the anchored `name=^cb-it-` filter deliberately
matches only the test containers, and guards remove by exact name only. This applies to your own
containers (a local dev database, for instance) too. While developing, run tier 1 against a server
you start yourself, not against a database you care about.

## Screenshot tests

    CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture
    CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture

`crates/ui/tests/snapshots.rs` renders fixed in-memory states (the Compare screens included, among them `compare_pairs_setup`, `compare_overview_mixed` and `compare_pair_results`; their states live in `tests/support/compare_multi_states.rs`) (no database; the landing page's
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
    cargo llvm-cov -p curlybrace-core --features test-support --fail-under-lines 80
    cargo llvm-cov -p curlybrace-ui          # reported, no gate

The gate is on lines. Nothing is excluded from the measurement. Do not combine `CURLYBRACE_IT=1`
with `--include-ignored` over the whole workspace: that also runs the docker tier (and needs both a
local server and docker). Use `scripts/integration-tests.sh` and `scripts/container-tests.sh`
instead.
