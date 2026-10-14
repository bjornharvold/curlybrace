# Several Simultaneous Connections (Slice 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep several saved connections open at once: a session map in the runtime, one tree root per open connection with its own status and view, and a Connections workspace to open more.

**Architecture:** `Shared` keeps the connector, secrets, config and Compare registry; the store, tunnel, generation and `epoch_lock` move into a per-connection `Session` held in a map. Epoch numbers come from one global counter so they never repeat. In the UI the existing browsing fields of `AppState` stay as the **active session's view** and every other open session's view is parked in a map; `activated` and `with_session` swap views, so no reducer logic is duplicated.

**Tech Stack:** Rust workspace (`crates/core`, `crates/ui`, `crates/app`), egui/eframe, tokio, the `mongodb` driver, `uuid`.

**Spec:** `docs/superpowers/specs/2026-10-14-multiple-sessions-design.md` (binding; the plan argues from it). Issue #46.

## Global Constraints

- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical); report new keys. No Studio 3T wording or assets.
- Immutable style: functions consume a state and return the next one. Small files (200–400 lines, 800 max). Handle every error explicitly.
- TDD: write the failing test, run it red, implement, run it green. Real objects over mocks (`InMemoryStore`, `MemorySecretStore`, the fake connectors already in `crates/core/src/runtime/tests`).
- No URI, password, passphrase or pasted string in any log, event, banner, error text or `Debug` output. Host-key verification keeps failing closed.
- Conventional commits (`type(scope): lowercase imperative`), each ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Stage explicit paths, never `git add -A`.
- Gates before each commit that touches code: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `scripts/check-no-db-in-ci.sh`.
- Snapshot tests: `CURLYBRACE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored` (update with `UPDATE_SNAPSHOTS=1`); look at every changed PNG. CI runs them, so refresh every snapshot whose screen changes.
- Work on branch `feat/connection-manager-slice-2`. The target of the PR is `main` (the only base that exists); do not merge it without the maintainer.

## Review Focus

The inputs the spec implies but no happy-path test exercises. Each line has its test in the task named.

1. **Same namespace on two sessions** (both have `app.users`): `Results` for A must never replace B's page (Task 4).
2. **Late events of a closed session** (Disconnect, then an answer or a `Connected` arrives): dropped, never applied to another session, never resurrect the root (Task 4).
3. **Disconnect then quick Connect of the same connection**: the old session's stale answers cannot look current to the new one (Task 1, Task 4).
4. **The active session fails or is disconnected while others are open**: the neighbour becomes active; with none left the manager shows; nothing else is touched (Task 4).
5. **Two connections needing a host-key answer at once**: one prompt visible, the other queued, each answer reaches only the connection that asked (Task 5).
6. **A write for a session that is not open** fails closed with a banner and is never applied to another session (Task 1, Task 4).
7. **Delete and replace confirmations name the connection** (Task 7).

---

## File Structure

Core (`crates/core/src`):
- Create `runtime/session.rs`: `SessionId`, `Session`, `Topology` use (about 120 lines).
- Modify `runtime/mod.rs` (Shared, Command, Event, dispatch), `runtime/handlers.rs`, `runtime/tunneling.rs` (`ConnectionCurrent` takes a session), `runtime/compare/mod.rs` (only if it reads `Shared` fields that moved), `store.rs` (`Topology`, `DocumentStore::topology`), `mongo_store.rs` (`hello`), `lib.rs` (exports).
- Tests: migrate `runtime/epoch_tests.rs`, `runtime/tests/*.rs`, `runtime/tunnel_tests.rs`, `runtime/prompt_gate_tests.rs`, `runtime/test_connection_tests.rs`; add `runtime/session_tests.rs`.

UI (`crates/ui/src`):
- Create `app/session.rs` (SessionView, park/activate, `with_session`, `roots`; about 220 lines), `app/session_actions.rs` (connect/focus/disconnect/reconnect handlers; about 200 lines), `views/sidebar_roots.rs` (tree roots; about 250 lines), `app/session_tests.rs`.
- Modify `app/state.rs`, `app/actions.rs`, `app/reducer.rs`, `app/mod.rs`, `app/panels.rs`, `app/compare.rs` (default source), `app/banner.rs`, `views/sidebar.rs`, `views/connections.rs` (nothing but the connecting indicator), `viewmodel/sidebar.rs` (session-scoped action), `locales/en.json`, `tests/` (wiring, accessibility, snapshots).

---

### Task 1: Runtime session map

