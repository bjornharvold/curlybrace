# Bulk update and delete by filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user update or delete every document matching the current filter, behind a count, an exact 3-document preview, a typed confirmation and a report, with the empty-filter and read-only rules enforced in the core.

**Architecture:** New request/report types and five new `DocumentStore` methods in `curlybrace-core` (guarded by `ReadOnlyStore`, empty-filter rule in the core), four new runtime commands answered by two new events, and a pure view-model plus a modal flow in `curlybrace-ui` that reuses the existing write guard (`Guard::Confirmed`) and `typed_text_matches`. The "after" preview runs the real update inside a transaction that is always aborted.

**Tech Stack:** Rust workspace (`core`, `ui` egui, `app`, `testkit`), `mongodb` 3.6, `bson` 2.15, `tokio`. Local-server integration tests via `scripts/integration-tests.sh` (`CURLYBRACE_IT=1`).

**Spec:** `docs/superpowers/specs/2026-11-09-bulk-update-delete-design.md`

## Global Constraints

- Bulk delete always requires the typed confirmation; the typed text is the collection name, compared with `typed_text_matches` (trimmed, exact, a blank requirement never passes).
- Bulk update requires the typed confirmation when the count is `> BULK_UPDATE_TYPED_CONFIRM_THRESHOLD` (`100`, a named constant; 100 itself does not require it), the count is unavailable, the connection is production, the scope is all-documents, or the update preview has no after-state.
- The preview shows exactly 3 documents (`BULK_PREVIEW_DOCS = 3`).
- Empty filter = the parsed filter document has no keys. The core rejects it unless `MatchScope::AllDocuments`; `AllDocuments` with a non-empty filter is also rejected.
- Only the filter is used; sort, skip and limit of the view are ignored, and the dialog says so.
- Never operate from a projected view; the core refuses writes on a read-only connection; `preview_update` is classified as a write.
- The preview never commits; every path aborts the transaction.
- `max_time` applies to the count and the preview reads, not to the write.
- Outcome after a connection loss, timeout or cancel is `BulkOutcome::Unknown`, never a guess; no follow-up count.
- Server text and document values are never copied into errors or reports.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json`; report new keys when done.
- Undo is out of scope; every confirmation says the action cannot be undone.
- Immutability: new values, no in-place mutation. Files under 800 lines, functions under 50.
- Conventional commits, subject lowercase imperative; commit trailer `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`.

## Review Focus

- A filter like `{ "$expr": true }` or `{ "x": { "$exists": false } }` that matches everything is an explicit user filter and passes; `{}` and a whitespace-only query bar never do.
- A count of exactly 100 does not require typed confirmation for update; 101 does; a timed-out count does.
- A document that changes between preview and run: the report's counts come from the run, not from the preview.
- Disconnecting, a superseded connect epoch or a reconnect between "parked" and "confirm" must not send a bulk write to the new connection.
- A pipeline update with an unsupported stage (`$lookup`, `$out`, `$merge`) is rejected before any server call.
- Preview on a capped collection, a standalone server, or with a write conflict degrades to the before-only preview (never a failed update, never a guessed after).
- Cancel when the server finished first: the report must be the real counts, not "unknown".
- A production + all-documents + delete run asks for the typed name exactly once and shows one dialog.

---

## File Structure

- Create `crates/core/src/bulk.rs`: request, scope, report and preview types; filter and update validation. (~250 lines)
- Create `crates/core/src/mongo_bulk.rs`: `MongoStore` implementation of count, update many, delete many, cancel and the transactional preview. (~300 lines)
- Modify `crates/core/src/store.rs`, `read_only_store.rs`, `read_only_store_tests.rs`, `mongo_store.rs`, `error.rs`, `lib.rs`.
- Create `crates/core/src/runtime/bulk.rs`: handlers for the four bulk commands. Modify `runtime/mod.rs`, `runtime/command.rs`, `runtime/handlers.rs`, `runtime/config.rs`, `runtime/read_only_tests.rs`.
- Create `crates/core/tests/local_bulk.rs`: integration tests.
- Create `crates/ui/src/viewmodel/bulk_flow.rs` (+ `bulk_flow_tests.rs`): pure step state.
- Create `crates/ui/src/app/bulk_actions.rs` (+ `bulk_tests.rs`): reducer arms. Modify `app/reducer.rs`, `app/state.rs`, `app/write_guard.rs`, `app/mod.rs`.
- Create `crates/ui/src/views/bulk_dialog.rs`; modify `views/documents.rs` (entry point), `views/write_warning.rs` (`WriteKind` messages), `views/mod.rs`, `locales/en.json`.
- Create `crates/ui/tests/accessibility_bulk.rs` and screenshot baselines following `accessibility_index_builder.rs` and `tests/support/indexes_shots.rs`.

---

### Task 1: Core types, validation and errors

**Files:**
- Create: `crates/core/src/bulk.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod bulk;`), `crates/core/src/error.rs`

**Interfaces:**
- Produces (all `pub`, `Clone + Debug + PartialEq`):
  - `enum MatchScope { Filtered, AllDocuments }`
  - `enum UpdateSpec { Operators(Document), Pipeline(Vec<Document>) }`
  - `struct BulkUpdateRequest { filter: Document, update: UpdateSpec, upsert: bool, scope: MatchScope }`
  - `struct BulkDeleteRequest { filter: Document, scope: MatchScope }`
  - `struct BulkRunId(pub uuid::Uuid)`
  - `enum BulkOutcome { Applied { matched: u64, modified: u64, upserted: Vec<Bson>, deleted: u64 }, Unknown { cancelled: bool } }`
  - `struct PreviewDoc { before: Document, after: Option<Document> }`
  - `enum PreviewGap { TransactionsUnavailable, Conflict, NoMatch }` and `struct UpdatePreview { docs: Vec<PreviewDoc>, gap: Option<PreviewGap> }`
  - `const BULK_PREVIEW_DOCS: usize = 3;`
  - `fn check_scope(filter: &Document, scope: MatchScope) -> Result<()>`
  - `fn parse_update(text: &str) -> Result<UpdateSpec>` (editor JSON; an object whose every key starts with `$` is `Operators`, a JSON array is `Pipeline`)
  - `fn validate_update(spec: &UpdateSpec) -> Result<()>`
- New `CoreError` variants with i18n keys: `BulkEmptyFilter` (`errors.bulk_empty_filter`), `BulkScopeMismatch` (`errors.bulk_scope_mismatch`), `BulkInvalidUpdate` (`errors.bulk_invalid_update`), `BulkPreviewUnavailable` (`errors.bulk_preview_unavailable`). Add all four to the distinct-key list in `error.rs` tests and to `en.json`.

- [ ] **Step 1: Write failing tests** in `bulk.rs` `#[cfg(test)]`: `empty_filter_without_flag_is_rejected`, `all_documents_with_empty_filter_passes`, `all_documents_with_nonempty_filter_is_rejected`, `expr_true_filter_counts_as_explicit`, `operators_update_requires_every_key_to_be_an_operator` (`{ "a": 1 }` fails, `{ "$set": { "a": 1 } }` passes, mixed fails), `empty_operator_document_is_rejected`, `pipeline_allows_set_unset_project_replace_stages_only` (`$set`, `$addFields`, `$unset`, `$project`, `$replaceRoot`, `$replaceWith` pass; `$lookup`, `$out`, `$merge`, `$match` fail), `empty_pipeline_is_rejected`, `parse_update_reads_object_and_array`, `errors_never_echo_values` (the error `Display` for `{ "$set": { "secret": "hunter2" } }` rejected input does not contain `hunter2`).
- [ ] **Step 2: Run** `cargo test -p curlybrace-core bulk::` → FAIL (module missing).
- [ ] **Step 3: Implement** the types and functions above in `bulk.rs`; add the four error variants and keys.
- [ ] **Step 4: Run** `cargo test -p curlybrace-core bulk:: error::` → PASS.
- [ ] **Step 5: Commit** `feat(bulk): add bulk update and delete request types with filter and update validation`

