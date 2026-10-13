# Data Compare, Slice 2c: Field-Level Apply Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a person copy individual fields of a changed document from one side to the other (set the field, or remove it when the origin lacks it) without replacing the whole document: tick the changed fields in the detail view, queue them, review and apply them through the same confirmed, re-verified pipeline as whole-document changes.

**Architecture:** Core gains a fifth write shape: an *update of named fields* (`ApplyKind::Fields` with the field paths on the op, a new `WriteOp::UpdateFields`, `$set`/`$unset` on the wire). The engine re-reads both documents, re-verifies the fingerprints exactly as for a replace, **re-diffs the two fresh documents** and only writes paths that are real, addressable changes, taking values from the origin's *original* document. The UI adds a per-document field selection in the detail view, a `Scope::Fields` pending change, a fields count in the review dialog and the result. Nothing new bypasses the slice-2b confirmation gate.

**Tech Stack:** Rust, `mongodb` 3.6 (write commands via `run_command`, `$set`/`$unset` update documents), `bson` 2.15, egui 0.32, the slice 2a/2b apply pipeline.

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` ("Apply (slice 2, designed now)" field-level bullet, "Slice 2a as built", "Slice 2b as built", the `FieldChange.segments` note). Issue #8. Slices 2a and 2b shipped in PRs #36 and #39.

## Decisions made while planning (rulings; each states the cost if wrong)

1. **One op per document, many fields.** After the first field of a document is written, the document no longer matches the recorded fingerprint, so a second op on the same document would always be `Stale`. A field op therefore carries *all* chosen paths of one document and is written as one update. `ApplyOp` gains `fields: Vec<Vec<PathSegment>>` (non-empty exactly when `kind == ApplyKind::Fields`); the existing "one op per (pair, key)" and duplicate rules apply unchanged. Cost if wrong: partial application per field.
2. **The engine re-diffs; it never trusts the UI's paths.** For a `Fields` op, after the normal fingerprint re-verification of both documents, the engine normalizes the two fresh documents with the pair's rules, diffs them, and requires every requested path to be exactly one reported, *addressable* change in the right direction. Any other path makes the op `Skipped(NotPlanned)` and nothing is written for that document. Cost if wrong: a UI bug could write an unexpected path.
3. **Values come from the origin's original document**, read at the path by segments, never from the normalized document (normalization can reorder fields and rewrite numbers). A path the origin lacks becomes `$unset` at the destination.
4. **Which paths are addressable** (pure function `field_write` in core, used by the engine *and* by the UI to enable a checkbox): the document key `_id` is never (immutable); a segment that is `AnyElement` never; a field name that is empty, contains `.` or starts with `$` never (it cannot be written through a dotted path); a path containing an array `Index` only for a `Changed` change whose parent array exists on both sides (setting `a.N` beyond the end would pad `null`s, and `$unset a.N` would null the element rather than remove it); `Added`/`Removed` changes only when the last segment is a field name. A set of paths where one is a prefix of another is refused as a whole (`$set` conflict). Cost if wrong: an unaddressable field shows disabled with a reason.
5. **Direction semantics follow `FieldChange`:** copying source to target: `Changed` -> `$set` the source value, `Removed` (source only) -> `$set` the source value on the target, `Added` (target only) -> `$unset` on the target. Target to source mirrors it. Only rows of the *different* category can have field ops (both documents exist).
6. **`WriteOp::UpdateFields { id, set: Vec<(Vec<PathSegment>, Bson)>, unset: Vec<Vec<PathSegment>> }`**, matched by the destination's own re-read `_id` like every other write. The store renders dotted paths from segments (the in-memory store applies the segments directly); a segment that cannot be rendered is a typed failure, never a silently wrong path. Size splitting counts the encoded `set` values. Cost if wrong: none observable.
7. **Destination rules:** a view destination is refused for field ops (any write); a time-series destination is refused (`ApplyTargetNotWritable`, only inserts are allowed there). Unchanged logic, new kind joins the "not insert" side.
8. **UI model:** field selection lives in `CompareState` per `(PairId, key)` as a set of segment paths, is only editable while that document's detail is loaded, and is cleared when the detail closes or changes, the pair is re-run/discarded, or the session drops. Queueing adds a `Scope::Fields { key, paths }` pending change (new `RowAction::CopyFieldsToTarget` / `CopyFieldsToSource`). One pending action per (pair, key) still holds: queueing fields for a key replaces any whole-document change for that key and vice versa; queueing more fields for the same key in the *same* direction merges the paths; a different direction replaces. Cost if wrong: a refused apply with a banner.
9. **Review and result:** `KindCounts` gains `fields` (the number of *documents* updated by field ops) and the dialog shows the number of fields too ("N fields in M documents"). Field ops never require a typed confirmation (nothing is deleted); the existing warnings stay; one new warning `FieldCopyKeepsRest` (the rest of each destination document is left as it is, and fields outside the compared scope are never touched). The report counts documents, as before.
10. **Out of scope:** copying fields of documents that exist on one side only; bulk field copy for a whole category ("copy this field everywhere"); array element insertion/removal; renaming fields. Cost if wrong: later slice.

## Global Constraints

- **Nothing writes without confirmation** (slice 2b rule): field ops reach `CompareCommand::Apply` only through `confirm_apply`; the exhaustive no-write test gains every new action variant.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical). No literal English in views/view-models/reducer; pseudo-locale frame tests for every new view; `i18n_usage_tests` rules apply (keys appear as string literals in production code).
- Keys, documents and field *values* are data: never logged, never in error text. Field paths are shown in the UI only.
- Immutability; files < 800 lines (aim 200-400); functions < 50 lines; nesting <= 4. `crates/core/src/compare/apply.rs` (about 440 lines) and `mongo_write.rs` (about 450) must stay under 700: put new core code in `compare/apply_fields.rs` and `mongo_write_fields.rs`; UI view-model code in new files (`field_select.rs`).
- Default `cargo test --workspace --locked` stays DB-, docker- and GPU-free. Tier-1 tests are `#[ignore]` + env-gated, need an explicit `CURLYBRACE_IT_MONGODB_URI`, use only `cbit_<id>` databases and only a `cb-it-dev-*` container the test run starts itself (`cb-it-dev-2c`, port 27121), never the user's `local7979`/`local265` or `127.0.0.1:27017`. GitHub CI never needs MongoDB (`scripts/check-no-db-in-ci.sh` stays green).
- Commits: conventional format ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Before every push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Sub-agents default to Sonnet; Tasks 1-4 (they write to databases) and 6 (the write gate) and the final review use Opus reviewers.

