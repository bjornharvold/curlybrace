# Copy and Paste Between Connections Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Copy databases, collections and documents between open connections, or within one, with a confirm dialog before every paste.

**Architecture:** A pure core copy engine (`core/src/copy/`) streams one collection at a time in `_id` order using the compare engine's `scan_sorted` and keyset helpers, and writes through `bulk_write` in `APPLY_BATCH` batches under a conflict policy. A new `Command::Copy` / `Event::Copy` runtime family, modelled on the Compare apply runtime but working on two **open sessions'** stores, drives it with preview counts (from `compare::engine::run_pair`), progress and cancel. The UI adds an in-app clipboard of *references*, sidebar multi-select, Copy/Paste menu entries, a paste dialog and a progress view; documents, indexes/options and drag and drop follow as separate slices.

**Tech Stack:** Rust, tokio, the `mongodb` driver and `bson`, egui/eframe, Transloco-style JSON i18n (`crates/ui/locales/en.json`), testkit-gated integration tests.

**Spec:** `docs/superpowers/specs/2026-10-25-copy-between-connections-design.md`

## Global Constraints

- Every user-visible string is a key in `crates/ui/locales/en.json` (nested objects); the `i18n_usage_tests` must stay green; report new keys.
- Document values and `_id` values are never logged and never in errors, reports or event payloads that are logged; failures carry the key for the UI only, bounded by `MAX_REPORTED_FAILURES` (100).
- Writes use `APPLY_BATCH` (500) per `bulk_write`; memory stays bounded (no whole-collection `Vec`).
- Read-only targets are refused **in the core** (`CoreError::ReadOnly`, `ReadOnlyStore`); the UI only disables entries.
- Nothing is ever deleted from the target by any policy. Skip = insert new only; Overwrite = replace existing only (whole document); Upsert = replace existing and insert new. Default Skip.
- `admin`, `local`, `config` and `system.*` are never copied; views are skipped.
- Hand-picked document selection is capped at 10,000 `_id`s.
- Immutable data: new values, no in-place mutation of shared state; files under 800 lines; functions under 50 lines.
- Tests: unit tests in sibling `*_tests.rs` files; shared fixtures in `core/src/test_support/mock_objects.rs` (add factories there, do not `mock()` value types); integration tests are gated (`local_*` need `CURLYBRACE_IT=1` and a local mongod, `docker_*` need docker), see `crates/testkit/src/gate.rs`.
- Verify with `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked`.
- Commits: conventional (`feat(copy): ...`), end with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Work happens on branch `feat/copy-between-connections`; PRs target `main` only through the project's usual flow.

## Review Focus

1. A collection pasted onto the database it came from without a new name (same namespace): must be refused or renamed, never copied onto itself. (Task 2 and Task 8.)
2. `_id`s of mixed BSON types in one collection (ObjectId, string, int, compound): none skipped or duplicated across batch boundaries. (Task 4.)
3. Cancel in the middle of a batch: the reported "written" count equals what is actually in the target. (Task 4.)
4. The source session reconnects, or the target becomes read-only, mid-run: the run fails with the exact written count and writes nothing more. (Task 6.)
5. Names with dots, spaces, unicode or a leading `system.`, and documents with `$`-prefixed or dotted field names: names are validated at the boundary and documents are copied faithfully. (Task 2 and Task 4.)

---

## Slice 1: engine, runtime, collection and database paste

### Task 1: Conflict policy and batch planning

**Files:**
- Create: `crates/core/src/copy/mod.rs`, `crates/core/src/copy/policy.rs`, `crates/core/src/copy/policy_tests.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod copy;`)

**Interfaces:**
- Produces: `pub enum ConflictPolicy { Skip, Overwrite, Upsert }` (`Default` = `Skip`); `pub struct ExistingIds` built with `ExistingIds::from_ids(Vec<Bson>)` and `fn contains(&self, id: &Bson) -> bool` (equality by `compare::key::compare_bson`, since `Bson` is not `Hash`); `pub fn plan_batch(policy: ConflictPolicy, docs: Vec<Document>, existing: &ExistingIds) -> BatchPlan`; `pub struct BatchPlan { pub ops: Vec<WriteOp>, pub skipped: u64 }`.