---

### Task 2: Store trait methods and the read-only guard

**Files:**
- Modify: `crates/core/src/store.rs`, `crates/core/src/read_only_store.rs`, `crates/core/src/read_only_store_tests.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces, on `DocumentStore` (each with a default that returns `CoreError::Unsupported("bulk writes".into())`, like the other optional methods):
  - `async fn count_matching(&self, ns: &Namespace, filter: &Document, max_time: Duration) -> Result<Option<u64>>` (`None` = timed out; a read)
  - `async fn preview_update(&self, ns: &Namespace, req: &BulkUpdateRequest) -> Result<UpdatePreview>` (write-classified)
  - `async fn update_many(&self, ns: &Namespace, req: &BulkUpdateRequest, run: BulkRunId) -> Result<BulkOutcome>` (write)
  - `async fn delete_many(&self, ns: &Namespace, req: &BulkDeleteRequest, run: BulkRunId) -> Result<BulkOutcome>` (write)
  - `async fn cancel_bulk(&self, run: BulkRunId) -> Result<()>` (best effort; not guarded, killing your own op is not a write)

- [ ] **Step 1: Write failing tests** in `read_only_store_tests.rs`: extend `WRITES` to include `"preview_update"`, `"update_many"`, `"delete_many"`; give `RecordingStore` counters returning non-default values; add `locked_store_refuses_bulk_writes_without_calling_inner` (each returns `CoreError::ReadOnly`, inner count 0), `unlocked_store_forwards_bulk_methods`, and `count_matching_and_cancel_are_forwarded_when_locked`. The existing `every_write_method_is_covered` must name the new methods.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core read_only_store` → FAIL (compile).
- [ ] **Step 3: Implement** the trait methods and forward all five explicitly in `ReadOnlyStore`, guarding `preview_update`, `update_many`, `delete_many`. Update the WRITE METHODS comment in `store.rs`.
- [ ] **Step 4: Run** the same command → PASS.
- [ ] **Step 5: Commit** `feat(bulk): add bulk store methods and refuse them on read-only connections`