## Review Focus

1. **A field write that touches more than the person chose.** Wrong path rendering, a `$set` on an ancestor, array padding, `$unset` of an array element, dotted/`$`-prefixed field names, `_id`, prefix-conflicting paths, a path that is not a real change on the fresh documents. Pinned in Tasks 1-3 (table tests per rule, tier-1 on a real server).
2. **Stale handling.** Fields of one document in a single op; a document changed since the compare is `Stale`; fields outside the compared scope are never written. Pinned in Tasks 3 and 9.
3. **Original vs normalized values.** Field order, numeric normalization (`numbers_equal_across_types`), `ignore_array_order`, field scope `Only`/`AllExcept`. Pinned in Task 3.
4. **Stale selection in the UI.** The field selection must not survive a changed detail, a re-run, a discard or a session drop; a late `Detail` answer must not attach fields to the wrong key. Pinned in Tasks 5 and 6.
5. **Honest counts and gates.** Dialog counts (fields, documents) equal the ops sent; the no-write guarantee holds for the new actions; disabled reasons are shown for non-addressable fields. Pinned in Tasks 5-7.

## File Structure

```
crates/core/src/compare/
  apply_fields.rs       NEW  FieldWrite, field_write(), paths_conflict(), plan_fields(), value_at()      (Task 1)
  apply.rs              ApplyKind::Fields, ApplyOp.fields, kind_fits(Fields)                            (Task 1)
  apply_engine.rs       verify + re-diff + plan for Fields ops                                          (Task 3)
crates/core/src/store.rs              WriteOp::UpdateFields                                           (Task 1)
crates/core/src/test_support/memory_store.rs (+ memory_fields.rs)  in-memory UpdateFields             (Task 1)
crates/core/src/mongo_write.rs / mongo_write_fields.rs  UpdateFields via update commands              (Task 2)
crates/core/src/runtime/compare/apply.rs       destination rules for Fields; duplicates                (Task 4)
crates/ui/src/viewmodel/compare/
  field_select.rs       NEW  FieldSelection per (PairId,key), ChangeLine.segments/addressable            (Task 5)
  pending.rs / selection.rs  Scope::Fields, RowAction::CopyFields*, merge rules, to_command_parts      (Task 5)
  review.rs             KindCounts.fields, field counts, warning                                        (Task 5)
crates/ui/src/app/compare_fields.rs  NEW  field actions                                               (Task 6)
crates/ui/src/views/compare/detail.rs / fields.rs  checkboxes + queue buttons                          (Task 7)
crates/ui/src/views/compare/{pending,apply,apply_result}.rs  field lines, counts                        (Task 7)
crates/ui/tests/{snapshots,accessibility_apply*,local_compare_apply_flows}.rs, support/*              (Tasks 8-9)
docs / README / site                                                                                  (Task 9)
```

