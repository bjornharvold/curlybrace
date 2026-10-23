# Read-only connections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A saved connection can be marked read-only; the core refuses every database write on it, and the UI shows and respects that.

**Architecture:** A `ReadOnlyStore` wrapper around `Arc<dyn DocumentStore>` rejects the four write methods with `CoreError::ReadOnly`. Each wrapper owns a shared `Arc<AtomicBool>`; a runtime registry keyed by saved-connection id lets a new `Command::LockReadOnly` raise that flag on every live store (data session and compare) without reconnecting. The UI mirrors #108's "snapshot OR live saved flag" rule for what it shows and gates.

**Tech Stack:** Rust workspace (`curlybrace-core`, `curlybrace-ui`), egui + egui_kittest/accesskit tests, serde, thiserror, `ui/locales/en.json` i18n.

**Spec:** `docs/superpowers/specs/2026-10-23-read-only-connections-design.md` (read it first, including "Calls made while writing this").

## Global Constraints

- The core enforces; the UI only mirrors. A write on a read-only connection never reaches the database, by any path including Data Compare apply.
- Missing `read_only` loads `false`; a present-but-non-boolean value loads `true` with a `log::warn!` and counts as repaired (mirror `lenient_production`, `crates/core/src/connection.rs:21-33`).
- `LockReadOnly` only raises a flag. Nothing lowers a live store's flag; unticking applies on reconnect.
- A read-only connection may be a Data Compare **source**, never the **target** (refused at `Open` and at `Apply` in the core, excluded in the picker).
- Refusal is `CoreError::ReadOnly` before any store call; never `WriteOutcomeUnknown`/`UnreachableWrite`.
- Connection management (edit, duplicate, delete the saved entry) stays allowed.
- Every user-facing string is a key in `crates/ui/locales/en.json` (nested). Report new keys in the PR. No colour-only signal.
- TDD, coverage >= 80%, immutable updates (`with_*`/new values), files < 800 lines, real objects over mocks (mock only at the store boundary).
- Commits: conventional format, lowercase subject, trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Maven is not used; run `cargo test -p <crate> <filter>`.

## Review Focus

Failure modes the spec implies; each has a test in the named task.
1. A write method added to `DocumentStore` later and forgotten in `ReadOnlyStore` (Task 3).
2. Reads through the wrapper silently degrading: defaulted trait methods (`scan_sorted_primary`, `ping`, `topology`, `server_version`, `list_collection_kinds`...) must forward, not fall back to the trait defaults (Task 3).
3. Reconnect after unticking must produce a writable store; a fresh flag per connect, not a shared sticky one (Task 4).
4. Two sessions on one saved connection (data + compare): one `LockReadOnly` locks both (Task 5).
5. Connection deleted or renamed while open: lock still reaches its stores, UI keeps the snapshot (Tasks 4, 7).

---

### Task 1: `read_only` on the saved connection

**Files:**
- Modify: `crates/core/src/connection.rs` (field, `new`, lenient reader), `crates/ui/src/app/persistence.rs:~257` (repaired detection)
- Test: `crates/core/src/connection_lenient_tests.rs`, `crates/ui/src/app/persistence_load_tests.rs`

**Interfaces:**
- Produces: `ConnectionConfig.read_only: bool`; `fn lenient_read_only<'de, D>(D) -> Result<bool, D::Error>`.

- [ ] **Step 1: Write failing tests** — `old_file_without_read_only_loads_false`; `non_boolean_read_only_loads_true` (e.g. `"read_only": "yes"` and `1` both -> `true`); `read_only_round_trips_and_duplicate_keeps_it`; in persistence tests `damaged_read_only_counts_as_repaired_and_is_rewritten_as_true`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core connection` and `cargo test -p curlybrace-ui persistence_load` — expect compile failure / FAIL.
- [ ] **Step 3: Implement** `#[serde(default, deserialize_with = "lenient_read_only")] pub read_only: bool` next to `production`, `false` in `ConnectionConfig::new`; extend the repair detection at `persistence.rs:257` so a damaged `read_only` is also "repaired" (same rewrite-as-`true` behaviour; count it in the same banner, own flag key — Task 6 adds the text).
- [ ] **Step 4: Run** the same commands — PASS. Also `cargo test -p curlybrace-core` for struct-literal fallout.
- [ ] **Step 5: Commit** `feat(connections): save a read-only flag with the connection`.