---

### Task 3: MongoStore count, update many, delete many (core enforcement)

**Files:**
- Create: `crates/core/src/mongo_bulk.rs`; Modify: `crates/core/src/mongo_store.rs` (delegate), `crates/core/src/lib.rs`
- Test: `crates/core/tests/local_bulk.rs`

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: `MongoStore` implements `count_matching`, `update_many`, `delete_many`. Each write calls `check_scope` and `validate_update` **before any server call**. Writes are sent as raw `update` / `delete` commands with `comment: <run id>` so Task 5 can find them; a driver error or a reply that cannot be trusted maps to `BulkOutcome::Unknown { cancelled: false }`; server `errmsg` is never copied.

- [ ] **Step 1: Write failing integration tests** (`#[ignore]`, `local_enabled`, `local_mongo()` as in `local_mongo_crud.rs`): `count_matching_counts_the_filter`, `count_matching_returns_none_when_max_time_expires` (use a tiny `max_time` against a seeded collection with an unindexed `$where`-free slow filter, or assert the `None` mapping with a unit test of the error-code mapping if the server is too fast), `update_many_sets_every_match_and_reports_counts` (seed 5, 3 match), `update_many_upsert_reports_the_new_id`, `update_many_pipeline_adds_a_field`, `update_many_with_empty_filter_and_filtered_scope_touches_nothing` (error, collection unchanged), `update_many_all_documents_updates_all`, `delete_many_removes_matches_and_reports_deleted`, `delete_many_empty_filter_without_flag_deletes_nothing`, `invalid_update_is_rejected_before_the_server` (collection unchanged).
- [ ] **Step 2: Run** `CURLYBRACE_IT=1 scripts/integration-tests.sh local_bulk` → FAIL.
- [ ] **Step 3: Implement** in `mongo_bulk.rs`; `count_matching` uses `count_documents` with `max_time`, mapping the `MaxTimeMSExpired` (code 50) error to `Ok(None)`.
- [ ] **Step 4: Run** the same → PASS.
- [ ] **Step 5: Commit** `feat(bulk): update and delete many with core-enforced scope rules`