**Files:**
- Create: `crates/core/src/runtime/session.rs`, `crates/core/src/runtime/session_tests.rs`
- Modify: `crates/core/src/runtime/mod.rs`, `handlers.rs`, `tunneling.rs`, `compare/mod.rs` if needed, `crates/core/src/lib.rs`; the existing runtime tests listed above; `crates/ui/src/app/actions.rs`, `reducer.rs`, `mod.rs` and tests only as far as needed to compile and keep single-session behaviour.

**Interfaces:**
- Produces:
  - `pub type SessionId = uuid::Uuid;` and `pub const EMBEDDED_SESSION: SessionId = SessionId::nil();` in `curlybrace_core::runtime`.
  - `Command::Disconnect { session: SessionId }`; every data command gains `session: SessionId` as its first field (`ListDatabases { session }`, `ListCollections { session, db }`, `Find { session, ns, query }`, `Insert { session, ns, json }`, `Replace { session, ns, id, json }`, `Delete { session, ns, id }`, `Clone { session, ns, source }`). `Connect(cfg)` keeps its shape: the session is `cfg.id`.
  - Events: `Connected { session, epoch }`, `Disconnected { session, epoch }`, `ConnectFailed { session, epoch, error }`, `Databases { session, names }`, `Collections { session, db, names }`, `Results { session, ns, query, page }`, `Mutated { session, ns }`, `Failed { session, context, error }`. `Event::Compare` and `Event::TestConnection` are unchanged.
  - `Command::session(&self) -> Option<SessionId>` (None for Compare and TestConnection) and `Event::session(&self) -> Option<SessionId>`.
- Consumes: nothing from earlier tasks.

Design (see the spec, "Runtime"):

```rust
// crates/core/src/runtime/session.rs
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use crate::store::DocumentStore;
use super::tunneling::TunnelHandle;

pub type SessionId = uuid::Uuid;
pub const EMBEDDED_SESSION: SessionId = SessionId::nil();

/// One open connection: its store, tunnel and current epoch.
pub(crate) struct Session {
    store: RwLock<Option<Arc<dyn DocumentStore>>>,
    tunnel: Mutex<Option<Box<dyn TunnelHandle>>>,
    generation: AtomicU64,
    /// Orders epoch bumps against the final check-and-send of data answers (see `handlers::send_answer`).
    pub(super) epoch_lock: RwLock<()>,
}

impl Session {
    pub(super) fn new(generation: u64, store: Option<Arc<dyn DocumentStore>>) -> Self { /* fields */ }
    pub fn current_store(&self) -> Option<Arc<dyn DocumentStore>> { /* read, poison-tolerant */ }
    pub fn is_current(&self, generation: u64) -> bool { self.generation.load(Ordering::SeqCst) == generation }
    /// Moves to `generation` and drops the store and tunnel. Holds the WRITE side of `epoch_lock`.
    pub(super) fn begin(&self, generation: u64) { /* the body of today's Shared::begin_epoch, minus the counter */ }
    pub fn install_if_current(&self, generation: u64, store: Arc<dyn DocumentStore>, tunnel: Option<Box<dyn TunnelHandle>>) -> bool { /* today's body */ }
}
```

`Shared` replaces `store`, `tunnel`, `generation`, `epoch_lock` with `sessions: RwLock<HashMap<SessionId, Arc<Session>>>` and `next_epoch: AtomicU64`, with:

```rust
/// Gets or creates the session and starts its next epoch. Called only from the dispatcher
/// (single-threaded), so request order decides.
fn begin_session_epoch(&self, id: SessionId) -> (Arc<Session>, u64) {
    let generation = self.next_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    let session = {
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        map.entry(id).or_insert_with(|| Arc::new(Session::new(generation, None))).clone()
    };
    session.begin(generation);
    (session, generation)
}

/// Starts the next epoch for `id`, drops its store and tunnel and removes it from the map.
/// Tasks that still hold the `Arc<Session>` see a stale generation.
fn end_session(&self, id: SessionId) -> u64 {
    let generation = self.next_epoch.fetch_add(1, Ordering::SeqCst) + 1;
    let removed = self.sessions.write().unwrap_or_else(|e| e.into_inner()).remove(&id);
    if let Some(session) = removed { session.begin(generation); }
    generation
}

fn session(&self, id: SessionId) -> Option<Arc<Session>> { /* read lock + clone */ }
```

`spawn_runtime_with_connector(…, store: Option<Arc<dyn DocumentStore>>, …)` keeps its signature: a given store is installed as session `EMBEDDED_SESSION` with generation 0. Tests that used to send data commands without connecting use `EMBEDDED_SESSION`.

