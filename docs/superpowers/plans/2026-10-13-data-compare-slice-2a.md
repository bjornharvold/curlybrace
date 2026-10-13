# Data Compare, Slice 2a: Apply Engine (core) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the core write the differences a compare found: copy a document to the other side, replace a changed one, delete an extra one, in batches, after re-verifying that nothing changed since the compare, with per-document outcomes, cancel and progress. No UI yet.

**Architecture:** Slice 2 is split in three plans so each stays reviewable and ships working, tested software: **2a (this plan) the apply engine in `curlybrace-core`** (structured field paths, a batched write contract on `DocumentStore`, pure op planning, the re-verifying engine, the runtime command/events); **2b** the UI (selection, pending-changes list, confirmation dialog, progress, results); **2c** field-level operations (`$set`/`$unset` of one path) on top of the structured paths and the same engine. Apply is a second job kind that shares the session's single "active job" slot with the compare run, so a run and an apply never overlap.

**Tech Stack:** Rust, `mongodb` 3.6 driver (write commands through `run_command`, so no server-8-only `bulkWrite`), `bson` 2.15, tokio, existing `curlybrace-core::compare` and `runtime::compare` modules.

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` ("Apply (slice 2, designed now)", "`DocumentStore` additions", "Security and privacy", the `FieldChange.path` note under "Sessions, commands and events"). Issue #8. Slices 1, 1b, 1c shipped in PRs #29, #32, #34.

## Decisions made while planning (rulings; each states what it costs if wrong)

1. **Split into 2a/2b/2c.** The spec's slice 2 spans engine, runtime, selection UI, confirmation and field-level ops; one PR would be unreviewable for a feature that writes to users' databases. 2a has no visible UI change. Cost if wrong: one more plan/PR cycle.
2. **Writes go through `run_command` (`insert`/`update`/`delete`, `ordered: false`)**, not the driver's `Client::bulk_write` (server 8.0+ only). It works on every server the app already supports. Cost if wrong: replace later behind the same `bulk_write` trait method.
3. **Destination documents are matched by their own `_id`, not by the key fields.** The spec says replace "matches by the key fields". The engine has just re-read the destination document, so it knows that document's `_id`; matching on it can never touch a different document, even if the key is later duplicated. The replacement document carries that `_id` (immutable). Cost if wrong: none observable; it is strictly narrower.
4. **`direction` is origin to destination of the copy** and the three kinds are what happens to the *destination* document: `Insert` (origin has it, destination does not), `Replace` (both have it, they differ), `Delete` (destination has it, origin does not). "Delete extras in the target" is `Delete` with `SourceToTarget` on an only-in-target row; "delete this source-only document" is `Delete` with `TargetToSource`. An op whose kind does not fit its row's recorded category is `Skipped(NotPlanned)`, never written.
5. **Re-verify = re-read both documents (with the pair's own filters), recompute both fingerprints with the pair's rules, require both to equal the recorded ones** (a missing document equals a recorded `None`). Any mismatch is `Skipped(Stale)`. Residual window between re-read and write stays and is documented (spec).
6. **Server error text is never surfaced.** A duplicate-key write error message contains the offending key value. The store maps write errors to a typed `WriteFailure` (`DuplicateKey`, `Rejected(code)`); only codes and kinds travel, never `errmsg`. Keys travel only inside `OpFailure.key` (UI-only, never logged), like the duplicates report.
7. **Views are refused up front; time-series collections accept inserts only** (the server restricts update/delete on them). The check uses `list_collection_kinds` on the destination database(s) before any write. A destination collection that does not exist yet is fine for inserts (the server creates it).
8. **One active job per session.** Apply takes the same `ActiveRun` slot as a compare run (`RunId` doubles as the apply id; `Cancel { run }` cancels it). `Apply` while a run is active, or a run while an apply is active, is refused. Cancel stops between batches; ops not yet processed are reported as `not_run`.
9. **Stored compare results are not edited by an apply.** After an apply the pair's rows are stale by definition; the UI (2b) re-runs the pair to refresh. A re-run converges: it plans only what still differs. Two known non-convergences are documented, not hidden: `IdPolicy::Generate` (inserts get new `_id`s, so an `_id`-keyed re-compare shows them again) and `include_id = true` with a key other than `_id` (a replace keeps the destination `_id`, so the `_id` difference remains). 2b warns about both.
10. **Per-op outcomes are aggregated, failures are capped.** The report carries counts plus the first 100 failures (key + typed error) and a `more_failures` flag, so a million-op apply never builds a million-element event.

## Global Constraints

- **Writes only on explicit command.** The core never writes unless it receives `CompareCommand::Apply`; nothing in this plan adds a call site for it (2b adds the confirmed UI path). `Apply` never writes without the re-verification step (spec: "Security and privacy").
- Keys and documents are data: **never logged**; error text never contains server `errmsg`. Logs carry ids, counts and error kinds only.
- Every new `CoreError` variant gets an `errors.*` key in `crates/ui/locales/en.json` (nested, alphabetical) and an entry in the `every_variant_has_a_distinct_i18n_key` test. No literal user-facing English in code.
- Immutability; files < 800 lines (aim 200-400); functions < 50 lines; nesting <= 4. `runtime/compare/mod.rs` (427) and `runner.rs` (381) must not grow past 600: new runtime code goes in a new `runtime/compare/apply.rs`.
- TDD for every task. Default `cargo test --workspace --locked` stays DB-, docker- and GPU-free (use `InMemoryStore`). Tier-1 tests are `#[ignore]` + env-gated (`CURLYBRACE_IT`), use only throwaway `cbit_<id>` databases and only a `cb-it-dev-*` container the test run starts itself (e.g. `cb-it-dev-2a`, removed afterwards), never the user's `local7979`/`local265` or `127.0.0.1:27017`. GitHub CI must never need MongoDB (`scripts/check-no-db-in-ci.sh` stays green).
- Commits: conventional format ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Before every push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Sub-agents default to Sonnet. Tasks 3, 5 and 6 (they write to databases, or are the concurrency/state-machine part) and the final review use Opus.