---

### Task 4: Transactional update preview

**Files:**
- Modify: `crates/core/src/mongo_bulk.rs`; Test: `crates/core/tests/local_bulk.rs`, `crates/core/tests/docker_bulk_preview.rs` (standalone-degradation cases, following `docker_mongo_sessions.rs`)

**Interfaces:**
- Consumes: Task 3.
- Produces: `MongoStore::preview_update` per the spec's "Preview mechanics": `find` (limit `BULK_PREVIEW_DOCS`, filter alone); if none match and `upsert`, run the upsert once in the transaction with the real filter and read the inserted document; otherwise run the real update with `_id: {$in: ids}` merged into the filter (`$and`), re-read those ids inside the transaction, **always abort**. Transaction unavailable (standalone, capped collection, old server, any transaction start error) → `UpdatePreview { docs: <before only>, gap: Some(TransactionsUnavailable) }`; a write conflict → `gap: Some(Conflict)`; nothing matched and no upsert → `gap: Some(NoMatch)`. An abort that fails returns `CoreError::BulkPreviewUnavailable` and is logged at `error` level.

- [ ] **Step 1: Write failing tests**: `preview_after_matches_what_update_many_then_produces` (for `$set`, `$inc`, `$push`, and a pipeline update: preview the 3, run `update_many`, compare each previewed after to the stored document), `preview_leaves_the_collection_unchanged` (including after the update errors mid-preview, e.g. `$inc` on a string field), `upsert_with_no_match_previews_the_inserted_document_and_inserts_nothing`, `preview_with_no_match_and_no_upsert_reports_nomatch`, `preview_on_standalone_degrades_to_before_only` (docker standalone), `preview_on_capped_collection_degrades`.
- [ ] **Step 2: Run** `CURLYBRACE_IT=1 scripts/integration-tests.sh local_bulk preview` → FAIL. (The transaction tests need a replica set; document in the test file header how to start one, and have them skip with a clear message on a standalone server.)
- [ ] **Step 3: Implement** with `client.start_session()`, `start_transaction()`, and an abort guard that runs on every path (an `async` block whose result is matched, then `abort_transaction` awaited before returning).
- [ ] **Step 4: Run** the same → PASS.
- [ ] **Step 5: Commit** `feat(bulk): preview an update's exact result in an aborted transaction`

---

### Task 5: Cancel

**Files:**
- Modify: `crates/core/src/mongo_bulk.rs`; Test: `crates/core/tests/local_bulk.rs`

**Interfaces:**
- Consumes: Task 3 (`comment` carries the run id).
- Produces: `MongoStore::cancel_bulk(run)`: find the operation by `comment` (`currentOp` on `admin`, matching `command.comment`), `killOp` it. A write that was killed returns `BulkOutcome::Unknown { cancelled: true }`. If the write completes before the kill lands, its real `Applied` counts are returned (the cancel is then a no-op). A deployment that does not allow `currentOp`/`killOp` for this user makes `cancel_bulk` return `Err(CoreError::Unsupported(..))`; the run continues and the UI says cancel was not possible.