`dispatch`: Compare and TestConnection are unchanged. `Connect(cfg)`: `begin_session_epoch(cfg.id)`. `Disconnect { session }`: `end_session(session)`; the handler answers `Disconnected { session, epoch }` without touching the map again. Data command: `shared.session(session)`; if `None`, send `Failed { session, context, error: CoreError::Unsupported("not connected".into()) }` and `repaint()`, then return; otherwise read `session.generation` and spawn as today, passing the `Arc<Session>`. `handlers::execute`, `run`, `finalize`, `send_answer`, `gate`, `connect_store` and `ConnectionCurrent` take the `Arc<Session>` (and its `SessionId`) instead of reading `shared.generation`, and use `session.epoch_lock` and `session.is_current`. `connect_store` installs into the session returned by `begin_session_epoch` (pass it down); if that session was removed meanwhile, the generation no longer matches and the connect reports `Superseded`.

- [ ] **Step 1: Write the failing tests** in `crates/core/src/runtime/session_tests.rs` (add `#[cfg(test)] mod session_tests;` to `mod.rs`). Use the fake `Connector` and `start`/`next` helpers from `runtime/tests/mod.rs`; where a helper does not exist, build the runtime with `spawn_runtime_with_connector` and a connector that returns a distinct `InMemoryStore` per connection id. Tests:

```rust
#[tokio::test]
async fn two_sessions_stay_connected_independently() {
    // connect a, connect b; both answer ListDatabases with their own store's data
}
#[tokio::test]
async fn disconnecting_one_session_leaves_the_other_answering() { /* a disconnected; b still lists */ }
#[tokio::test]
async fn reconnecting_one_session_does_not_supersede_the_other() {
    // b has a slow in-flight Find (use GatedFindStore from tests/races.rs); connect a again; b's answer still arrives as Results, not Superseded
}
#[tokio::test]
async fn a_data_command_for_an_unknown_session_fails_closed_for_that_session() {
    // Find on a never-connected id -> Failed { session: id, context: "find", error: Unsupported(..) }; no other session's store is read
}
#[tokio::test]
async fn a_late_answer_of_a_disconnected_session_is_superseded() { /* Find gated, Disconnect, release -> Failed Superseded for that session */ }
#[tokio::test]
async fn epochs_never_repeat_across_sessions_or_reconnects() {
    // connect a (e1), connect b (e2), disconnect a (e3), connect a (e4): all distinct and increasing
}
#[test]
fn command_and_event_report_their_session() { /* Command::Find{..}.session() == Some(id); Compare/TestConnection -> None */ }
```

- [ ] **Step 2: Run them to see them fail** (they do not compile yet): `cargo test -p curlybrace-core session_tests`. Expected: compile errors about `session`, `EMBEDDED_SESSION`.
- [ ] **Step 3: Implement** `session.rs`, the `Shared` change, `Command`/`Event` changes, `dispatch`, `handlers`, `tunneling` as designed. Update `lib.rs` exports (`SessionId`, `EMBEDDED_SESSION` re-exported from `runtime`).
- [ ] **Step 4: Migrate the existing core tests.** Follow the compiler. Rules: a test that connects then sends data commands uses the connection's id as the session; a test that spawns with a ready store uses `EMBEDDED_SESSION`; assertions on `Event::Databases(v)` become `Event::Databases { names, .. }`. `epoch_tests.rs` builds `Shared` directly: build it with an empty `sessions` map and `next_epoch: AtomicU64::new(0)` and call `begin_session_epoch`. Keep every existing assertion's intent; do not weaken any.
- [ ] **Step 5: Keep the UI compiling with unchanged single-session behaviour.** Add `AppState::session_id(&self) -> Uuid` returning `self.connected.unwrap_or_default()` and stamp every command built in `actions.rs` with it (`Command::Disconnect { session: state.session_id() }` is built before `session_cleared` clears `connected`). In `reducer.rs` bind `session` in each event arm and ignore it (`session: _`) for now, except `Event::Failed`'s `Superseded` arm. Update UI tests mechanically. Behaviour must be identical; the UI wiring tests must pass unchanged in intent.
- [ ] **Step 6: Run the gates.** All four commands from Global Constraints. Expected: green.
- [ ] **Step 7: Commit**

```bash
git add crates/core crates/ui
git commit -m "feat(runtime): hold one session per connection with a global epoch counter"
```

(Add the trailer. Check `git status` shows nothing outside `crates/core` and `crates/ui` before committing.)

---

### Task 2: Topology on connect

**Files:**
- Modify: `crates/core/src/store.rs`, `crates/core/src/mongo_store.rs`, `crates/core/src/runtime/handlers.rs`, `crates/core/src/runtime/mod.rs` (`Event::Connected`), `crates/core/src/test_support/` (in-memory store reports `None`), UI reducer compile fixes.
- Test: `crates/core/src/runtime/session_tests.rs`, `crates/core/src/mongo_store.rs` unit test for the pure classifier, `crates/core/tests/` tier-2 test if a container test file already covers connect.