## Review Focus

1. **A write that should not have happened.** Stale document (changed on either side since the compare), op kind that does not fit the row, pair discarded, view destination, op for a key that is not in the results: none may reach `bulk_write`. Pinned in Tasks 4, 5, 6.
2. **Secret/data leakage.** No server `errmsg` and no key in any log line, `Display`, or `CoreError` text. Pinned in Tasks 2, 3 (duplicate-key test asserts the error text contains no key).
3. **`_id` handling.** Key is `_id` vs a business key; `_id` of mixed BSON types (ObjectId, string, int, a sub-document); replace keeps the destination `_id`; `IdPolicy::Generate`. Pinned in Tasks 4 and 7.
4. **Batch boundaries and partial failure.** Exactly 500, 501, 0 ops; one duplicate-key insert among good ops (neighbours still applied, outcome counted once); a whole batch rejected (unauthorized). Pinned in Tasks 5 and 7.
5. **Cancel and close mid-apply.** Cancel between batches reports `not_run`; a closed session emits nothing more; a second `Apply`/`Start` while one is active is refused; the slot is freed on panic. Pinned in Task 6.

## File Structure

```
crates/core/src/compare/
  diff.rs                  + PathSegment, FieldChange.segments, display path built from segments   (Task 1)
  apply.rs                 NEW  ApplyOp, Direction, ApplyKind, IdPolicy, OpOutcome, SkipReason,
                                ApplyReport, OpFailure, plan_write()                               (Task 4)
  apply_engine.rs          NEW  apply_pair(): verify, plan, batch, bulk_write, cancel, report      (Task 5)
  protocol.rs              + CompareCommand::Apply, CompareEvent::{ApplyProgress,ApplyPairDone,ApplyDone} (Task 6)
  mod.rs                   + pub mod apply; pub mod apply_engine
crates/core/src/
  store.rs                 + WriteOp, WriteFailure, WriteOutcome, DocumentStore::bulk_write         (Task 2)
  error.rs                 + ApplyTargetNotWritable, ApplyWriteRejected (+ keys, test entries)      (Task 2)
  mongo_store.rs           + bulk_write via run_command                                             (Task 3)
  test_support/memory_store.rs  + bulk_write                                                        (Task 2)
crates/core/src/runtime/compare/
  apply.rs                 NEW  start_apply(): preconditions, job slot, task, events                (Task 6)
  mod.rs / session.rs      route Apply; command_context "compare_apply"                              (Task 6)
crates/core/tests/local_compare_apply.rs   NEW tier-1 (real server)                                 (Tasks 3, 7)
crates/ui/locales/en.json  + errors.apply_*; failures.compare_apply                                 (Tasks 2, 6)
docs/superpowers/specs/...-design.md, docs/superpowers/notes/testing.md   "Slice 2a as built"       (Task 7)
```

---

### Task 1: Structured field paths

The detail view shows `FieldChange.path` (`a.b[2].c`). A field name containing a dot or bracket is ambiguous in that string; field-level apply (2c) needs an address. Add structured segments next to the display string; keep `path` unchanged for the UI.