### Task 2: `CoreError::ReadOnly`

**Files:**
- Modify: `crates/core/src/error.rs` (variant ~101, `i18n_key` ~122-160, distinct-keys test ~201), `crates/ui/locales/en.json` (`errors.read_only`)
- Test: existing `every_variant_has_a_distinct_i18n_key`, `crates/ui/src/i18n.rs` key-exists test

**Interfaces:**
- Produces: `CoreError::ReadOnly` with `i18n_key() == "errors.read_only"`.

- [ ] **Step 1:** Add `CoreError::ReadOnly` to the distinct-keys test list; run `cargo test -p curlybrace-core error` — FAIL (no variant).
- [ ] **Step 2: Implement** the variant (`#[error("connection is read-only")]`, `Clone + PartialEq` like its siblings), the `i18n_key` arm, and `errors.read_only` in `en.json` ("This connection is read-only, so nothing was changed.").
- [ ] **Step 3: Run** `cargo test -p curlybrace-core error` and `cargo test -p curlybrace-ui i18n` — PASS.
- [ ] **Step 4: Commit** `feat(core): add the read-only error`.

### Task 3: `ReadOnlyStore`

**Files:**
- Create: `crates/core/src/read_only_store.rs` (+ `mod` in `lib.rs`)
- Modify: `crates/core/src/store.rs` (trait gains `fn is_read_only(&self) -> bool { false }`)
- Test: `crates/core/src/read_only_store_tests.rs`; use the existing in-memory/recording stores under `crates/core/src/test_support`

**Interfaces:**
- Produces:
  - `pub struct ReadOnlyStore { inner: Arc<dyn DocumentStore>, locked: Arc<AtomicBool> }`
  - `ReadOnlyStore::wrap(inner: Arc<dyn DocumentStore>, locked: Arc<AtomicBool>) -> Arc<dyn DocumentStore>`
  - `DocumentStore::is_read_only(&self) -> bool` (wrapper returns `locked.load(SeqCst)`).
  - When `locked` is false the wrapper behaves exactly like `inner`.