**Interfaces:**
- Produces: `pub enum Topology { Direct, ReplicaSet, Sharded }` in `store.rs` (`Clone, Copy, Debug, PartialEq, Eq`); `DocumentStore::topology(&self) -> Result<Option<Topology>>` with default `Ok(None)`; `Event::Connected { session, epoch, topology: Option<Topology> }`; `pub fn classify_hello(reply: &bson::Document) -> Option<Topology>` in `mongo_store.rs`.
- Consumes: Task 1 events.

Classifier rule (pure, tested): `msg == "isdbgrid"` is `Sharded`; else a `setName` string means `ReplicaSet`; else `Direct`. A missing or malformed reply is `None`.

- [ ] **Step 1: Write the failing tests.** In `mongo_store.rs` tests: `classify_hello(&doc!{"msg":"isdbgrid"}) == Some(Sharded)`, `doc!{"setName":"rs0","isWritablePrimary":true}` is `ReplicaSet`, `doc!{"isWritablePrimary":true}` is `Direct`, `doc!{}` is `Direct`, `doc!{"setName": 1}` is `Direct` (not a string). In `session_tests.rs`: a connector whose store reports `Some(Topology::ReplicaSet)` yields `Connected { topology: Some(ReplicaSet), .. }`; a store whose `topology()` errors yields `Connected { topology: None, .. }` (a failure never fails the connect).
- [ ] **Step 2: Run red.** `cargo test -p curlybrace-core topology`.
- [ ] **Step 3: Implement.** `MongoStore::topology` runs `{ hello: 1 }` on the `admin` database with the existing client, bounded by the existing command timeout, and returns `classify_hello(&reply)`; any error becomes `Ok(None)`. `connect_store` calls `store.topology().await.unwrap_or(None)` after the server-version-free connect and before installing, inside the existing `connect_timeout` bound, then includes it in `Connected`.
- [ ] **Step 4: Gates and commit** `feat(core): report the server topology when a session connects`.

---

### Task 3: Per-session views in the UI state

**Files:**
- Create: `crates/ui/src/app/session.rs`, `crates/ui/src/app/session_tests.rs`
- Modify: `crates/ui/src/app/state.rs` (new fields, `ConnStatus::Failed`), `crates/ui/src/app/mod.rs` (`mod session;`)

**Interfaces:**
- Produces on `AppState`: new fields `open: Vec<Uuid>`, `active: Option<Uuid>` (replacing the `connected` field; `session_id()` from Task 1 reads `active`), `parked: BTreeMap<Uuid, SessionView>`, `epoch_counter: u64`, `failure: Option<CoreError>`, `topology: Option<Topology>`.
- Produces: `ConnStatus::Failed`; `pub struct SessionView` (fields below); `AppState::activated(self, id: Uuid) -> AppState`; `AppState::with_session(self, id: Uuid, f: impl FnOnce(AppState) -> Out) -> Out`; `AppState::session_dropped(self, id: Uuid) -> AppState`; `AppState::is_open(&self, id: Uuid) -> bool`; `AppState::roots(&self) -> Vec<Root>` with `pub struct Root { pub id: Uuid, pub name: String, pub status: ConnStatus, pub topology: Option<Topology>, pub failure: Option<CoreError>, pub active: bool }`.
- Consumes: nothing from Tasks 1 and 2 except `Topology`.