---

### Task 1: Field-write planning and the write contract (Opus review)

**Files:**
- Create: `crates/core/src/compare/apply_fields.rs`, `crates/core/src/test_support/memory_fields.rs`
- Modify: `crates/core/src/compare/{mod,apply}.rs`, `crates/core/src/store.rs`, `crates/core/src/test_support/{mod,memory_store}.rs`, every `ApplyOp { .. }` construction the compiler reports (core tests, the UI: add `fields: Vec::new()`)
- Test: `apply_fields.rs` `mod tests`, `memory_fields.rs` tests

**Interfaces:**
- Produces:
  ```rust
  // apply.rs
  pub enum ApplyKind { Insert, Replace, Delete, Fields }          // Fields: update named fields of an existing document
  pub struct ApplyOp { pub pair, pub key, pub kind, pub direction, pub fields: Vec<Vec<PathSegment>> }
  // kind_fits(Fields, _, has_source, has_target) == has_source && has_target

  // store.rs
  WriteOp::UpdateFields { id: Bson, set: Vec<(Vec<PathSegment>, Bson)>, unset: Vec<Vec<PathSegment>> }

  // apply_fields.rs
  #[derive(Clone, Debug, PartialEq)]
  pub enum FieldWrite { Set(Vec<PathSegment>, Bson), Unset(Vec<PathSegment>) }
  /// Is copying this change from origin to destination writable? (ruling 4/5)
  pub fn field_write(change: &FieldChange, direction: Direction, origin: &Document) -> Result<FieldWrite, Unaddressable>;
  pub enum Unaddressable { Id, AnyElement, BadName, ArrayElement, NotAChange }   // Display-free, i18n via a key fn
  impl Unaddressable { pub fn i18n_key(self) -> &'static str }                   // compare.apply.field_reason.*
  /// Refuse a set of paths where one is a prefix of another (or duplicates).
  pub fn paths_conflict(paths: &[Vec<PathSegment>]) -> bool;
  /// The value at `segments` in `doc` (the ORIGINAL origin document), if present.
  pub fn value_at<'a>(doc: &'a Document, segments: &[PathSegment]) -> Option<&'a Bson>;
  /// Builds the update for the requested `paths` from the fresh diff of the two documents.
  pub fn plan_fields(paths: &[Vec<PathSegment>], changes: &[FieldChange], direction: Direction,
                     origin: &Document, dest_id: &Bson) -> Result<WriteOp, CoreError>;
  ```
  `plan_fields` fails with `CoreError::InvalidDocument(String::new())` (static, value-free) when `paths` is empty, conflicts, or any path is not exactly one reported addressable change in the requested direction; the engine maps that to `Skipped(NotPlanned)`.