**Files:**
- Modify: `crates/core/src/compare/diff.rs`
- Test: same file (`mod tests`)

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq, Eq)]
  pub enum PathSegment {
      Field(String),
      Index(usize),
      /// An element of an unordered array (`[*]`): not addressable, so never a write target.
      AnyElement,
  }
  // FieldChange gains: pub segments: Vec<PathSegment>
  impl FieldChange { pub fn is_addressable(&self) -> bool } // false when any segment is AnyElement
  pub fn render_path(segments: &[PathSegment]) -> String   // "a.b[2].c", "a[*]"
  ```
  `path` is always `render_path(&segments)`.

- [ ] **Step 1: Write the failing tests** (in `diff.rs` `mod tests`; reuse the file's `doc!` import and existing helper style)

  ```rust
  #[test]
  fn a_field_name_with_a_dot_is_one_segment() {
      let s = doc! { "a.b": 1 };
      let t = doc! { "a.b": 2 };
      let changes = diff_normalized(&s, &t, false).unwrap();
      assert_eq!(changes[0].segments, vec![PathSegment::Field("a.b".into())]);
      assert_eq!(changes[0].path, "a.b"); // display form is ambiguous on purpose; segments are the address
  }

  #[test]
  fn nested_and_array_segments_render_to_the_display_path() {
      let s = doc! { "a": { "b": [ { "c": 1 } ] } };
      let t = doc! { "a": { "b": [ { "c": 2 } ] } };
      let c = &diff_normalized(&s, &t, false).unwrap()[0];
      assert_eq!(
          c.segments,
          vec![
              PathSegment::Field("a".into()),
              PathSegment::Field("b".into()),
              PathSegment::Index(0),
              PathSegment::Field("c".into()),
          ]
      );
      assert_eq!(c.path, "a.b[0].c");
      assert!(c.is_addressable());
  }

  #[test]
  fn an_element_of_an_unordered_array_is_not_addressable() {
      let s = doc! { "a": [1, 2] };
      let t = doc! { "a": [1, 3] };
      let changes = diff_normalized(&s, &t, true).unwrap();
      assert!(changes.iter().all(|c| !c.is_addressable()));
      assert!(changes.iter().all(|c| c.path == "a[*]"));
  }
  ```

- [ ] **Step 2: Run to verify failure** — `cargo test -p curlybrace-core diff::tests -- --locked` → FAIL (`PathSegment` not defined).
- [ ] **Step 3: Implement.** Replace the `join(path, key)`/`format!("{path}[{i}]")` string building in `Walker` with a `Vec<PathSegment>` carried down the recursion (`&mut Vec<PathSegment>` push/pop, cloned into `FieldChange` at `push`). `path` = `render_path(&segments)`. Fix every `FieldChange { .. }` construction site the compiler reports (`rg "FieldChange \{" crates`), including UI tests and `viewmodel/compare/detail.rs`; the UI keeps using `path`.
- [ ] **Step 4: Run** — `cargo test --workspace --locked` → PASS (all existing diff and UI detail tests unchanged).
- [ ] **Step 5: Commit** — `feat(compare): structured field paths on FieldChange`

---

### Task 2: Batched write contract, typed errors, in-memory implementation

**Files:**
- Modify: `crates/core/src/store.rs`, `crates/core/src/error.rs`, `crates/core/src/test_support/memory_store.rs`, `crates/ui/locales/en.json`
- Test: `memory_store.rs` tests, `error.rs` tests

**Interfaces:**
- Produces (in `store.rs`):
  ```rust
  /// One write against one collection. Matched by `_id` (see plan ruling 3).
  #[derive(Clone, Debug, PartialEq)]
  pub enum WriteOp {
      Insert { doc: Document },                 // `_id` generated by the server when the doc has none
      Replace { id: Bson, doc: Document },      // `doc` carries the same `_id`
      Delete { id: Bson },
  }
  /// Why one write did not happen. Never carries server text or document values.
  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum WriteFailure {
      /// Nothing matched `_id` (a Replace or Delete): the document is gone.
      Missing,
      /// A unique index (or `_id`) already holds that value.
      DuplicateKey,
      /// The server refused it; the numeric error code.
      Rejected(i32),
  }
  pub type WriteOutcome = std::result::Result<(), WriteFailure>;
  // DocumentStore:
  /// Applies `ops` (unordered, no ordering guarantee between them) and returns one outcome per
  /// op, index-aligned. `Err` only when the call as a whole failed (connection, authorization):
  /// then nothing may be assumed written.
  async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<Vec<WriteOutcome>> { Err(Unsupported(..)) }
  ```
- Produces (in `error.rs`): `CoreError::ApplyTargetNotWritable` (key `errors.apply_target_not_writable`), `CoreError::ApplyWriteRejected` (key `errors.apply_write_rejected`). Texts must be static and secret-free. `en.json` text: "The destination cannot be written to (a view, or a time-series collection for this kind of change)." and "The server refused the write.".

- [ ] **Step 1: Failing tests**
  - `error.rs`: add both variants to the `every_variant_has_a_distinct_i18n_key` list; a test that `ApplyWriteRejected.to_string()` is non-empty and contains no `{`.
  - `memory_store.rs` (use `InMemoryStore::with_docs` and `Namespace::new` as the neighbouring tests do):
    - `bulk_write_inserts_replaces_and_deletes_by_id` — three ops of each kind on a 3-doc collection, assert final docs and `vec![Ok(()); 3]`.
    - `bulk_write_reports_missing_for_replace_and_delete_of_an_absent_id`.
    - `bulk_write_reports_duplicate_key_for_an_insert_with_an_existing_id_and_still_applies_the_rest` — ops `[Insert dup, Insert fresh]` → `[Err(DuplicateKey), Ok(())]`, fresh doc present.
    - `bulk_write_on_a_missing_collection_creates_it_for_insert`.
    - `bulk_write_of_an_empty_op_list_is_a_no_op`.
    - `bulk_write_default_is_unsupported` (a minimal store that implements only the required methods answers `Unsupported`).
- [ ] **Step 2: Run to verify failure** — `cargo test -p curlybrace-core -- --locked bulk_write every_variant` → FAIL.
- [ ] **Step 3: Implement** the types, the defaulted trait method, the two error variants (+ `i18n_key` arms, + `en.json` entries, alphabetical within `errors`), and `InMemoryStore::bulk_write` (lock once, apply ops in order against a cloned vec, commit the clone, using the existing `position`, `id_of`, `with_id` helpers; duplicate `_id` on insert → `DuplicateKey`).
- [ ] **Step 4: Run** — `cargo test --workspace --locked` → PASS.
- [ ] **Step 5: Commit** — `feat(core): batched write contract on DocumentStore`

---

### Task 3: `MongoStore::bulk_write` (Opus review: it writes to databases)

**Files:**
- Modify: `crates/core/src/mongo_store.rs` (if it passes ~600 lines, put the write code in a new `crates/core/src/mongo_write.rs` and call it from the impl)
- Create: `crates/core/tests/local_compare_apply.rs` (tier-1, `#[ignore]`, env-gated like `local_compare_store.rs`; read that file for the gating and `LocalDbs`/`support` helpers)

**Interfaces:**
- Consumes: `WriteOp`, `WriteFailure`, `WriteOutcome` (Task 2).
- Behaviour: group ops by kind; for each non-empty group send one write command on the database with `ordered: false` and no more than 500 ops (split larger groups): `insert` (`documents`), `update` (`updates: [{ q: {_id}, u: <replacement doc>, multi: false }]`), `delete` (`deletes: [{ q: {_id}, limit: 1 }]`). Write errors come back as `writeErrors: [{ index, code, .. }]` (index is within the command's list; map it back to the op's position). Code `11000` → `DuplicateKey`; any other code → `Rejected(code)`. For `update`, `n == 0` for an op (no write error but `n` per command only, not per op): since a command returns one total `n`, a single-op update command is not used; instead use `upserted`/`nModified` only for totals and detect `Missing` by issuing replace ops as updates with `q: {_id}` and checking the command's `n` equals the number of replace ops sent; when it does not, re-probe the *ids of that group* with one `find({_id: {$in: ids}}, projection {_id: 1})` and mark `Missing` for ids not found. Same for delete. A command-level error (`ok: 0`, e.g. unauthorized) → `Err(CoreError)` through the existing `db_err`, so nothing is assumed written. Never copy `errmsg` into any returned value.
- Concern to pin in a test comment: an `update` with a replacement document requires the document to contain no `$` operators; the replacement comes from user data, so field names starting with `$` are rejected by the server and surface as `Rejected(code)`.

- [ ] **Step 1: Failing tier-1 tests** in `local_compare_apply.rs` (each `#[ignore]`, start with the same gate as its neighbours; use `unique_db_name`/`LocalDbs` so only `cbit_<id>` databases are touched):
  - `bulk_write_round_trips_all_three_kinds_on_a_real_server`
  - `bulk_write_maps_a_duplicate_id_to_duplicate_key_and_keeps_going` — assert the returned `WriteFailure::DuplicateKey` and, separately, that `format!("{outcomes:?}")` does not contain the duplicate `_id` value string.
  - `bulk_write_reports_missing_ids_for_replace_and_delete`
  - `bulk_write_handles_more_than_500_ops_in_one_call` (1,203 inserts, then 1,203 deletes)
  - `bulk_write_preserves_mixed_id_types` (ObjectId, string, int64, sub-document `_id`)
  - `bulk_write_into_a_view_is_rejected` (create a view with `createView`; insert → `Rejected(_)`, not a panic)
- [ ] **Step 2: Run** — start the agent's own container per `docs/superpowers/notes/testing.md` (name `cb-it-dev-2a`), then `CURLYBRACE_IT=1 CURLYBRACE_IT_URI=<that container> cargo test -p curlybrace-core --test local_compare_apply -- --ignored --locked` → FAIL (default `Unsupported`).
- [ ] **Step 3: Implement** as specified. Keep each function < 50 lines (split: `group_ops`, `run_inserts`, `run_updates`, `run_deletes`, `map_write_errors`).
- [ ] **Step 4: Run** the tier-1 tests → PASS; `cargo test --workspace --locked` → PASS; remove `cb-it-dev-2a` afterwards (never touch `local7979`/`local265`).
- [ ] **Step 5: Commit** — `feat(core): batched writes on MongoStore through write commands`

---

### Task 4: Apply types and pure op planning

**Files:**
- Create: `crates/core/src/compare/apply.rs`; modify `crates/core/src/compare/mod.rs` (`pub mod apply;`)
- Test: `apply.rs` `mod tests`

**Interfaces:**
- Consumes: `DiffEntry`, `Category`, `Fingerprint`, `KeyTuple`, `PairId`, `WriteOp`, `WriteFailure`, `CoreError`, `fingerprint_of`, `CompareRules`.
- Produces:
  ```rust
  pub const APPLY_BATCH: usize = 500;
  pub const MAX_REPORTED_FAILURES: usize = 100;

  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum Direction { SourceToTarget, TargetToSource }   // origin -> destination of the copy
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum ApplyKind { Insert, Replace, Delete }          // what happens to the DESTINATION document
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
  pub enum IdPolicy { #[default] Keep, Generate }         // Insert only

  #[derive(Clone, Debug, PartialEq)]
  pub struct ApplyOp { pub pair: PairId, pub key: KeyTuple, pub kind: ApplyKind, pub direction: Direction }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum SkipReason { Stale, NotPlanned }
  #[derive(Clone, Debug, PartialEq)]
  pub enum OpOutcome { Applied, Skipped(SkipReason), Failed(CoreError) }

  #[derive(Clone, Debug, PartialEq)]
  pub struct OpFailure { pub key: KeyTuple, pub error: CoreError }   // key: UI only, never logged
  #[derive(Clone, Debug, Default, PartialEq)]
  pub struct ApplyReport {
      pub applied: u64, pub stale: u64, pub not_planned: u64, pub failed: u64, pub not_run: u64,
      pub failures: Vec<OpFailure>,       // first MAX_REPORTED_FAILURES
      pub more_failures: bool,
  }
  impl ApplyReport { pub fn record(&mut self, key: &KeyTuple, outcome: &OpOutcome) }

  /// Does `kind` fit a row that exists on the sides `has_source`/`has_target` (taken from the
  /// recorded `DiffEntry`)? Insert: origin yes, destination no. Replace: both. Delete: destination yes, origin no.
  pub fn kind_fits(kind: ApplyKind, direction: Direction, has_source: bool, has_target: bool) -> bool;

  /// The write for one verified op. `origin`/`dest` are the freshly re-read documents.
  pub fn plan_write(kind: ApplyKind, origin: Option<&Document>, dest: Option<&Document>, policy: IdPolicy) -> Result<WriteOp>;
  pub fn map_write_failure(f: WriteFailure) -> CoreError;  // Missing -> NotFound, DuplicateKey -> Database("duplicate key") ... see below
  ```
  `map_write_failure`: `Missing` → `CoreError::NotFound`; `DuplicateKey` → `CoreError::Database("duplicate key".into())` (static text, no value); `Rejected(_)` → `CoreError::ApplyWriteRejected`.
  `plan_write` rules: Insert → origin doc (with `_id` removed when `Generate`); Replace → origin doc rebuilt as `_id` = dest `_id` first, then origin's other fields in order (origin `_id` dropped); Delete → `WriteOp::Delete { id: dest._id }`. Missing `_id` on the destination for Replace/Delete, or a missing required document → `CoreError::InvalidDocument`.

- [ ] **Step 1: Failing tests** (pure, table-driven where possible):
  - `kind_fits` — full table: OnlyInSource row (`true,false`): `(Insert,S→T)` ✓, `(Delete,T→S)` ✓, `(Delete,S→T)` ✗, `(Insert,T→S)` ✗, `(Replace,*)` ✗; OnlyInTarget row (`false,true`): `(Delete,S→T)` ✓, `(Insert,T→S)` ✓, others ✗; Different row (`true,true`): `Replace` either direction ✓, Insert/Delete ✗.
  - `plan_write_insert_keeps_the_origin_id_by_default` and `..._drops_it_with_generate`.
  - `plan_write_replace_carries_the_destination_id_and_origin_fields` — origin `{_id: 1, email: "a", n: 2}`, dest `{_id: 9, email: "a", n: 1}` → `Replace { id: 9, doc: {_id: 9, email: "a", n: 2} }`, `_id` first.
  - `plan_write_replace_works_for_every_id_type` (ObjectId, string, Int64, Document `_id`).
  - `plan_write_delete_uses_the_destination_id`.
  - `plan_write_refuses_a_missing_destination_for_replace_and_delete`.
  - `report_counts_each_outcome_once_and_caps_failures` — 150 failed outcomes → `failed == 150`, `failures.len() == 100`, `more_failures`.
  - `map_write_failure_text_carries_no_values`.
- [ ] **Step 2: Run** — `cargo test -p curlybrace-core apply:: -- --locked` → FAIL.
- [ ] **Step 3: Implement** (all pure; no I/O).
- [ ] **Step 4: Run** → PASS; `cargo clippy -p curlybrace-core --all-targets --locked -- -D warnings` clean.
- [ ] **Step 5: Commit** — `feat(compare): apply operation types and pure write planning`

---

### Task 5: The apply engine (Opus review: writes + re-verification)

**Files:**
- Create: `crates/core/src/compare/apply_engine.rs`; modify `compare/mod.rs`
- Test: `apply_engine.rs` `mod tests` with `InMemoryStore` (also a counting/failing store defined in the test module)

**Interfaces:**
- Consumes: Task 4 types; `PairSpec`, `ResultSet`, `CancelFlag`, `fetch_one` logic (move `fetch_one` from `runtime/compare/runner.rs` into `compare/scan.rs` as `pub(crate) async fn fetch_one(...)` and import it in both places; behaviour unchanged), `fingerprint_of`, `DocumentStore::bulk_write`.
- Produces:
  ```rust
  pub struct ApplyInput<'a> {
      pub spec: &'a PairSpec,
      pub results: &'a ResultSet,
      pub source: Arc<dyn DocumentStore>,
      pub target: Arc<dyn DocumentStore>,
      pub ops: &'a [ApplyOp],          // all for spec.id, already de-duplicated
      pub policy: IdPolicy,
  }
  /// Verifies and applies `ops` in batches of `APPLY_BATCH`; `progress(done_in_pair)` after each batch.
  pub async fn apply_pair(input: ApplyInput<'_>, cancel: &CancelFlag, progress: &mut dyn FnMut(u64)) -> Result<ApplyReport>;
  ```
  Per batch: for each op, look the key up in `results` across the three categories (not found → `Failed(CoreError::NotFound)` counted as failed, never written); `kind_fits` against the recorded entry (no → `Skipped(NotPlanned)`); `fetch_one` both sides with the pair's filters; recompute both fingerprints (`fingerprint_of(doc, &spec.rules)`) and compare to `entry.source`/`entry.target` (`None` equals a missing document); any difference → `Skipped(Stale)`; else `plan_write`. Group the planned writes by destination side and call `bulk_write` once per side per batch; map each `WriteOutcome` back to its op (`Ok` → `Applied`, `Err(f)` → `Failed(map_write_failure(f))`). A whole-call `Err` marks that group's ops `Failed(err)` and continues with the next batch (the caller sees the counts; a persistent failure shows as all-failed). The cancel flag is checked before each batch; remaining ops count as `not_run`. A fetch error (`Err` from a store read) marks that op `Failed(err)`.

- [ ] **Step 1: Failing tests** (build two `InMemoryStore`s with docs, run the real compare engine `run_pair` once to obtain a real `ResultSet`, then apply; reuse the setup helpers from `engine.rs`/`runtime/compare/run_tests.rs`):
  - `copies_a_source_only_document_to_the_target` / `deletes_a_target_only_document` / `replaces_a_changed_document` / `copies_a_target_only_document_back_to_the_source` / `deletes_a_source_only_document`.
  - `replace_with_a_business_key_keeps_the_destination_id`.
  - `a_document_changed_on_the_destination_since_the_compare_is_skipped_stale` (mutate the target store after `run_pair`, before apply; assert the target document is untouched).
  - `a_document_changed_on_the_origin_since_the_compare_is_skipped_stale`.
  - `an_op_whose_kind_does_not_fit_the_row_is_skipped_not_planned_and_nothing_is_written` (e.g. `Insert` on a `Different` row; assert store unchanged via `find`).
  - `an_op_for_a_key_that_is_not_in_the_results_fails_and_writes_nothing`.
  - `one_duplicate_key_failure_does_not_stop_its_neighbours` (target already has the `_id`... build via a stale-proof route: a store wrapper whose `bulk_write` returns `Err(DuplicateKey)` for one chosen insert and delegates the rest).
  - `batches_of_500`: 1,201 inserts → a counting wrapper observes bulk_write call sizes `[500, 500, 201]`; `applied == 1201`.
  - `cancel_between_batches_reports_not_run` (cancel from the progress callback after batch 1 → `applied == 500`, `not_run == 701`).
  - `an_empty_op_list_writes_nothing_and_reports_zeroes`.
  - `a_whole_call_error_marks_that_group_failed_and_the_next_batch_still_runs`.
  - `applying_then_comparing_again_finds_no_difference` (convergence for `_id` key and for an `email` key with `include_id = false`).
  - `the_fingerprint_check_uses_the_pairs_filter` (a doc outside the filter reads as absent → `Stale` for a Replace).
- [ ] **Step 2: Run** — `cargo test -p curlybrace-core apply_engine -- --locked` → FAIL.
- [ ] **Step 3: Implement.** Functions < 50 lines: `verify_op`, `plan_batch`, `write_group`, `merge_outcomes`, `apply_pair`.
- [ ] **Step 4: Run** → PASS (+ whole workspace).
- [ ] **Step 5: Commit** — `feat(compare): apply engine with re-verification and batched writes`

---

### Task 6: Runtime command and events (Opus review: job slot, cancel, close)

**Files:**
- Create: `crates/core/src/runtime/compare/apply.rs`
- Modify: `crates/core/src/compare/protocol.rs`, `crates/core/src/runtime/compare/mod.rs` (route + `command_context`), `crates/core/src/runtime/compare/session.rs` only if the slot type needs a kind tag (prefer none), `crates/ui/locales/en.json` (`failures.compare_apply`)
- Test: `crates/core/src/runtime/compare/apply_tests.rs` (use the harness style of `run_tests.rs`/`closed_tests.rs`)

**Interfaces:**
- Produces (protocol):
  ```rust
  // CompareCommand:
  Apply { session: SessionId, run: RunId, ops: Vec<ApplyOp>, id_policy: IdPolicy },
  // CompareEvent:
  ApplyProgress { session: SessionId, run: RunId, pair: PairId, done: u64, total: u64 },
  ApplyPairDone { session: SessionId, run: RunId, pair: PairId, report: ApplyReport },
  ApplyDone { session: SessionId, run: RunId },
  ```
  A refused `Apply` answers `Failed { context: "compare_apply", error }`. `Cancel { run }` cancels an apply with the same id.
- Behaviour of `start_apply` (synchronous part, registers the slot before returning, like `runner::start`): session open and connected; `ops` non-empty; no two ops with the same `(pair, key)` (`Unsupported("duplicate operation")`); every `op.pair` has stored results (`Unsupported("no results for this pair")`); no active job (`Unsupported("a comparison is already running")` — same text as `Start`); then, before the slot is taken, read `list_collection_kinds` for each destination database used: a `View` destination → `CoreError::ApplyTargetNotWritable`; a `Timeseries` destination with any non-`Insert` op → `ApplyTargetNotWritable`. Then take the slot (`ActiveRun`), spawn `run_apply_contained` (inner task so a panic still frees the slot and still emits `ApplyDone`), which for each pair in first-appearance order calls `apply_pair`, forwarding progress as `ApplyProgress` and emitting exactly one `ApplyPairDone` per pair and finally one `ApplyDone` after the last `ApplyPairDone` (not after a close: every emission goes through `Session::emit_if_open`). A pair discarded between the request and its turn counts all its ops `failed` (`NotFound`) in its `ApplyPairDone`. On cancel the remaining pairs still get an `ApplyPairDone` with `not_run == their op count`.
- Concurrency rule: `runner::start` must also refuse while an apply holds the slot (it already refuses when the slot is occupied, so no change is expected; the test pins it).

- [ ] **Step 1: Failing tests** (`InMemoryStore`-backed session as in `run_tests.rs`):
  - `apply_writes_the_selected_rows_and_reports_per_pair` (two pairs → events in order: `ApplyProgress`*, `ApplyPairDone(p1)`, `ApplyPairDone(p2)`, `ApplyDone`).
  - `apply_is_refused_while_a_run_is_active_and_a_run_while_an_apply_is_active`.
  - `apply_for_an_unknown_or_discarded_pair_is_refused_and_writes_nothing`.
  - `apply_into_a_view_is_refused_up_front_and_writes_nothing` (store reporting `CollectionKind::View` for the destination via a wrapper).
  - `apply_to_a_timeseries_destination_allows_insert_only`.
  - `duplicate_ops_are_refused`.
  - `cancel_mid_apply_emits_pair_done_with_not_run_then_apply_done_and_frees_the_slot` (a following `Start` is accepted).
  - `closing_the_session_mid_apply_emits_nothing_more` (assert no event after `Close` is processed; connections released).
  - `a_panicking_store_still_frees_the_slot_and_ends_with_apply_done`.
  - `an_apply_before_opened_is_refused` (same rule as every command).
  - `no_event_contains_a_document_value_in_its_debug_output_except_op_failure_keys` (smoke test: format all events and assert the destination document's sentinel string value never appears).
- [ ] **Step 2: Run** — `cargo test -p curlybrace-core runtime::compare::apply -- --locked` → FAIL.
- [ ] **Step 3: Implement.** Route `Apply` synchronously in the dispatcher like `Start` (command order decides the outcome), add `command_context` arm `"compare_apply"`, add `failures.compare_apply` to `en.json`. Fix the exhaustive `match`es over `CompareCommand`/`CompareEvent` the compiler reports in the UI (the reducer ignores the new events for now; it must not panic or banner on them: add explicit no-op arms with a comment pointing at slice 2b).
- [ ] **Step 4: Run** → PASS; `cargo test --workspace --locked`; clippy clean.
- [ ] **Step 5: Commit** — `feat(runtime): apply command with progress, cancel and per-pair reports`

---

### Task 7: Tier-1 end-to-end flows, docs

**Files:**
- Modify: `crates/core/tests/local_compare_apply.rs`; `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md`; `docs/superpowers/notes/testing.md`; `README.md` only if it lists capabilities (it should still say compare is read-only in the UI: do not claim apply)
- Test: the same tier-1 file

- [ ] **Step 1: Failing tier-1 tests** (real server, own container, `cbit_<id>` databases; drive the runtime through `Command::Compare` like `local_compare_session.rs`):
  - `compare_then_apply_then_compare_again_converges` — two collections with insert/replace/delete work, key `_id`; after apply a second run reports zero differences.
  - `business_key_apply_keeps_destination_ids_and_converges` — key `email`, `include_id = false`.
  - `a_stale_document_is_skipped_on_a_real_server` — change the target between compare and apply through a second client.
  - `apply_with_generate_ids_assigns_new_ids` and the documented non-convergence assertion for an `_id`-keyed re-compare.
  - `a_unique_index_violation_is_a_per_document_failure` (unique index on `email` in the target; one insert collides; neighbours applied; report has `failed == 1` and the error text has no email).
  - `apply_refuses_a_view_destination` (real `createView`).
  - `cancel_mid_apply_on_a_large_set` (5,000 inserts; cancel after first progress; report `not_run > 0`; the target holds exactly `applied` new documents).
- [ ] **Step 2: Run** against `cb-it-dev-2a` → FAIL/PASS iteratively (implementation exists from earlier tasks; failures here are real findings).
- [ ] **Step 3: Docs.** Spec: add "Slice 2a as built" under the Apply section (the rulings above that differ from the sketch: match by destination `_id`, command-level writes, `IdPolicy`, report shape with the 100-failure cap, the two documented non-convergences, view/time-series rule, shared job slot, stored results not edited). Testing note: how to run the new tier-1 file. Remove the "slice 2 must add structured path segments" caveat from the `FieldChange.path` bullet and say segments exist.
- [ ] **Step 4: Verify** — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `scripts/check-no-db-in-ci.sh`; remove `cb-it-dev-2a`; confirm `docker ps` shows only `local7979` and `local265`.
- [ ] **Step 5: Commit** — `test(compare): tier-1 apply flows on a real server; docs for slice 2a`

---

## Self-review

- **Spec coverage (Apply section):** plan/direction/kinds (Tasks 4, 5), re-verify (5), unordered batches of 500 and per-op outcomes (2, 3, 5), replace and `_id` (4), insert and `_id` policy (4), views refused up front (6), across pairs one at a time with per-pair report (6), progress and cancel as a second job kind (6), structured paths prerequisite (1). **Deferred to 2b (UI):** selection, pending-changes list, confirmation dialog with typed target name for deletes, "Apply selection per pair", progress UI. **Deferred to 2c:** field-level `$set`/`$unset` (needs Task 1's segments and Task 2's `WriteOp`, which gains a `Set`/`Unset` variant there). **Spec deviation recorded:** replace matches by destination `_id` instead of key fields (ruling 3).
- **Placeholders:** none; where a test says "reuse the setup helpers from X" the helper exists in the named file.
- **Type consistency:** `WriteOp`/`WriteFailure`/`WriteOutcome` (Task 2) are the only write types; `ApplyOp`/`ApplyKind`/`Direction`/`IdPolicy`/`ApplyReport`/`OpFailure` (Task 4) are used unchanged in Tasks 5-7; `APPLY_BATCH = 500` is used by Tasks 3 (command split) and 5 (batching).