```rust
// crates/ui/src/app/session.rs
/// Everything one open session shows; the active session's copy lives in `AppState` itself.
#[derive(Debug, Default)]
pub struct SessionView {
    pub status: ConnStatus,
    pub failure: Option<CoreError>,
    pub latest_requested_epoch: u64,
    pub topology: Option<Topology>,
    pub sidebar: SidebarState,
    pub loading: BTreeSet<String>,
    pub find: FindBarState,
    pub issued_query: Query,
    pub page_query: Query,
    pub loading_page: bool,
    pub page: Option<Page>,
    pub page_key: Option<PageKey>,
    pub tree: TreeState,
    pub view_mode: ViewMode,
    pub editor: Option<EditorState>,
    pub pending: Option<Pending>,
    pub ssh_endpoint: Option<HostEndpoint>,
}

impl AppState {
    /// Moves the active view out (leaving defaults); local mutation only, the public API stays immutable.
    fn take_view(&mut self) -> SessionView {
        SessionView {
            status: std::mem::take(&mut self.status),
            failure: self.failure.take(),
            latest_requested_epoch: std::mem::take(&mut self.latest_requested_epoch),
            topology: self.topology.take(),
            sidebar: std::mem::take(&mut self.sidebar),
            loading: std::mem::take(&mut self.loading),
            find: std::mem::take(&mut self.find),
            issued_query: std::mem::take(&mut self.issued_query),
            page_query: std::mem::take(&mut self.page_query),
            loading_page: std::mem::take(&mut self.loading_page),
            page: self.page.take(),
            page_key: self.page_key.take(),
            tree: std::mem::take(&mut self.tree),
            view_mode: std::mem::take(&mut self.view_mode),
            editor: self.editor.take(),
            pending: self.pending.take(),
            ssh_endpoint: self.ssh_endpoint.take(),
        }
    }

    fn restore_view(&mut self, v: SessionView) { /* assign every field back */ }

    pub fn activated(self, id: Uuid) -> AppState {
        if self.active == Some(id) || !self.open.contains(&id) { return self; }
        let mut next = self;
        if let Some(current) = next.active {
            let view = next.take_view();
            next.parked.insert(current, view);
        }
        let incoming = next.parked.remove(&id).unwrap_or_default();
        next.restore_view(incoming);
        next.active = Some(id);
        next
    }

    /// Runs `f` as if `id` were the active session, then puts the previous active session back.
    /// The commands `f` builds carry `id` because `active` is `id` while it runs.
    pub fn with_session(self, id: Uuid, f: impl FnOnce(AppState) -> Out) -> Out {
        if self.active == Some(id) { return f(self); }
        if !self.open.contains(&id) { return (self, Vec::new()); }
        let previous = self.active;
        let (state, commands) = f(self.activated(id));
        let state = match previous.filter(|p| state.open.contains(p)) {
            Some(p) => state.activated(p),
            None => state,
        };
        (state, commands)
    }
}
```

- [ ] **Step 1: Write the failing tests** in `session_tests.rs` (`#[cfg(test)] mod session_tests;` in `session.rs`). Build states with a helper `open_two()` that sets `open = [a, b]`, `active = Some(a)`, gives `a` a page and `b` a parked view with a different page and editor text:

```rust
#[test] fn activating_a_session_parks_the_current_view_and_restores_the_target() { /* a.page shown; activated(b) shows b.page; parked holds a's */ }
#[test] fn activating_twice_round_trips_every_field() { /* activated(b).activated(a) equals the original on all 17 fields */ }
#[test] fn activating_an_unknown_or_already_active_session_changes_nothing() { }
#[test] fn with_session_applies_to_the_parked_view_and_restores_the_active_one() {
    // with_session(b, |s| (AppState{ pending: Some(..), ..s }, vec![])) -> b's parked view has pending, a is still active and untouched
}
#[test] fn with_session_drops_nothing_when_the_callback_closes_the_active_session() { /* previous no longer open -> state stays on the callback's active session */ }
#[test] fn roots_follow_opening_order_and_flag_the_active_one() { }
#[test] fn session_dropped_removes_the_view_and_the_root() { }
```