- [ ] **Step 1: Failing tests** (pure + in-memory):
  - `field_write` table (one row per rule in ruling 4/5, both directions): Changed -> Set(origin value); Removed with S→T -> Set; Added with S→T -> Unset; the mirror for T→S; `_id` -> `Id`; a segment `AnyElement` -> `AnyElement`; names `""`, `"a.b"`, `"$x"` -> `BadName`; an `Index` anywhere with kind Added/Removed -> `ArrayElement`; `Changed` with an `Index` -> Set; Unset on a trailing `Index` -> `ArrayElement`.
  - `value_at` through documents and arrays; absent path -> `None`; value comes from the original document even when it differs in type from the normalized one (e.g. `Int32` kept).
  - `paths_conflict`: duplicates, prefix (`a` and `a.b`), siblings fine, `a.0` vs `a.1` fine.
  - `plan_fields`: happy path builds `UpdateFields` with `set`/`unset` in request order; a path not in `changes` -> Err; wrong direction kind (an `Added` path copied with `Set` semantics is fine only as Unset) pinned; empty -> Err; conflicting -> Err; never includes `_id`.
  - `kind_fits(Fields, ..)` table for the three row shapes and both directions.
  - In-memory `bulk_write` with `UpdateFields`: sets nested field, sets array element by index, unsets a field, unsets nested, missing id -> `Missing`, mixed with other op kinds in one call, index-aligned outcomes, a segment path that does not exist on the stored document for `set` of a *field* creates it, for an `Index` beyond the array -> `Rejected`.
- [ ] **Step 2: Run** — `cargo test -p curlybrace-core -- --locked apply_fields memory_fields kind_fits` → FAIL.
- [ ] **Step 3: Implement.** Keep functions < 50 lines (`field_write` split by rule helper). `WriteOp` matches elsewhere (`mongo_write.rs`) get an `UpdateFields` arm that returns `Err(CoreError::Unsupported(..))` until Task 2 so the workspace compiles.
- [ ] **Step 4: Run** — `cargo test --workspace --locked` → PASS; clippy clean.
- [ ] **Step 5: Commit** — `feat(compare): plan field-level writes and extend the write contract`

---

### Task 2: `MongoStore` field updates (Opus review: writes to databases)

**Files:**
- Create: `crates/core/src/mongo_write_fields.rs`
- Modify: `crates/core/src/mongo_write.rs`, `crates/core/tests/local_compare_apply.rs` or a new `crates/core/tests/local_compare_fields.rs` (tier-1; file must stay < 800 lines)

**Interfaces:**
- Behaviour: group `UpdateFields` ops with the other kinds like replaces: one `update` command per chunk (count <= 500 and encoded bytes within the existing budget; add `element_size` for `UpdateFields` = encoded `set` values + paths + the same per-op overhead), each statement `{ q: { _id: { $eq: id } }, u: { $set: { "<dotted>": value, .. }, $unset: { "<dotted>": "" , .. } }, multi: false }` (omit empty `$set`/`$unset`; never both empty). `render_path(segments) -> Result<String, WriteFailure>`: a `Field` name that is empty, contains `.` or starts with `$`, or a path starting with `_id`, is refused **before any command is sent** with `WriteFailure::Rejected(0)`-style typed failure mapped to that op only (other ops in the call still run). `Missing` detection exactly like replaces (command `n` short -> probe ids). Write errors map as before (code 11000 impossible here; other codes `Rejected(code)`); never copy `errmsg`.

- [ ] **Step 1: Failing tier-1 tests** (`#[ignore]`, explicit `CURLYBRACE_IT_MONGODB_URI`, own container `cb-it-dev-2c` port 27121, `cbit_` databases only):
  - `update_fields_sets_nested_fields_and_leaves_the_rest`
  - `update_fields_unsets_a_field_and_keeps_siblings`
  - `update_fields_sets_an_array_element_by_index_and_only_that_one`
  - `update_fields_reports_missing_for_an_absent_id`
  - `update_fields_refuses_dotted_dollar_and_id_paths_without_touching_neighbours` (an op list `[bad, good]` -> `[Rejected, Ok]`, good applied)
  - `update_fields_mixes_with_insert_replace_delete_in_one_call_and_keeps_index_alignment`
  - `update_fields_with_large_values_splits_by_bytes` (e.g. 20 documents each updated with a ~1 MB value, all `Ok`)
  - `update_fields_matches_a_sub_document_id_literally` (the `$eq` rule: an `_id` sub-document `{"$gt": ""}` matches nothing else)
  - unit tests (no DB) for `render_path` and the update-statement builder (exact BSON), `element_size`.
- [ ] **Step 2: Run** against the own container → FAIL (Unsupported from Task 1); **Step 3: Implement**; **Step 4: Run** tier-1 + default suite → PASS; remove the container (`docker ps` shows only `local7979`/`local265`).
- [ ] **Step 5: Commit** — `feat(core): field-level updates on MongoStore through update commands`