- [ ] **Step 1: Write failing tests** in `policy_tests.rs`: `skip_inserts_only_new_documents`, `overwrite_replaces_existing_and_inserts_nothing`, `upsert_replaces_existing_and_inserts_new`, `existing_ids_match_across_numeric_types` (`Int32(1)` equals `Int64(1)` as `compare_bson` says), `skipped_counts_documents_without_an_op`. Use `mock_doc_with_id`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core copy::policy` and confirm it fails to compile or fails.
- [ ] **Step 3: Implement** the types above; one `WriteOp` per document, `Replace { id, doc }` carries the document's own `_id`.
- [ ] **Step 4: Run** the same command; expect all pass.
- [ ] **Step 5: Commit** `feat(copy): conflict policies and batch planning`.

### Task 2: Names, exclusions and name validation

**Files:**
- Create: `crates/core/src/copy/names.rs`, `crates/core/src/copy/names_tests.rs`

**Interfaces:**
- Produces: `pub fn is_excluded_database(name: &str) -> bool` (`admin`, `local`, `config`); `pub fn is_excluded_collection(name: &str) -> bool` (`system.` prefix); `pub fn unique_copy_name(wanted: &str, taken: &[String]) -> String` (`orders` -> `orders_copy`, then `orders_copy2`, `orders_copy3`...); `pub fn validate_collection_name(name: &str) -> Result<(), NameProblem>`; `pub fn validate_database_name(name: &str) -> Result<(), NameProblem>`; `pub enum NameProblem { Empty, TooLong, ForbiddenChar, SystemPrefix }`; `pub fn is_same_namespace(a: &Namespace, b: &Namespace) -> bool`.

- [ ] **Step 1: Write failing tests**: `unique_copy_name_adds_copy_suffix_then_counts`, `unique_copy_name_keeps_unicode_and_dots`, `database_name_rejects_slash_dot_space_dollar_and_null`, `collection_name_rejects_dollar_null_empty_and_system_prefix`, `collection_name_accepts_dots_spaces_unicode`, `database_name_limit_is_63_bytes_collection_namespace_255` (MongoDB's limits), `excluded_names_cover_admin_local_config_and_system_collections`, `same_namespace_is_exact_match`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core copy::names`; expect FAIL.
- [ ] **Step 3: Implement** as specified, no regex dependency needed.
- [ ] **Step 4: Run**; expect PASS.
- [ ] **Step 5: Commit** `feat(copy): name rules, exclusions and unique copy names`.

### Task 3: Store support for collection options (read-only guarded)

**Files:**
- Modify: `crates/core/src/store.rs` (trait + types), `crates/core/src/mongo_store.rs`, `crates/core/src/test_support/memory_store.rs`, `crates/core/src/read_only_store.rs`
- Create: `crates/core/src/read_only_store_copy_tests.rs` (or extend `read_only_store_tests.rs` if under the size limit), `crates/core/tests/docker_copy_options.rs`

**Interfaces:**
- Produces: `pub struct CollectionOptions { pub capped: Option<CappedSpec>, pub validator: Option<Document>, pub validation_level: Option<String>, pub validation_action: Option<String>, pub collation: Option<Document> }`, `pub struct CappedSpec { pub size: u64, pub max: Option<u64> }`; trait methods `async fn collection_options(&self, ns: &Namespace) -> Result<Option<CollectionOptions>>` (`None` when the collection does not exist) and `async fn create_collection(&self, ns: &Namespace, options: &CollectionOptions) -> Result<()>`, both with `Unsupported` defaults like the existing methods. Only `CollectionKind::Collection` is copied (per `list_collection_kinds`); views and time-series collections are skipped.