- [ ] **Step 2: Run red** (`cargo test -p curlybrace-ui session_tests`), implement `session.rs` and the `state.rs` fields (`session_dropped`, `is_open`, `roots` are small; `roots` reads the active view from the foreground fields and the rest from `parked`, using `AppState::connection(id)` for the name, falling back to the id's first 8 characters if the saved connection was deleted).
- [ ] **Step 3: Replace `connected`** with `active` across the crate (`grep -rn "\.connected\b" crates/ui crates/app`): reads use `state.active`, the 14 existing references stay behaviourally identical. `session_cleared` also clears `failure`, `topology`.
- [ ] **Step 4: Gates and commit** `feat(ui): park the view of every open session next to the active one`.

---

### Task 4: Reducer: connect, focus, disconnect, failure and event routing

**Files:**
- Create: `crates/ui/src/app/session_actions.rs`
- Modify: `crates/ui/src/app/actions.rs` (move `connect`/`disconnect` out), `reducer.rs`, `state.rs`, tests `reducer_tests.rs`, `reducer_epoch_tests.rs`, `reducer_session_tests.rs`, new `app/session_flow_tests.rs`

**Interfaces:**
- Produces: `Input::Connect(Uuid)` (unchanged shape: connect, reconnect or focus), `Input::Disconnect(Uuid)` (now names the session), `Input::Activate(Uuid)`, `SidebarAction::InSession { session: Uuid, action: Box<SidebarAction> }` (so a click under a parked root says where it happened); `session_actions::{connect, disconnect, activate}`.
- Consumes: Task 1 events/commands, Task 3 `activated`/`with_session`/`session_dropped`/`roots`, `epoch_counter`.

Behaviour (spec "Flows"):
- `connect(state, id)`: unknown saved connection: unchanged. Open and not `Failed`: `activated(id)`, no command. Otherwise: `epoch = state.epoch_counter + 1`; park the active view (`activated` handles it once `id` is in `open`): push `id` to `open` if missing, insert a fresh parked `SessionView { status: Connecting, latest_requested_epoch: epoch, ssh_endpoint: HostEndpoint::of(&cfg), .. }`, `activated(id)`, set `epoch_counter = epoch`, clear the banner, `workspace = Browse`, command `Connect(cfg)`.
- `disconnect(state, id)`: not open: unchanged. `epoch = counter + 1`; command `Disconnect { session: id }`; if `id` is active choose the neighbour (the open entry after it, else the one before it), `activated(neighbour)` first, then `session_dropped(id)`; with no neighbour set `workspace = Connections` (Task 6 adds the variant; until then leave the manager visible as today because `active` is `None`). Clears `pending` and the dropped view's markers by dropping the view.
- Event routing in `on_event`: take `ev.session()`. If `Some(s)`: `if !state.is_open(s) { return (state, vec![]) }` (late answer of a closed session) then `state.with_session(s, |st| on_session_event(st, ev))`. Compare and TestConnection events keep their existing arms. `on_session_event` is today's `on_event` body for session events, with these changes: `at_epoch` compares against the session's own `latest_requested_epoch` (the foreground one after `with_session`); `ConnectFailed` and a lost session (`Failed { context: "list_databases" .. }` is NOT a loss; only `ConnectFailed` and `Failed { context: "disconnect" }`) set `status = Failed`, `failure = Some(error)`, keep the root and its sidebar cleared, and raise the banner; `Connected` stores `topology`, sets `status = Connected`, clears `failure`; `Disconnected` for a session still open is ignored (the drop already happened in `disconnect`). `Results` additionally requires the session to match (guaranteed by routing) AND `selected_ns == ns && issued_query == query` as today.
- Commands built by `actions.rs` use `state.session_id()` which is `active.unwrap_or_default()`; since `with_session` activates the target, background answers produce commands for the right session (for example `Connected` produces `ListDatabases { session }`).
- A data action (gated) while the active session is not `Connected` raises `Banner::NotConnected` as today.

- [ ] **Step 1: Write the failing tests** in `session_flow_tests.rs` (reducer-level, no runtime, using `update` and `apply_event`):

```rust
#[test] fn connecting_a_second_connection_parks_the_first_and_activates_the_second() { /* two Connect inputs; open == [a, b]; active b; a parked Connected view intact; commands: Connect(cfg_b) with epoch counter 2 */ }
#[test] fn connect_on_an_open_connection_focuses_it_and_sends_nothing() { }
#[test] fn connect_on_a_failed_root_reconnects_it_with_a_new_epoch() { }
#[test] fn a_parked_session_receives_its_own_events() { /* Databases{session: a} while b active -> a's parked sidebar has them, b's untouched */ }
#[test] fn the_same_namespace_on_two_sessions_never_crosses() { /* both select app.users; Results{session:a} replaces only a's page */ }
#[test] fn a_late_answer_of_a_closed_session_is_dropped_and_never_resurrects_the_root() { /* disconnect a; Connected{session:a, epoch old} -> open unchanged */ }
#[test] fn a_stale_connected_after_a_quick_reconnect_is_ignored() { /* Connect a (epoch 1), Disconnect a (2), Connect a (3); Connected{epoch:1} ignored; epoch 3 applies */ }
#[test] fn disconnecting_the_active_session_activates_the_neighbour() { /* [a,b,c], active b -> c; active c -> b */ }
#[test] fn disconnecting_the_last_session_leaves_nothing_active() { }
#[test] fn a_connect_failure_marks_only_that_root_failed() { /* b fails; a stays Connected with its page; failure Some on b's view; banner raised */ }
#[test] fn a_pending_write_of_one_session_does_not_block_another() { /* a.pending set; activate b; mutation on b is sent */ }
#[test] fn a_data_action_without_a_connected_active_session_raises_the_banner_and_sends_nothing() { }
#[test] fn a_sidebar_action_in_a_parked_session_toggles_without_activating_and_open_activates() { }
```

- [ ] **Step 2: Run red**, implement `session_actions.rs` and the reducer changes, migrate `Input::Disconnect` call sites (`grep -rn "Input::Disconnect"`), the sidebar `InSession` wrapper (reducer: `Toggle`/`LoadCollections` use `with_session`, `Open` and the rest `activated(session)` first), and existing reducer tests that pass `Event`s.
- [ ] **Step 3: Review Focus 1, 2, 3, 4, 6** are covered by the tests named above; confirm each by running it and by changing the guard to see it fail once.
- [ ] **Step 4: Gates and commit** `feat(ui): open, focus, fail and close sessions independently`.

---

### Task 5: Shell wiring and the host-key prompt

**Files:**
- Modify: `crates/ui/src/app/mod.rs` (`pump`, `dispatch`, `handle_connection_action` uses of `connected`), `crates/ui/src/app/connection_ops.rs` (`mark_connected` per session), `crates/ui/src/app/host_key.rs` (no change to the queue), `crates/ui/src/app/panels.rs` (host-key dialog text), `crates/ui/src/app/banner.rs`, `locales/en.json`
- Test: `crates/ui/src/app/mod.rs` tests, `crates/ui/tests/wiring.rs`

**Interfaces:**
- Consumes: Task 4 inputs and events.
- Produces: `App::pump` stamps `last_connected` for the session named in `Event::Connected { session }` (not `state.active`); `pending_host_key()` returns `(host, fingerprint, Option<String>)` where the name is the unique open connecting session whose `ssh_endpoint.host` matches, else `None`; i18n key `host_key.for_connection` (`"For connection {name}"`).

- [ ] **Step 1: Write the failing tests.** Wiring: connect two saved connections with the fake connector so both reach `Connected`; each has `last_connected` stamped (and only its own). A second connect to an already open connection sends no `Connect` command (assert on the runtime's command log or the fake connector's call count). Host key: two sessions whose tunnels both ask; `pending_host_key()` shows the first and names its connection, `answer_host_key(Trust)` reaches only the first waiter, the second is then shown; rejecting the first leaves the second answerable (Review Focus 5). When two connecting sessions share one endpoint the name is `None`.
- [ ] **Step 2: Run red, implement.** In `pump`, replace the `was_connected` comparison with: for each `Event::Connected { session, .. }` accepted by the reducer (the session is open and `Connected` afterwards), close the edit dialog and call `mark_connected(state, session, now, path)`. Reject all queued host-key prompts only when the last open session closes (as today on Disconnect); a single session closing leaves the others' prompts alone, and its own abandoned request drops out through `pruned()` once its waiter goes away.
- [ ] **Step 3: Gates and commit** `feat(ui): stamp last connected per session and name the connection in host-key prompts`.

---

### Task 6: The tree with one root per session, and the Connections workspace

**Files:**
- Create: `crates/ui/src/views/sidebar_roots.rs`
- Modify: `crates/ui/src/views/sidebar.rs` (keep the per-session database tree; export what the roots need), `crates/ui/src/views/mod.rs`, `crates/ui/src/app/panels.rs` (`show_browser`, `show_connections_screen`), `crates/ui/src/app/state.rs` (`Workspace::Connections`), `crates/ui/src/app/mod.rs` (frame routing), `crates/ui/src/viewmodel/sidebar.rs`, `crates/ui/locales/en.json`
- Test: `crates/ui/src/views/sidebar_roots.rs` (inline tests), `crates/ui/tests/accessibility.rs`, `crates/ui/tests/snapshots.rs` (new `sessions_tree_*` cases, refresh the others)

**Interfaces:**
- Consumes: Task 3 `roots()`, Task 4 `Input::Activate`, `Input::Disconnect(id)`, `Input::Connect(id)`, `SidebarAction::InSession`.
- Produces: `show_roots(ui, state: &AppState, i18n) -> Option<Input>`; `Workspace::Connections`; i18n keys below.

Layout: the sidebar panel shows a "Connections" button, then for each root a collapsible header (status dot or word, name, topology badge) with Disconnect, and Reconnect when `Failed`. Under an expanded connected root, the existing database tree for that root (the active root reads `state.sidebar`, a parked root reads its parked view's `sidebar`; wrap each emitted `SidebarAction` in `InSession { session, .. }`). A `Failed` root shows its error text (`CoreError` through the existing `errors.*` mapping) in the root's body. The Workspace switcher (Browse/Compare) stays. When `active` is `None` or `workspace == Connections`, the central area shows the manager screen; Connect there calls `Input::Connect`, which sets `workspace = Browse` (Task 4). The manager's Connecting indicator reads the status of the connection being opened.

New i18n keys (under existing prefixes, alphabetical): `sidebar.connections`, `sidebar.disconnect`, `sidebar.reconnect`, `sidebar.status_connected`, `sidebar.status_connecting`, `sidebar.status_failed`, `sidebar.topology_direct`, `sidebar.topology_replica_set`, `sidebar.topology_sharded`, `sidebar.root_failed`, plus `host_key.for_connection` from Task 5. Reuse `connections.disconnect` where it already says the right thing; do not duplicate keys.

- [ ] **Step 1: Write the failing tests.** Inline view tests: `show_roots` renders zero, one, three roots (connected, connecting, failed) without panicking and returns `None`; flattened row accounting for several roots; a click helper test that `Open` under a parked root yields `InSession { session: parked_id, .. }`. Accessibility (follow the existing pattern in `tests/accessibility.rs`): every root header, Disconnect, Reconnect and the Connections button is reachable and has a distinct accessible name containing the connection name. Snapshots: `sessions_tree_three_roots` (connected with a database open, connecting, failed), `sessions_tree_one_root`.
- [ ] **Step 2: Run red, implement.** Keep `show_sidebar` working for a single `SidebarState` (the per-root body calls it), so the existing virtualised-tree tests still apply unchanged.
- [ ] **Step 3: Gates, refresh and review the snapshots** (look at every PNG), commit `feat(ui): show one tree root per open connection`.

---

### Task 7: The connection is always named; Compare defaults to the active session

**Files:**
- Modify: `crates/ui/src/app/panels.rs` (documents header), `crates/ui/src/views/documents.rs` or `editor.rs` where the delete and replace confirmations are built, `crates/ui/src/app/compare.rs` (`state.connected` becomes `state.active`), `locales/en.json`
- Test: `crates/ui/src/app/compare_tests/*` where the default source is asserted, `crates/ui/tests/wiring.rs`, `crates/ui/tests/snapshots.rs` (`delete_confirmation` etc. change)

**Interfaces:**
- Consumes: `AppState::roots()` / `connection(active)`.
- Produces: header line `documents.connection` (`"Connection: {name}"`); the delete confirmation key (find it with `grep -rn "delete_confirm\|confirm_delete" crates/ui/locales/en.json crates/ui/src`) gains a `{connection}` parameter in its text; the editor save button's tooltip or the editor title names the connection; no change to what a write does.

- [ ] **Step 1: Write the failing tests.** A frame test that the documents header contains the active connection's name; the delete confirmation text contains it (Review Focus 7); `compare_to` uses the active session as the source when two sessions are open; with the active session switched, the default changes with it.
- [ ] **Step 2: Run red, implement, refresh snapshots (review each PNG), gates, commit** `feat(ui): name the connection on every write confirmation`.

---

### Task 8: Integration, wiring and accessibility coverage

**Files:**
- Create: `crates/core/tests/docker_mongo_sessions.rs` (tier 2, same gating as `docker_mongo_auth.rs`)
- Modify: `crates/ui/tests/wiring.rs`, `crates/ui/tests/connections_secrets_audit.rs` (assert no secret reaches any session's `Debug` or the `parked` views), `crates/ui/tests/accessibility.rs`

**Interfaces:** consumes everything above.

- [ ] **Step 1: Write the tests.**
  - Tier 2 (`#[ignore]`, run by `scripts/container-tests.sh`): start two MongoDB containers (reuse the helpers in `docker_mongo_auth.rs`); connect both through the real runtime; `ListDatabases` on each returns that server's data; stop container A; B still answers `Find`; A's next command fails with a connection error stamped with A's session, not B's.
  - Wiring (`wiring.rs`): full path with the fake connector: open three connections, switch active between them via `Activate`, each restores its own page and editor text; disconnect the middle one; the others are unaffected.
  - Secrets audit: after opening two sessions with typed passwords, `format!("{:?}", app.state())` and the files on disk contain no password or SSH secret.
  - Accessibility: Tab order reaches Connections button, every root, every root action.
- [ ] **Step 2: Run** `scripts/container-tests.sh` if Docker is available; otherwise say it was not run. Tier 1 (`scripts/integration-tests.sh`) likewise.
- [ ] **Step 3: Gates and commit** `test: cover two simultaneous sessions end to end`.

---

### Task 9: Docs and tracking

**Files:**
- Modify: `README.md` (roadmap: move "several connections at once" from Next to Built, keep the issue link), `site/index.html` (same roadmap entry), `docs/superpowers/specs/2026-10-14-connection-manager-design.md` (slice table: mark slice 2 built), `CONTRIBUTING.md` only if it mentions the single session

- [ ] **Step 1:** Make the edits. No new claims about other tools; own wording only.
- [ ] **Step 2:** `grep -rn "single\|one connection\|only one" README.md site/index.html docs/*.md` and fix stale statements about a single connection.
- [ ] **Step 3: Gates and commit** `docs: record several simultaneous connections as built`.

---

## After the last task

Run the final whole-branch review (most capable model), fix findings test-first, re-review the fixes, then push and open the PR against `main` (confirm with the maintainer first). In the PR body list: the spec and plan paths, the new i18n keys, a Security section (sessions, tunnels, host-key trust), the unchecked manual steps (tier-1 and tier-2 tests if not run, a click-through with two real servers), and `Refs #46`. Update #85 and #46 with anything triaged as a follow-up (search across connections, Compare on open sessions for slice 3).