---

### Task 3: Engine support for `Fields` ops (Opus review)

**Files:**
- Modify: `crates/core/src/compare/apply_engine.rs` (+ `apply_engine_tests.rs`, or a new `apply_fields_engine_tests.rs` wired with `#[path]`)

**Interfaces:**
- Consumes: Task 1 (`ApplyKind::Fields`, `plan_fields`, `FieldChange`), existing `verify`, `fetch_unique`, `normalize`, `diff_normalized`.
- Behaviour: a `Fields` op goes through the same checks as a replace (`kind_fits` against the recorded row: must be a *different* row; both documents re-read from the primary with the pair's filters and fingerprint-equal to the recorded ones, else `Stale`/`NotPlanned` as today). After verification the engine normalizes both fresh documents with `spec.rules`, diffs them (`diff_normalized(source, target, rules.ignore_array_order)`), and calls `plan_fields(&op.fields, &changes, op.direction, origin_original, dest_id)`; `Err` -> `Skipped(NotPlanned)`; `Ok(write)` joins the normal grouped `bulk_write` (destination side by direction). Repeated keys and duplicates behave as before (a `Fields` op and any other op on one key are duplicates).

- [ ] **Step 1: Failing tests** (InMemoryStore, real `run_pair` for the `ResultSet`):
  - `copies_one_changed_field_and_leaves_the_rest_of_the_target_document` (other differing fields stay different afterwards; a re-compare still shows the others);
  - `copies_several_fields_of_one_document_in_one_write` (counting wrapper: exactly one update for the document; `applied == 1`);
  - `unsets_a_field_that_exists_only_in_the_target` (S→T) and the mirror T→S;
  - `adds_a_field_that_exists_only_in_the_source`;
  - `a_nested_field_and_an_array_element_by_index`;
  - `an_unaddressable_path_makes_the_op_not_planned_and_writes_nothing` (`_id`, `a.b`-named field, `$x`, an `Added` array element, `AnyElement` under `ignore_array_order`);
  - `a_path_that_is_not_a_real_change_is_not_planned` (a field identical on both sides);
  - `conflicting_paths_are_not_planned`;
  - `a_document_changed_since_the_compare_is_stale_for_fields_too` (either side);
  - `fields_on_a_row_that_exists_on_one_side_only_are_not_planned`;
  - `values_come_from_the_original_document_not_the_normalized_one` (origin has `Int32(1)` and the pair has `numbers_equal_across_types`; the written value is `Int32(1)`; field order scrambled with `ignore_field_order`);
  - `field_scope_only_never_writes_outside_the_scope`;
  - `applying_the_fields_then_comparing_again_shows_only_the_unchosen_differences`;
  - accounting property: `applied+stale+not_planned+failed+not_run == ops.len()` with mixed whole-document and field ops in one call.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** (functions < 50 lines; a `plan_fields_op` helper); **Step 4: Run** → PASS (+ workspace).
- [ ] **Step 5: Commit** — `feat(compare): the apply engine writes selected fields after re-verifying`

---

### Task 4: Runtime rules for `Fields` ops

**Files:**
- Modify: `crates/core/src/runtime/compare/apply.rs` (+ `apply_tests.rs` / `apply_fields_tests.rs`)

**Behaviour:** destination kind check: a `View` destination is refused for any op (already), a `Timeseries` destination is refused for a `Fields` op (like replace/delete); an empty `fields` on a `Fields` op, or a non-empty `fields` on any other kind, is refused up front with `Unsupported("operation does not fit this category")`; `refuse_duplicates` still keys on (pair, key) via `compare_tuples` and so rejects a `Fields` op next to a whole-document op for the same key; `BulkApply` is unchanged (no field bulk, ruling 10).

- [ ] **Step 1: Failing tests:** `fields_op_to_a_view_destination_is_refused_and_writes_nothing`; `fields_op_to_a_timeseries_destination_is_refused`; `fields_op_with_no_paths_is_refused`; `a_whole_document_op_with_paths_is_refused`; `fields_and_replace_on_one_key_are_refused_as_duplicates` (also with `Int32(1)` vs `Int64(1)` keys); `fields_ops_apply_through_the_runtime_with_events_in_order` (InMemory session; `ApplyProgress`, `ApplyPairDone`, `ApplyDone`; the target field changed, the rest not); cancel/close behave as for other ops (one smoke test each).
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement**; **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(runtime): apply validates field-level operations before any write`

---

### Task 5: UI view-models: field selection, `Scope::Fields`, counts

**Files:**
- Create: `crates/ui/src/viewmodel/compare/field_select.rs` (+ `field_select_tests.rs`)
- Modify: `viewmodel/compare/{detail,selection,pending,review,mod}.rs` and their tests

**Interfaces:**
- Produces:
  ```rust
  // detail.rs
  pub struct ChangeLine { pub path: String, pub kind: &'static str, pub source_text, pub target_text,
                          pub segments: Vec<PathSegment>, pub change: ChangeKind }   // ChangeKind from core
  impl ChangeLine { pub fn copy_block(&self, direction: Direction) -> Option<Unaddressable> }  // None = copyable; pure, uses core `field_write` rules without needing the origin doc (a `can_copy(change, direction)` core helper split from `field_write`; add it in Task 1's file if not there)
  // field_select.rs
  pub struct FieldSelection { by: BTreeMap<(PairId, OrdKey), BTreeSet<PathKey>> }   // PathKey: Ord wrapper of Vec<PathSegment>
  impl FieldSelection { toggled(pair, &key, &segments), cleared(pair,&key), without_pair(pair), is_selected(pair,&key,&segments), paths(pair,&key) -> Vec<Vec<PathSegment>>, count(pair,&key) }
  // selection.rs
  RowAction::{CopyFieldsToTarget, CopyFieldsToSource}   // kind_for(Different) == Fields; offered_for(Different) unchanged (fields actions are offered only by the detail pane: add `RowAction::offered_for_fields()`)
  // pending.rs
  Scope::Fields { key: KeyTuple, paths: Vec<Vec<PathSegment>> }
  PendingList::with_fields(pair, key, direction-action, paths) -> Self     // merge rules (ruling 8)
  to_command_parts(): Scope::Fields -> one ApplyOp{kind: Fields, fields: paths}
  // review.rs
  KindCounts { insert, replace, delete, fields }; Review shows `field_count` too; Warning::FieldCopyKeepsRest (i18n compare.apply.warning.field_copy_keeps_rest)
  ```
  `CompareState` gains `field_selection: FieldSelection` (reset in `session_dropped`; `without_results` drops the pair).

- [ ] **Step 1: Failing tests** (pure): `copy_block` table (every `Unaddressable` reason incl. `Id`, `AnyElement`, `BadName`, `ArrayElement`, copyable cases, both directions); `FieldSelection` toggle/clear/without_pair/isolation per (pair,key), `OrdKey` equality for `Int32(1)`/`Int64(1)` keys and a NaN key; `PathKey` ordering is total; pending merge rules: same key + same direction merges (dedup, order stable), different direction replaces, whole-document item for the same key is replaced by fields and vice versa, category item swallows (ignored count) as for keys; `to_command_parts` emits `ApplyOp{kind: Fields, fields}` with the right direction and no `BulkApply`; `KindCounts` fields count documents; review warning appears iff a `Fields` item exists; `can_review` unaffected otherwise; property-style test (200 deterministic cases) that counts equal ops sent including field items.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** (+ en.json keys for the warning and the `compare.apply.field_reason.*` texts, literal in code); **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(ui): field selection and field-level pending changes`

---

### Task 6: Reducer: field actions (Opus review: write gate)

**Files:**
- Create: `crates/ui/src/app/compare_fields.rs` (+ tests under `app/compare_tests/fields.rs`)
- Modify: `app/compare.rs` (route only), `app/compare_apply.rs` only if the rebuild/equality code needs the new scope, `app/compare_tests/no_write.rs`

**Interfaces:**
- New actions: `ToggleField(Vec<PathSegment>)`, `ClearFieldSelection`, `AddFieldsToPending(RowAction)` (CopyFieldsToTarget/Source). Rules: act only on the loaded detail of `open_pair` (`state.compare.detail` loaded, not loading) and its key; `ToggleField` refuses a non-copyable path for the *direction-agnostic* check (refuse only paths that are unaddressable in both directions; the direction check happens at queue time per path: `AddFieldsToPending` queues only the paths that are copyable in that direction and refuses when none or when the pair is applied/not completed/busy); the field selection is cleared when the detail closes (`Back`), changes (`Select`), the category or page changes (existing `moved()`), the pair is re-run/discarded/removed, or the session drops; after a successful queue the selection for that key is cleared and the pending list updated by `with_fields`. `confirm_apply` needs no change except that the rebuild/equality (`same_changes`) must handle `Scope::Fields` (paths compared as segment lists, keys by `compare_tuples`).
- The exhaustive no-write test gains the three variants (compile-time exhaustive chain still holds).

- [ ] **Step 1: Failing tests:** field toggle/clear/queue happy path produces the right pending item and command parts; queueing refused for: no loaded detail, loading detail, applied pair, non-completed pair, busy apply, empty selection, paths all unaddressable in the direction; selection cleared on every event listed above (table test); late `Detail` for another key attaches nothing; replace-on-conflict through the reducer (whole-doc change for the key replaced; other direction replaced; same direction merged); review rebuild equality with a `Fields` item (including NaN key) -> confirm sends exactly one `Apply` whose `ops[0].fields` equal the reviewed paths; a key/path swap between review and confirm -> no command; the no-write test enumerates the new actions (no `Apply` from any of them in any state).
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement**; **Step 4: Run** → PASS (+ workspace).
- [ ] **Step 5: Commit** — `feat(ui): reducer for field-level selection and pending changes`

---

### Task 7: Views: detail checkboxes, queue buttons, pending/review/result lines

**Files:**
- Create: `crates/ui/src/views/compare/fields.rs`
- Modify: `views/compare/{detail,pending,apply,apply_result,mod}.rs`, `locales/en.json`
- Test: kittest in a new `crates/ui/tests/accessibility_fields.rs`; frames in a new `app/compare_tests/frames_fields.rs`

**Behaviour:** in the detail pane's changes grid each row gets a leading checkbox (accessible name: the field path) bound to `field_selection`; unaddressable rows show the checkbox disabled with the reason from `copy_block` as hover text (checked per both directions: disabled only if blocked in both); above the grid two buttons **Copy selected fields to target** / **Copy selected fields to source** (`AddFieldsToPending`), enabled when at least one selected path is copyable in that direction and the existing gates allow, with `on_disabled_hover_text` reasons (nothing selected, not copyable in this direction, applied, busy, running, checking). The pending panel shows "Copy N fields of one document to <destination>" lines with remove; the review dialog table gains a **Fields** column (documents) and a line "N fields in M documents"; the finished panel needs no new layout (counts are documents) but its per-pair text mentions field updates through the existing counts; Keyboard: checkboxes reachable in order; no shortcut applies anything.

- [ ] **Step 1: Failing tests:** kittest: checkbox per changed field with the path as accessible name; toggling emits `ToggleField(segments)`; blocked rows disabled with the reason text (`_id` change impossible in practice: use a fixture with a `$x`-named and a dotted field name and an `AnyElement` change); the two buttons emit `AddFieldsToPending(CopyFieldsToTarget/Source)` and show direction-specific enabled/disabled states and reasons; pending panel line for a `Fields` item with correct counts and remove emits `RemovePending`; review dialog shows the Fields column and the "N fields in M documents" line, Apply still needs the same confirmation (no typed text for field-only reviews: Apply enabled immediately, Enter/Space on open does not confirm); pseudo-locale frames cover the detail pane with checkboxes, the pending line and the dialog (`assert_translated`); `i18n_usage_tests` green.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** (+ en.json: `compare.detail.copy_fields_to_target|source`, `compare.apply.field_reason.*` (id, any_element, bad_name, array_element, not_a_change), `compare.apply.pending.fields` ("Fields: {fields}, documents: {documents}"), review column/line keys, `compare.apply.reason.no_fields`, `…not_copyable_here`); **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(ui): copy selected fields from the detail view`

---

### Task 8: Snapshots and visual review

**Files:**
- Modify: `crates/ui/tests/support/compare_apply_states.rs` (+ new `compare_field_states.rs` if it would pass ~700 lines), `crates/ui/tests/snapshots_apply.rs` (or `snapshots.rs`), new PNGs

- [ ] **Step 1:** builders scripted through the real reducer: `detail_with_field_checkboxes` (several changed fields, two ticked, one blocked with reason visible by label), `pending_with_fields` (a fields item next to a whole-document item), `review_with_fields` (Fields column, "N fields in M documents", no typed-confirmation prompt, the new warning), `finished_with_field_updates`.
- [ ] **Step 2:** add the GPU-gated `shot!` entries, generate baselines locally, **inspect every new PNG** with the image viewer (truncation, clipped text, disabled states and reasons, icons with labels, long field paths wrapping), fix layout problems found and regenerate; list defects found and fixed in the report. Run the whole local snapshot tier and the default suite (delete a PNG before regenerating when only a few pixels changed: the comparison skips tiny diffs).
- [ ] **Step 3: Commit** — `test(ui): snapshots for field-level apply`

---

### Task 9: Tier-1 flows, docs, landing page

**Files:**
- Create: `crates/ui/tests/local_compare_field_flows.rs`; modify `crates/ui/tests/support/compare.rs` (gesture helpers), the spec, `docs/superpowers/notes/testing.md`, `README.md`, `site/index.html`

- [ ] **Step 1: Tier-1 flows** (`#[ignore]`, `CURLYBRACE_IT=1` **and** an explicit `CURLYBRACE_IT_MONGODB_URI` (skip with a message otherwise), own container `cb-it-dev-2c` port 27121, `cbit_` databases only, gestures only: open a pair, open a row's detail, tick fields, click the queue button, Review changes…, Apply, wait for Finished, assert through `f.db`):
  - `copy_one_field_to_the_target_and_leave_the_rest`
  - `copy_two_fields_of_one_document_in_one_apply_and_rerun_shows_the_rest`
  - `remove_a_field_that_only_the_target_has`
  - `a_field_in_a_nested_document_and_an_array_element_by_index`
  - `a_document_changed_after_the_compare_is_reported_stale_and_left_alone`
  - `a_field_with_a_dot_in_its_name_is_shown_disabled_and_cannot_be_queued`
  - `field_and_whole_document_changes_for_different_documents_in_one_apply`
  Failures are real findings; do not weaken assertions.
- [ ] **Step 2: Docs.** Spec: "Slice 2c as built" under Apply (the rulings: one op per document with many fields, engine re-diff, original values, addressability rules, `$unset` semantics, destination rules, UI model, review/result changes, limits: no field-level bulk, no array element insertion/removal, no rename; fields outside the compared scope are never touched; `WriteOp::UpdateFields`, `ApplyKind::Fields`, `ApplyOp.fields`); update the Apply bullet "Field-level operations" to say built. Testing note: the new tier-1 file. README and `site/index.html`: compare can copy individual fields as well; roadmap line becomes "Still to build: exporting reports" (and the plan/export wording from slice 3 as it stands); keep the "New in the next release" wording until a release is cut; no Studio 3T assets.
- [ ] **Step 3: Verify** — fmt, clippy `-D warnings`, `cargo test --workspace --locked`, `scripts/check-no-db-in-ci.sh`, the new tier-1 file and the existing `local_compare_apply_flows` against `cb-it-dev-2c`; remove the container; `docker ps` shows only `local7979` and `local265`.
- [ ] **Step 4: Commit** — `test(ui): tier-1 field-level apply flows; docs for slice 2c`

---

## Self-review

- **Spec coverage (field-level bullet):** copy a single field path either direction, `$set` or `$unset` when absent at the origin, only that field changed, same fingerprint re-check, reported like any operation (Tasks 1-4, 7); builds from structured paths (2a) and the pending-list model (2b) (Tasks 5-6). **Deferred:** field-level bulk, array element insertion/removal, rename (ruling 10).
- **Placeholders:** none; where a test says "mixed with other op kinds" the tests list the combination.
- **Type consistency:** `ApplyKind::Fields` and `ApplyOp.fields` (Task 1) are produced only by `PendingList::to_command_parts` (Task 5) and consumed by `plan_fields` (Tasks 1, 3) and the runtime checks (Task 4); `WriteOp::UpdateFields` (Task 1) is implemented by `memory_fields.rs` (Task 1) and `mongo_write_fields.rs` (Task 2); `Unaddressable`/`copy_block` (Tasks 1, 5) back the disabled reasons (Task 7).