- [ ] **Step 1: Write failing tests**: in-memory round trip `create_collection_then_collection_options_returns_them`; `ReadOnlyStore` `create_collection_is_refused_when_locked` (`CoreError::ReadOnly`) and `collection_options_is_forwarded_when_locked`; the existing "every trait method is forwarded" guard test must include both methods. Docker test `docker_copy_options.rs`: create a capped + validator + collation collection on a real server, read it back equal.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core read_only_store` and `memory`; expect FAIL.
- [ ] **Step 3: Implement** the trait methods; the Mongo version reads `listCollections` options and issues `create`; `ReadOnlyStore` guards `create_collection` and forwards `collection_options` explicitly (see the header comment of `read_only_store.rs`).
- [ ] **Step 4: Run** unit tests; run `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test docker_copy_options` when docker is available.
- [ ] **Step 5: Commit** `feat(store): read and create collection options`.

### Task 4: Streaming copy of one collection

**Files:**
- Create: `crates/core/src/copy/engine.rs`, `crates/core/src/copy/engine_tests.rs`
- Modify: `crates/core/src/test_support/mock_objects.rs` (factories: `mock_docs_with_mixed_ids()`)

**Interfaces:**
- Consumes: `ConflictPolicy`, `plan_batch`, `ExistingIds` (Task 1); `DocumentStore::{scan_sorted, find, bulk_write}`; `compare::key::KeySpec` for `_id`; `compare::keyset::keyset_filter`; `compare::engine::CancelFlag`; `compare::apply::{APPLY_BATCH, MAX_REPORTED_FAILURES}`.
- Produces: `pub struct CopyJob { pub source: Arc<dyn DocumentStore>, pub source_ns: Namespace, pub filter: Document, pub target: Arc<dyn DocumentStore>, pub target_ns: Namespace, pub policy: ConflictPolicy, pub batch: usize }` (`batch` is `APPLY_BATCH` in production; tests lower it); `pub struct CopyProgress { pub scanned: u64, pub written: u64, pub skipped: u64, pub failed: u64 }`; `pub struct CopyReport { pub progress: CopyProgress, pub failures: Vec<CopyFailure>, pub outcome: CopyOutcome }`; `pub struct CopyFailure { pub key: Bson, pub reason: WriteFailure }` (UI-only, never logged); `pub enum CopyOutcome { Completed, Cancelled, Failed(CoreError) }`; `pub async fn copy_collection(job: CopyJob, cancel: &CancelFlag, on_progress: &mut (dyn FnMut(CopyProgress) + Send)) -> CopyReport`.

- [ ] **Step 1: Write failing tests** against two `InMemoryStore`s: `copies_all_documents_in_id_order_across_batches` (1,250 docs), `mixed_id_types_are_neither_skipped_nor_duplicated_at_batch_boundaries` (ObjectId, string, int, compound `_id`, with `CopyJob.batch` set to 3), `filter_limits_the_copied_set`, `skip_overwrite_upsert_end_states_match_policy` (assert target contents), `cancel_between_batches_reports_exact_written_count_equal_to_target_len`, `a_failing_batch_reports_written_so_far_and_stops` (store that fails `bulk_write` on call 2), `per_document_failures_are_bounded_and_counted`, `documents_with_dollar_and_dotted_field_names_are_copied_unchanged`, `same_namespace_in_same_store_is_refused` (returns `Failed(CoreError::...)` before reading).
- [ ] **Step 2: Run** `cargo test -p curlybrace-core copy::engine`; expect FAIL.
- [ ] **Step 3: Implement** `copy_collection`: loop `scan_sorted(source_ns, filter, _id key, from, batch)` (start key included per its contract: drop the duplicate first document when `from` is set); for each batch fetch existing target ids with `find` filter `{_id: {$in: ids}}` projection `{_id: 1}`, `plan_batch`, then `bulk_write`; map `WriteOutcome::Err` to `CopyFailure`; check `cancel` between batches and make store calls cancellable with `run_until_cancelled`. No values are logged.
- [ ] **Step 4: Run** the same command; expect PASS.
- [ ] **Step 5: Commit** `feat(copy): stream one collection under a conflict policy`.

### Task 5: Preview counts

**Files:**
- Create: `crates/core/src/copy/preview.rs`, `crates/core/src/copy/preview_tests.rs`

**Interfaces:**
- Consumes: `compare::engine::{run_pair, PairInput, Summary, Progress, EngineOutcome}`; `compare::rules::CompareRules`; the `_id` `KeySpec`.
- Produces: `pub struct PreviewCounts { pub new: u64, pub changed: u64, pub unchanged: u64 }`; `pub async fn preview_collection(job: &CopyJob, cancel: &CancelFlag, on_progress: &mut (dyn FnMut(u64) + Send)) -> Result<(EngineOutcome, PreviewCounts)>` mapping `Summary { only_in_source -> new, different -> changed, equal -> unchanged }` and ignoring `only_in_target`; a missing target collection counts everything as new.

- [ ] **Step 1: Write failing tests**: `counts_new_changed_unchanged_between_two_stores`, `target_only_documents_are_not_counted`, `missing_target_collection_counts_all_as_new`, `filter_applies_to_the_source_side_only`, `cancel_returns_cancelled_outcome`. (Read `compare/engine.rs` and an existing `engine` test for how to build `PairInput` and `Budget`.)
- [ ] **Step 2: Run** `cargo test -p curlybrace-core copy::preview`; expect FAIL.
- [ ] **Step 3: Implement** `preview_collection` over `run_pair`; the target side filter is empty.
- [ ] **Step 4: Run**; expect PASS.
- [ ] **Step 5: Commit** `feat(copy): preview counts from the compare engine`.

### Task 6: Runtime commands and events

**Files:**
- Create: `crates/core/src/copy/protocol.rs`, `crates/core/src/runtime/copy/mod.rs`, `crates/core/src/runtime/copy/preview.rs`, `crates/core/src/runtime/copy/run.rs`, `crates/core/src/runtime/copy_tests.rs` (and `copy_failure_tests.rs` if size requires)
- Modify: `crates/core/src/runtime/mod.rs` (`Command::Copy(CopyCommand)`, `Event::Copy(CopyEvent)`, dispatch next to `Command::Compare`, the `kind`/session-routing match arms at the lines that list `Self::Compare(_)`)

**Interfaces:**
- Consumes: `copy_collection`, `preview_collection` (Tasks 4, 5); `Session::current_store`, `Session::is_current`, `Session::emit_if_open`; `RunSlot` pattern from `runtime/compare/apply.rs`.
- Produces: `pub enum CopyScope { Collections(Vec<CopyItem>), Database { source_db: String, target_db: String } }`; `pub struct CopyItem { pub source: Namespace, pub target: Namespace, pub filter: Document }`; `pub struct CopyRequest { pub source: SessionId, pub target: SessionId, pub scope: CopyScope, pub policy: ConflictPolicy, pub with_indexes_and_options: bool }`; `pub enum CopyCommand { Preview { run: RunId, request: CopyRequest }, Start { run: RunId, request: CopyRequest }, Cancel { run: RunId } }`; `pub enum CopyEvent { Planned { run, items: Vec<CopyItem> }, PreviewProgress { run, index: usize, scanned: u64 }, PreviewItemDone { run, index: usize, counts: PreviewCounts }, PreviewDone { run }, Progress { run, index: usize, progress: CopyProgress, estimated_total: Option<u64> }, ItemDone { run, index: usize, report: CopyReport }, Done { run, cancelled: bool }, Failed { run, error: CoreError } }`. `Planned` is sent first for a `Database` scope after listing collections (excluded and views removed, via `list_collection_kinds`).

- [ ] **Step 1: Write failing tests** using `spawn_runtime_with_store`-style helpers as in `runtime/session_tests.rs` with two sessions: `preview_emits_planned_then_counts_then_done`, `start_copies_and_emits_item_done_then_done`, `database_scope_skips_system_collections_and_views`, `target_read_only_store_fails_before_any_write` (event `Failed { error: ReadOnly }`, target empty), `target_locked_mid_run_stops_with_exact_written_count`, `source_reconnect_mid_run_fails_with_written_count_and_no_further_writes` (bump the source epoch), `closed_session_emits_nothing_more`, `second_start_while_running_is_refused`, `cancel_emits_done_cancelled_with_exact_counts`, `no_event_or_log_line_contains_a_document_value` (use a recording logger or assert `Debug` of events for a sentinel string).
- [ ] **Step 2: Run** `cargo test -p curlybrace-core runtime::copy`; expect FAIL.
- [ ] **Step 3: Implement** the command handling: register the run synchronously (sessions open and connected, slot free) like `compare::apply::dispatch`; run preview/copy in a task over the two sessions' stores; check `Session::is_current(generation)` before each collection and between batches' reports.
- [ ] **Step 4: Run** `cargo test -p curlybrace-core`; expect PASS.
- [ ] **Step 5: Commit** `feat(runtime): copy commands and events over two open sessions`.

### Task 7: Integration tests on real servers

**Files:**
- Create: `crates/core/tests/local_copy_same_server.rs`, `crates/core/tests/docker_copy_between_servers.rs`

**Interfaces:**
- Consumes: the runtime from Task 6; testkit `local_enabled`, `docker_enabled`, container helpers in `crates/testkit/src/container.rs`.

- [ ] **Step 1: Write the tests** (gated): `copies_a_collection_to_another_database_on_one_server`, `copies_a_database_under_a_new_name_on_one_server`, `copies_between_two_containers_with_skip_overwrite_and_upsert`, `read_only_target_container_is_refused_and_untouched`, `cancel_mid_copy_leaves_target_with_exactly_the_reported_documents`, `system_databases_are_never_copied`.
- [ ] **Step 2: Run** `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test local_copy_same_server` and the docker one; expect FAIL before Task 6 is wired in a real store, then PASS.
- [ ] **Step 3:** fix any real-driver differences found (key order, `$in` limits) in Tasks 4 to 6 code.
- [ ] **Step 4: Run** both; expect PASS (or `SKIPPED` lines when ungated, which is acceptable in CI).
- [ ] **Step 5: Commit** `test(copy): integration tests on real servers`.

### Task 8: Clipboard and paste rules (UI view model)

**Files:**
- Create: `crates/ui/src/viewmodel/copy_clipboard.rs`, `crates/ui/src/viewmodel/copy_clipboard_tests.rs`
- Modify: `crates/ui/src/viewmodel/mod.rs`

**Interfaces:**
- Consumes: `curlybrace_core::copy::names::{is_excluded_database, is_same_namespace, unique_copy_name}`, `SessionId`.
- Produces: `pub enum CopyClipboard { Collections { session: SessionId, db: String, names: Vec<String> }, Database { session: SessionId, db: String }, Documents { session: SessionId, ns: Namespace, selection: DocSelection } }`; `pub enum DocSelection { Ids(Vec<Bson>), Query { filter: Document, sort: Option<Document> } }` (`Ids` limited to 10,000 by `DocSelection::ids(Vec<Bson>) -> Result<DocSelection, PasteBlock::TooManyDocuments>`); `pub enum PasteTarget { Connection { session: SessionId }, Database { session: SessionId, db: String }, Collection { session: SessionId, ns: Namespace } }`; `pub struct PasteContext<'a> { pub open: &'a dyn Fn(SessionId) -> bool, pub read_only: &'a dyn Fn(SessionId) -> bool, pub existing_names: &'a dyn Fn(SessionId, &str) -> Vec<String> }`; `pub enum PasteBlock { ReadOnlyTarget, SourceClosed, TargetClosed, DatabaseOntoDatabase, DocumentsOntoDatabase, CollectionsOntoCollection, SameNamespaceDocuments, ExcludedDatabase, TooManyDocuments }`; `impl CopyClipboard { pub fn paste_onto(&self, target: &PasteTarget, ctx: &PasteContext) -> Result<CopyRequest, PasteBlock>; pub fn menu_label_key(&self) -> (&'static str, u64) }`.

- [ ] **Step 1: Write failing tests**: one per `PasteBlock` reason, plus `collections_onto_database_builds_a_collections_request`, `database_onto_connection_builds_a_database_request`, `same_database_same_connection_suggests_unique_copy_names` (target names `orders_copy`), `documents_onto_collection_builds_an_ids_or_query_filter` (`{_id: {$in: [...]}}` or the query filter), `ids_selection_over_10000_is_refused`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui copy_clipboard`; expect FAIL.
- [ ] **Step 3: Implement** the pure rules; the clipboard holds references only (no document data except the hand-picked `_id`s).
- [ ] **Step 4: Run**; expect PASS.
- [ ] **Step 5: Commit** `feat(ui): copy clipboard and paste rules`.

### Task 9: Sidebar multi-select, Copy and Paste entries

**Files:**
- Create: `crates/ui/src/viewmodel/sidebar_selection.rs` (+ `_tests.rs`)
- Modify: `crates/ui/src/viewmodel/sidebar.rs`, `crates/ui/src/views/sidebar.rs`, `crates/ui/src/views/sidebar_roots.rs`, `crates/ui/src/views/context_menu.rs`, `crates/ui/src/app/actions.rs`, `crates/ui/src/app/state.rs`, `crates/ui/locales/en.json`
- Test: `crates/ui/src/app/copy_menu_tests.rs`

**Interfaces:**
- Produces: `pub struct SidebarSelection` with `toggled(self, node: NodeRef) -> Self` (cmd/ctrl-click), `extended_to(self, node: NodeRef, visible: &[NodeRef]) -> Self` (shift-click), `single(self, node) -> Self`, `nodes(&self) -> &[NodeRef]`, `pub enum NodeRef { Database { session: SessionId, db: String }, Collection { session: SessionId, ns: Namespace } }`; new `SidebarAction::{Copy, Paste(PasteTarget)}`; `AppState.clipboard: Option<CopyClipboard>`.

- [ ] **Step 1: Write failing tests**: selection (`click_selects_one`, `cmd_click_toggles`, `shift_click_selects_range_in_visible_order`, `mixed_databases_and_collections_copy_only_the_collections_of_one_database`), menu (`copy_entry_names_the_count`, `paste_entry_is_disabled_with_the_block_reason_key`, `paste_is_disabled_on_a_read_only_connection`), reducer (`copy_action_stores_a_reference_clipboard`, `paste_action_opens_the_paste_dialog_request`).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui sidebar_selection copy_menu`; expect FAIL.
- [ ] **Step 3: Implement**; keyboard shortcut Copy = cmd/ctrl+C and Paste = cmd/ctrl+V when the sidebar has focus; every label via `i18n` keys under `copy.*` in `en.json`; list the new keys in the PR.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui`; expect PASS (including `i18n_usage_tests`).
- [ ] **Step 5: Commit** `feat(ui): sidebar multi-select with copy and paste entries`.

### Task 10: Paste dialog, progress and report

**Files:**
- Create: `crates/ui/src/app/copy.rs` (reducer handlers), `crates/ui/src/app/copy_state.rs`, `crates/ui/src/views/copy_dialog.rs`, `crates/ui/src/views/copy_progress.rs`, `crates/ui/src/app/copy_tests.rs`, snapshot tests alongside the existing ones in `crates/ui/tests/`
- Modify: `crates/ui/src/app/reducer.rs` (`Input` variants, `apply_event` for `Event::Copy`), `crates/ui/src/app/state.rs`, `crates/ui/locales/en.json`

**Interfaces:**
- Consumes: `CopyRequest` (Task 6), `PreviewCounts`, the Compare typed-confirmation state (`crates/ui/src/app/compare_apply.rs`: `confirm_typed`, `confirmation_passes`), the connection manager's production marking (`crates/core/src/connection_color.rs`).
- Produces: `pub struct CopyDialog { request: CopyRequest, policy: ConflictPolicy, names: Vec<(String, String)>, with_indexes_and_options: bool, counts: Vec<Option<PreviewCounts>>, counting: bool, typed: String }`; `Input::Copy(CopyInput)` with `SetPolicy`, `SetName { index, name }`, `ToggleIndexesAndOptions`, `TypeConfirmation(String)`, `Confirm`, `Cancel`, `Close`; `CopyDialog::needs_typed_confirmation(&self, target_is_production: bool) -> bool` (true for `Overwrite` or a production target); `CopyDialog::name_problem(&self, index) -> Option<NameProblem>`.

- [ ] **Step 1: Write failing tests**: `opening_sends_preview_and_dialog_shows_counting`, `counts_arrive_per_collection_and_counting_ends_on_preview_done`, `confirm_is_enabled_while_counting_and_says_counts_are_incomplete`, `skip_and_upsert_need_no_typed_word_unless_target_is_production`, `overwrite_needs_the_typed_word`, `confirm_sends_start_only_when_the_dialog_allows`, `invalid_or_duplicate_target_names_block_confirm` (Task 2 validators), `default_policy_is_skip_and_options_off`, `progress_events_update_the_view`, `cancel_sends_cancel_and_shows_exact_written_counts`, `failure_report_shows_keys_only_and_is_bounded`, `read_only_error_shows_the_localized_message`, plus snapshot tests for the dialog and the progress view (follow `read_only_snapshot_tests.rs`).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui app::copy`; expect FAIL.
- [ ] **Step 3: Implement** the reducer handlers and the two views; every string through `i18n`; focus order and labels meet the accessibility tests already in the crate; a dialog is always shown for every paste (no silent path).
- [ ] **Step 4: Run** `cargo test --workspace --locked` and the snapshot command from `CONTRIBUTING.md` for the new snapshots; expect PASS.
- [ ] **Step 5: Commit** `feat(ui): paste dialog with counts, policy, progress and report`.

## Slice 2: documents

### Task 11: Copy and paste documents

**Files:**
- Modify: `crates/ui/src/viewmodel/table.rs`, `crates/ui/src/viewmodel/tree_state.rs`, `crates/ui/src/viewmodel/doc_actions.rs`, `crates/ui/src/views/documents.rs`, `crates/ui/src/app/copy.rs`, `crates/ui/locales/en.json`
- Test: `crates/ui/src/viewmodel/doc_copy_tests.rs`, `crates/ui/src/app/copy_documents_tests.rs`

**Interfaces:**
- Consumes: `CopyClipboard::Documents`, `DocSelection` (Task 8). First check whether the documents pane already supports multi-select (`grep -n -i select crates/ui/src/viewmodel/table.rs crates/ui/src/viewmodel/tree_state.rs`); add it if not.
- Produces: `DocAction::Copy`; "select all in result" producing `DocSelection::Query { filter, sort }`; paste onto a collection in the sidebar reuses Task 10's dialog with a `CopyItem` whose `filter` is the selection filter and whose target is the pasted-onto collection.

- [ ] **Step 1: Write failing tests**: `selecting_rows_builds_an_ids_selection`, `select_all_result_builds_a_query_selection_not_the_visible_page`, `copy_is_offered_with_a_projection_unlike_edit` (copy of a projected page copies full documents because the filter, not the page, is used), `over_10000_selected_rows_is_refused`, `paste_documents_onto_own_collection_is_disabled`, `documents_paste_uses_the_collection_dialog_and_filter`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui doc_copy copy_documents`; expect FAIL.
- [ ] **Step 3: Implement**; the engine already copies filtered sets, so no core change is needed.
- [ ] **Step 4: Run** `cargo test --workspace --locked`; expect PASS. Add one integration case to `local_copy_same_server.rs`: `copies_selected_documents_by_id_between_collections`.
- [ ] **Step 5: Commit** `feat(copy): copy and paste documents`.

## Slice 3: indexes and options

### Task 12: Collection options first, indexes last

**Files:**
- Modify: `crates/core/src/copy/engine.rs` (or create `crates/core/src/copy/structure.rs` + `structure_tests.rs`), `crates/core/src/runtime/copy/run.rs`

**Interfaces:**
- Consumes: `collection_options`, `create_collection` (Task 3); `list_index_specs`, `create_index`, `IndexSpec`.
- Produces: `pub async fn copy_structure_before(job: &CopyJob) -> Result<StructureReport>` (creates the target with options only when it does not exist; never touches an existing target's options) and `pub async fn copy_indexes_after(job: &CopyJob) -> Result<IndexReport>` (adds indexes the target lacks by name; the `_id` index is never copied; a failed build is recorded in `IndexReport`, the data stays). `CopyReport` gains `pub indexes: Option<IndexReport>`.

- [ ] **Step 1: Write failing tests**: `new_target_gets_options_before_the_data`, `existing_target_keeps_its_options`, `indexes_are_built_after_the_data`, `id_index_is_not_copied`, `existing_index_with_the_same_name_is_left_alone`, `failed_unique_index_is_reported_and_data_stays`, `unchecked_option_copies_data_only`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core copy::structure`; expect FAIL.
- [ ] **Step 3: Implement** and call these from the runtime when `with_indexes_and_options` is set; show the index outcome in the report view (new i18n keys).
- [ ] **Step 4: Run** `cargo test --workspace --locked`; add a docker case `copies_options_and_indexes_between_two_containers` to `docker_copy_between_servers.rs`; expect PASS.
- [ ] **Step 5: Commit** `feat(copy): copy indexes and collection options`.

## Slice 4: drag and drop

### Task 13: Drag selections onto a target

**Files:**
- Modify: `crates/ui/src/views/sidebar.rs`, `crates/ui/src/views/sidebar_roots.rs`, `crates/ui/src/viewmodel/copy_clipboard.rs`
- Test: `crates/ui/src/app/copy_drag_tests.rs`

**Interfaces:**
- Consumes: egui drag and drop (`ui.dnd_drag_source`, `ui.dnd_drop_zone` or `Response::dnd_hover_payload`), `CopyClipboard::paste_onto`.
- Produces: a payload `CopyClipboard` set on drag start from the selection (or the dragged row when it is not selected); dropping runs the same `SidebarAction::Paste(target)` path. A drop that `paste_onto` blocks shows the block reason as the drop tooltip and does nothing.

- [ ] **Step 1: Write failing tests** (pure, on the action/viewmodel layer; egui drag is not unit-testable): `drag_of_unselected_row_drags_only_that_row`, `drag_of_selected_row_drags_the_whole_selection`, `drop_runs_the_same_paste_path_as_the_menu`, `blocked_drop_reports_the_reason_and_sends_nothing`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui copy_drag`; expect FAIL.
- [ ] **Step 3: Implement** the egui wiring; the menu and keyboard path stays the primary way and must keep working with no pointer.
- [ ] **Step 4: Run** `cargo test --workspace --locked`; try it by hand with two connections and say so in the PR description.
- [ ] **Step 5: Commit** `feat(ui): drag and drop selections onto a target`.

### Task 14: Documentation and roadmap

**Files:**
- Create: `docs/copy.md`
- Modify: `README.md` (move #48 from **Next** to **Built**, mention "In progress" is empty again), `crates/ui/locales/en.json` (final key audit)

- [ ] **Step 1:** write `docs/copy.md` (what can be copied, the dialog, the three policies with the exact semantics from Global Constraints, exclusions, cancel and report behaviour, limits such as 10,000 hand-picked documents and no snapshot consistency).
- [ ] **Step 2:** update the README roadmap and link the guide, in the style of the Compare guide link.
- [ ] **Step 3:** run `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo test --workspace --locked`; expect clean.
- [ ] **Step 4: Commit** `docs(copy): guide and roadmap`.