- [ ] **Step 1: Write failing tests**: `cancel_stops_a_slow_update_and_reports_unknown_cancelled` (a pipeline update using `$function` or a `$where` sleep over a few hundred documents; cancel after the first matches begin; assert `Unknown { cancelled: true }`), `cancel_after_completion_returns_the_real_counts`, `cancel_of_an_unknown_run_is_ok`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(bulk): cancel a running bulk write best-effort`

---

### Task 6: Runtime commands and events

**Files:**
- Create: `crates/core/src/runtime/bulk.rs`; Modify: `runtime/mod.rs` (Command/Event), `runtime/command.rs`, `runtime/handlers.rs`, `runtime/config.rs`, `runtime/read_only_tests.rs`; Test: `crates/core/src/runtime/bulk_tests.rs`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces:
  - `Command::BulkCount { session, ns, filter: Document, request: u64 }` (read)
  - `Command::BulkPreview { session, ns, request: BulkUpdateRequest, request_id: u64 }` (write-classified, update only; delete previews use `Find` with limit 3 from the UI)
  - `Command::BulkUpdate { session, ns, request: BulkUpdateRequest, run: BulkRunId }` and `Command::BulkDelete { session, ns, request: BulkDeleteRequest, run: BulkRunId }` (writes)
  - `Command::BulkCancel { session, run: BulkRunId }` (not a write)
  - `Event::BulkCount { session, ns, request: u64, count: Option<u64> }`, `Event::BulkPreview { session, ns, request_id: u64, outcome: Result<UpdatePreview, CoreError> }`, `Event::BulkFinished { session, ns, run: BulkRunId, outcome: Result<BulkOutcome, CoreError> }`, `Event::BulkCancelResult { session, run: BulkRunId, outcome: Result<(), CoreError> }`
  - `RuntimeConfig.bulk_write_timeout` default `BULK_WRITE_TIMEOUT = 1 hour` (like `INDEX_BUILD_TIMEOUT`), used by `BulkUpdate`/`BulkDelete` in the `limit` match in `handlers.rs`.
  - `is_write`: `BulkPreview`, `BulkUpdate`, `BulkDelete` true; `BulkCount`, `BulkCancel` false. `context()` strings `bulk_count`, `bulk_preview`, `bulk_update`, `bulk_delete`, `bulk_cancel`. `session()` returns the session for all five.
  - A bulk write that times out or loses its connection answers `Event::BulkFinished` with `Ok(BulkOutcome::Unknown { cancelled: false })`, not `Event::Failed`, and does not run the settle-by-ping path that turns a lost write into `WriteOutcomeUnknown`. `BulkPreview` never reports a write-unknown error; its failures are `Err` outcomes shown inline.

- [ ] **Step 1: Write failing tests** with the existing fake-store runtime test helpers (see `read_only_tests.rs`, `index_tests.rs`): `bulk_commands_are_classified_as_writes_or_reads`, `read_only_session_refuses_preview_update_and_delete_before_the_store` (store call count 0, `Failed { error: ReadOnly }`), `bulk_count_is_allowed_on_read_only`, `bulk_update_answers_finished_with_the_store_outcome`, `bulk_write_timeout_answers_unknown_not_failed`, `superseded_epoch_does_not_run_a_bulk_write`, and extend `read_only_tests.rs` `classify` so `is_write_agrees_with_what_the_read_only_store_refuses` covers the new commands.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core runtime::bulk runtime::read_only` → FAIL.
- [ ] **Step 3: Implement** the handlers in `runtime/bulk.rs` and wire them in `data_command`; add the config field and constant.
- [ ] **Step 4: Run** `cargo test -p curlybrace-core` → PASS.
- [ ] **Step 5: Commit** `feat(bulk): add bulk commands and events to the runtime`

---

### Task 7: Bulk flow view-model (pure)

**Files:**
- Create: `crates/ui/src/viewmodel/bulk_flow.rs`, `crates/ui/src/viewmodel/bulk_flow_tests.rs`; Modify: `viewmodel/mod.rs`