- [ ] **Step 1: Write failing tests** with a recording store that counts calls per method: `locked_store_refuses_insert_replace_delete_and_bulk_write_with_read_only` (assert `Err(CoreError::ReadOnly)` and the inner write count stays 0); `unlocked_store_forwards_writes`; `locking_after_wrap_refuses_the_next_write`; `reads_and_defaulted_methods_forward_to_inner` (one assertion per read/defaulted method listed in Review Focus 2, using an inner store that returns non-default values); `every_write_method_is_covered` — a test listing the four write methods that fails to compile or run if `DocumentStore` gains a write method not named here (builder picks mechanism; at minimum a comment on the trait's write methods pointing at this test).
- [ ] **Step 2: Run** `cargo test -p curlybrace-core read_only_store` — FAIL.
- [ ] **Step 3: Implement** the wrapper; the four writes check the flag first and return `CoreError::ReadOnly`; every other trait method delegates explicitly (including all defaulted ones).
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** `feat(core): refuse writes through a read-only store wrapper`.

### Task 4: Runtime wiring for data sessions

**Files:**
- Create: `crates/core/src/runtime/read_only_registry.rs`
- Modify: `crates/core/src/runtime/mod.rs` (`Command::LockReadOnly`, `is_write`/`context`/`session` arms, `Shared` gains the registry, dispatch), `crates/core/src/runtime/handlers.rs` (`connect_store` ~245-270 wraps; `run` ~125 early check)
- Test: `crates/core/src/runtime/read_only_tests.rs`

**Interfaces:**
- Produces:
  - `ReadOnlyRegistry::attach(&self, connection: Uuid, initial: bool) -> Arc<AtomicBool>` — creates a fresh flag, remembers it by `Weak`, prunes dead ones.
  - `ReadOnlyRegistry::lock(&self, connection: Uuid)` — sets every live flag for that connection to `true`.
  - `Command::LockReadOnly { connection: Uuid }`: `is_write() == false`, `context() == "lock_read_only"`, `session() == None`, independent of sessions like `TestConnection` (no epoch, no answer event; confirm in `dispatch` that it neither creates a session nor emits `Failed`).
- Consumes: `ReadOnlyStore::wrap`, `DocumentStore::is_read_only`, `CoreError::ReadOnly`.

- [ ] **Step 1: Write failing tests** (runtime tests, in-memory connector): `connect_with_read_only_config_refuses_each_write_command` (loop `Insert`, `Replace`, `Delete`, `Clone` -> `Event::Failed { error: CoreError::ReadOnly, context }` with the matching context, inner write count 0, never `WriteOutcomeUnknown`); `reads_still_work_on_a_read_only_session`; `lock_read_only_refuses_writes_on_an_open_session`; `reconnect_with_unticked_config_is_writable_again` (Review Focus 3); `lock_for_unknown_connection_is_a_no_op`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core runtime::read_only` — FAIL.
- [ ] **Step 3: Implement** the registry (`Mutex<HashMap<Uuid, Vec<Weak<AtomicBool>>>>`); in `connect_store` wrap the connector's store with `registry.attach(cfg.id, cfg.read_only)`; in `run`, before `data_command`, if `command.is_write() && store.is_read_only()` return `Err(CoreError::ReadOnly)` so no timeout/ping bookkeeping starts; add the `Command` arm plus the `data_command` `unreachable!` arm.
- [ ] **Step 4: Run** — PASS, then `cargo test -p curlybrace-core runtime` for regressions.
- [ ] **Step 5: Commit** `feat(core): enforce read-only on data sessions and lock open ones`.

### Task 5: Data Compare

**Files:**
- Modify: `crates/core/src/runtime/compare/mod.rs` (`open_session` ~449-470, `open_side` ~484), `crates/core/src/runtime/compare/apply.rs` (`start_apply` ~88)
- Test: `crates/core/src/runtime/compare/read_only_tests.rs`

**Interfaces:**
- Consumes: registry `attach`/`lock`, `ReadOnlyStore::wrap`, `is_read_only`.
- Behaviour: both sides are wrapped via `attach(cfg.id, cfg.read_only)`; `open_session` fails with `CompareEvent::OpenFailed { error: CoreError::ReadOnly }` when the **target** config is read-only; `start_apply` returns `Err(CoreError::ReadOnly)` first thing (before expanding ops) when `connections.target.is_read_only()`.

- [ ] **Step 1: Write failing tests:** `open_with_read_only_source_succeeds_and_reads`; `open_with_read_only_target_fails_with_read_only`; `apply_after_target_is_locked_is_refused_with_context_compare_apply` (`LockReadOnly` between `Open` and `Apply`; target store write count 0); `one_lock_reaches_data_session_and_compare_store` (Review Focus 4); `export_to_file_still_works_for_a_read_only_source`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core compare::read_only` — FAIL.
- [ ] **Step 3: Implement** as described; `Failed { context: "compare_apply" }` is the existing refusal path in `apply::dispatch`.
- [ ] **Step 4: Run** it and `cargo test -p curlybrace-core compare` — PASS.
- [ ] **Step 5: Commit** `feat(core): keep read-only connections out of compare targets`.

### Task 6: Connection dialog and form

**Files:**
- Modify: `crates/ui/src/viewmodel/connection_form.rs` (copy at ~204, load at ~250), `crates/ui/src/views/connections_edit.rs`, `crates/ui/locales/en.json` (`connection.read_only`, `connection.read_only_hint`, repaired-banner key next to the `repaired_*` keys)
- Test: `crates/ui/src/viewmodel/connection_form_tests.rs`, `crates/ui/tests/accessibility_connections.rs`

**Interfaces:**
- Produces: `read_only: bool` on the form viewmodel, written to/read from `ConnectionConfig`.

- [ ] **Step 1: Write failing tests:** `form_round_trips_read_only`; `new_form_defaults_read_only_false`; accessibility `read_only_checkbox_is_labelled_and_toggles` (`get_by_role_and_label(Role::CheckBox, &i18n.t("connection.read_only"))`, hint present); the repaired banner names the read-only flag.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui connection_form` and `--test accessibility_connections` — FAIL.
- [ ] **Step 3: Implement** the checkbox next to production (mirror its row/ids/focus order) and keys; `i18n_usage_tests` must pass (keys referenced in source).
- [ ] **Step 4: Run** — PASS; also `cargo test -p curlybrace-ui i18n_usage`.
- [ ] **Step 5: Commit** `feat(ui): edit the read-only flag in the connection dialog`.

### Task 7: Session state, lock command, write gating

**Files:**
- Modify: `crates/ui/src/app/write_guard.rs:~69-84` (add `session_read_only(id)`, `active_is_read_only()` mirroring `session_production`), `crates/ui/src/app/state.rs` (+ `SessionView` snapshot field beside `production`), `crates/ui/src/app/session_actions.rs:26`, `crates/ui/src/app/connection_save.rs` (send `Command::LockReadOnly` when saving a connection that has open sessions and `read_only` newly true), `crates/ui/src/app/actions.rs` (`mutation` ~287 refuses with a banner before `Guard` logic), `crates/ui/src/app/compare_apply.rs:90`, `crates/ui/src/app/banner.rs` (new read-only banner modelled on `Banner::ProjectionReadOnly`), `en.json` (`banner.read_only` or the file's prefix convention)
- Test: `crates/ui/src/app/read_only_snapshot_tests.rs` (model on `production_snapshot_tests.rs`), `write_guard_tests.rs`

**Interfaces:**
- Produces: `AppState::session_read_only(&self, id: Uuid) -> bool` = snapshot (active: `self.read_only`, parked: `view.read_only`) `||` saved flag; deleted connection judged by snapshot alone (Review Focus 5).
- Consumes: `Command::LockReadOnly { connection }`.

- [ ] **Step 1: Write failing tests:** `ticking_saved_flag_locks_open_session_at_once`; `unticking_keeps_snapshot_until_reconnect`; `deleted_connection_keeps_snapshot_read_only`; `saving_newly_read_only_connection_sends_lock_command` (and not when unchanged or unticked); `insert_edit_clone_delete_are_refused_with_banner_and_send_no_command`; `compare_apply_not_sent_when_target_read_only`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui read_only` — FAIL.
- [ ] **Step 3: Implement** per Interfaces; the refusal precedes `Guard::Warn`/`Confirmed` so a read-only production connection never shows the confirm modal.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui` — PASS.
- [ ] **Step 5: Commit** `feat(ui): lock writes on read-only sessions and send the live lock`.

### Task 8: Marker, disabled actions, compare picker

**Files:**
- Modify: `crates/ui/src/viewmodel/connection_tag.rs` and the places that render `production_badge` (`state.rs:230 session_badge`, `panels.rs:~186`, `views/documents.rs`, `views/connections_*`), `crates/ui/src/app/compare_select.rs` / `connection_picker.rs` (exclude read-only targets with a reason), `crates/ui/src/views/compare/apply.rs`, `en.json` (`read_only_badge`, `read_only_reason`, `compare.target_read_only`)
- Test: `crates/ui/tests/accessibility_read_only.rs` (new), `crates/ui/tests/wiring_read_only.rs` (new), update affected snapshots in `crates/ui/tests/snapshots` deliberately

**Interfaces:**
- Consumes: `session_read_only`, existing badge helpers. Badge is text ("Read-only"), shown beside the production badge, never colour alone.

- [ ] **Step 1: Write failing tests:** accessibility `read_only_badge_is_text_on_list_header_and_compare`; `write_actions_are_disabled_with_a_stated_reason` (labels via accesskit, disabled state, reason text present); `compare_target_picker_excludes_read_only_and_says_why`; `compare_source_picker_still_lists_read_only`; wiring test driving save -> lock -> next write refused end to end with the in-memory runtime.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui --test accessibility_read_only --test wiring_read_only` — FAIL.
- [ ] **Step 3: Implement** the badge, disabled/hidden actions with tooltips from `read_only_reason`, picker filtering.
- [ ] **Step 4: Run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` — PASS; review snapshot diffs visually before accepting.
- [ ] **Step 5: Commit** `feat(ui): mark read-only connections and disable their writes`.

---

## Self-review notes

- Spec coverage: model (T1), error (T2), core wrapper + live flag (T3-4), compare source/target + both checks (T5), dialog + repaired banner (T6), UI snapshot-or-live rule + lock send + gating (T7), marker + picker + a11y (T8), i18n in every UI task, testing section distributed across tasks.
- Call 3 of the spec (`is_write()` not trusted alone) is covered by T3's wrapper backstop plus T4's all-commands test; if the early check ever disagrees, `ReadOnly` still comes from the store before ping bookkeeping can matter, because the wrapper rejects before touching the inner store.
- Builder decides: the exact "every write method covered" mechanism (T3), whether `LockReadOnly` needs an acknowledgement event (default: none), badge placement, and string wording.