**Interfaces:**
- Consumes: core types from Task 1; `typed_text_matches` from `viewmodel/typed_confirmation.rs`.
- Produces:
  - `const BULK_UPDATE_TYPED_CONFIRM_THRESHOLD: u64 = 100;`
  - `enum BulkKind { Update, Delete }`, `enum BulkStep { Define, Preview, Confirm, Running, Report }`
  - `struct BulkFlow { kind, ns: Namespace, filter: Document, update_text: String, upsert: bool, all_documents_ticked: bool, count: CountState, preview: PreviewState, typed: String, step: BulkStep, production: bool, run: Option<BulkRunId> }` with `enum CountState { Pending, Known(u64), Unavailable }`
  - Pure methods returning new values: `BulkFlow::open(kind, ns, filter, production) -> BulkFlow`, `.with_update_text(String)`, `.with_upsert(bool)`, `.with_all_documents(bool)`, `.with_count(CountState)`, `.with_preview(PreviewState)`, `.with_typed(String)`, `.advanced() -> BulkFlow` (Define→Preview→Confirm), `.running(BulkRunId)`, `.reported(Result<BulkOutcome, CoreError>)`.
  - `fn needs_all_documents_step(&self) -> bool` (filter has no keys), `fn needs_typed(&self) -> bool` (rules in Global Constraints), `fn can_run(&self) -> bool` (all-documents ticked when needed, typed matches the collection name when needed, count not Pending, update parses and validates for Update), `fn scope(&self) -> MatchScope`.

- [ ] **Step 1: Write failing tests**: threshold boundary (`99`, `100`, `101`, `Unavailable`); `delete_always_needs_typed`; `update_needs_typed_on_production`; `update_needs_typed_when_preview_has_no_after`; `empty_filter_requires_the_all_documents_tick_before_can_run`; `typed_must_match_collection_name_exactly_trimmed`; `blank_collection_name_never_confirms`; `scope_is_all_documents_only_when_filter_empty_and_ticked`; `advance_is_blocked_while_update_text_is_invalid`; `reported_records_unknown_outcome`; every updater returns a new value and leaves the original unchanged.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui bulk_flow` → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(bulk): add the pure bulk flow view-model`

---

### Task 8: Reducer wiring, gating and the write guard

**Files:**
- Create: `crates/ui/src/app/bulk_actions.rs`, `crates/ui/src/app/bulk_tests.rs`; Modify: `app/reducer.rs` (`Input::Bulk(BulkAction)` and the Event arms), `app/state.rs` (`bulk: Option<BulkFlow>`), `app/write_guard.rs` (`WriteKind::BulkUpdate`, `WriteKind::BulkDelete`), `app/mod.rs`, `views/write_warning.rs` (`message_key` arms)

**Interfaces:**
- Consumes: Tasks 6–7; `gated`, `mutation`, `Guard::Confirmed`, `Pending` from `app/actions.rs` / `write_guard.rs`.
- Produces: `enum BulkAction { Open(BulkKind), SetUpdateText(String), SetUpsert(bool), SetAllDocuments(bool), SetTyped(String), Next, Back, Run, Cancel, Close }`.
  - `Open` is refused with a banner when the active session is read-only (`Banner::ReadOnly`), and does nothing when the page is projected (`state.page_is_projected()`) or another bulk flow or a parked write is open. It opens the flow on the current query's **filter only** and sends `Command::BulkCount`.
  - `Next` from Define sends `BulkPreview` (update) or `Find` limit 3 (delete) and moves to Preview; from Preview moves to Confirm.
  - `Run` is a no-op unless `can_run()`. It sends `BulkUpdate`/`BulkDelete` through the write guard with `Guard::Confirmed` (the bulk dialog's typed confirmation is the acknowledgement; a production connection therefore sees one dialog, not the parked-write warning on top).
  - A reconnect (changed connect epoch), disconnect or closed session closes the flow; `Run` with a stale epoch sends nothing.
  - `Event::BulkFinished` moves to Report and triggers the same re-query as `Event::Mutated`.

- [ ] **Step 1: Write failing tests**: `open_is_refused_on_read_only_with_the_banner`, `open_does_nothing_on_a_projected_page`, `open_uses_the_filter_and_ignores_sort_skip_and_limit`, `open_sends_count_for_the_filter`, `run_without_typed_confirmation_sends_nothing`, `run_with_confirmation_sends_bulk_delete_once`, `run_on_production_sends_without_a_second_warning` (`write_warning` stays `None`), `epoch_change_closes_the_flow_and_run_sends_nothing`, `finished_unknown_shows_unknown_report_and_requeries`, `cancel_while_running_sends_bulk_cancel`, `escape_and_close_cancel_from_every_step_except_running`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui bulk` → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(bulk): wire the bulk flow into the reducer behind the write guard`

---

### Task 9: Dialog, entry point, i18n, accessibility and baselines

**Files:**
- Create: `crates/ui/src/views/bulk_dialog.rs`, `crates/ui/tests/accessibility_bulk.rs`, `crates/ui/tests/support/bulk_shots.rs`; Modify: `views/documents.rs` (entry buttons "Update matching" / "Delete matching", disabled with a reason when read-only or projected), `views/mod.rs`, `crates/ui/locales/en.json`, snapshot baselines under the existing screenshot directory

**Interfaces:**
- Consumes: Tasks 7–8; `modal_frame`, `filled_button`, `text_button`, `focus_cancel_unless_focused` and the typed-confirmation field pattern from `views/index_dialogs.rs`.
- Produces: `show_bulk_dialog(ctx, flow: &BulkFlow, i18n: &I18n) -> Option<BulkAction>`. Cancel owns initial focus; Enter never confirms; the destructive button is disabled until `can_run()`. Steps render: Define (update document or pipeline editor, upsert checkbox), Preview (count, "matching the filter, not just this page", 3 before/after or 3 documents to be removed, the no-after note when the preview has a gap, the "this cannot be undone" warning), Confirm (all-documents tick when needed, typed field naming the collection), Running (Cancel button), Report (matched, modified, deleted, upserted, or the unknown-outcome text with a cancelled variant). New i18n keys under `bulk.*` and `errors.bulk_*`; every label, tooltip, aria-label and title uses one.

- [ ] **Step 1: Write failing tests**: in `accessibility_bulk.rs` (pattern of `accessibility_index_builder.rs`) every control of every step has an accessible name and the destructive button reports disabled until confirmed; `bulk_dialog_strings_are_all_i18n_keys` via the existing `i18n_usage_tests.rs` mechanism; `entry_buttons_are_disabled_with_a_reason_when_read_only_or_projected` (pattern of `accessibility_read_only_reasons.rs`); screenshot tests for Define, Preview (with and without after-state), Confirm (all documents + typed), Report (unknown) in light and dark.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui accessibility_bulk i18n_usage` → FAIL.
- [ ] **Step 3: Implement** the dialog and entry points; add all keys to `en.json`.
- [ ] **Step 4: Generate baselines** the way the repo's screenshot tests document (see `tests/snapshots.rs`), review them visually, then run `cargo test -p curlybrace-ui` → PASS.
- [ ] **Step 5: Commit** `feat(bulk): add the bulk update and delete dialog`

---

### Task 10: Whole-feature verification

- [ ] **Step 1:** `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings` → clean.
- [ ] **Step 2:** `cargo test --workspace` → PASS; `CURLYBRACE_IT=1 scripts/integration-tests.sh` against a replica-set server → PASS.
- [ ] **Step 3:** Check coverage of the new modules is at least 80% (`cargo llvm-cov` if the repo uses it, else report which new branches are untested).
- [ ] **Step 4:** Run the app (`run` skill) against a local replica set and walk: delete with empty filter (blocked until ticked and typed), update with preview, standalone degradation, cancel, read-only and production connections. Report what was observed.
- [ ] **Step 5:** Open a follow-up issue "bulk edit: undo" and reference it from #51; list the new i18n keys in the PR description. PR targets the integration branch per the user's git rules.
