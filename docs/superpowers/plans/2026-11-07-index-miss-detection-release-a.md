# Index-miss detection, release A (query-bar verdict) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After every query run in the query bar, show an automatic, non-blocking verdict (index used, full collection scan, sort not covered, inefficient index use, or unavailable) with a reason and a suggested index.

**Architecture:** A pure `classify` function in `curlybrace-core` turns the server's explain output into a typed `Verdict`. A new read-only `DocumentStore::explain_find` fetches the explain; a `Command::ExplainFind` / `Event::Explain` pair carries it through the runtime independently of `Command::Find`. The UI keeps the verdict in `AppState` keyed by the run's query, drops stale ones, and renders a badge in the results area.

**Tech Stack:** Rust workspace (`curlybrace-core`, `curlybrace-ui` egui, `curlybrace-testkit`), `mongodb` driver, bson documents, `en.json` i18n.

**Spec:** `docs/superpowers/specs/2026-11-07-index-miss-detection-design.md` (release A only; release B is a separate plan).

## Global Constraints

- No hardcoded user-facing strings: every label, reason and tooltip is a key in `crates/ui/locales/en.json` (nested, alphabetical); report the new keys at the end. Core returns i18n key + params, never English.
- Immutable data patterns; files under about 800 lines, functions under about 50; one `*_tests.rs` sibling per module as in `crates/core/src/history`.
- TDD; at least 80% coverage on the new core code; real BSON documents in tests, not mocks.
- Explain is a read: `ReadOnlyStore` must forward `explain_find` explicitly (a missing entry silently uses the trait default). Test it.
- Explain runs at `executionStats` with `maxTimeMS` 2000; on timeout fall back to `queryPlanner` and mark the verdict plan-only.
- Release A touches no profiler, no `history.sqlite`, no shape normalisation.
- Real-MongoDB tests are opt-in (`CURLYBRACE_IT=1`) and new files are added by hand to `scripts/integration-tests.sh`.
- Verify with `cargo test --workspace` (offline) and `cargo clippy --workspace --all-targets`.
- Commits use conventional format with the `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` trailer.

## Review Focus

1. **Nested plans.** `$or`, `$in` and sort-merge plans put several `IXSCAN` stages under `OR`/`SORT_MERGE`, wrapped in `FETCH`/`PROJECTION_*`/`LIMIT`/`SKIP`; classification must walk the whole tree, not look at the root. (Task 1)
2. **Slot-based plans.** Servers using SBE put the stage tree under `winningPlan.queryPlan`; classification must read both shapes. (Task 1)
3. **Missing or empty collection.** Explain on an empty or non-existent collection yields an `EOF` stage: show the neutral note, never "full collection scan". (Task 1)
4. **Views and failures.** Explain on a view, a permission error or a timeout shows "verdict unavailable" with a reason and never a red error banner or a blocked results list. (Tasks 3, 4)
5. **Rapid reruns.** A slow explain for run 1 arriving after run 2 started must not appear against run 2's results; paging keeps the verdict of its run. (Task 4)

---

### Task 1: `classify` and the `Verdict` types

**Files:**
- Create: `crates/core/src/explain/mod.rs`, `crates/core/src/explain/verdict.rs`, `crates/core/src/explain/classify.rs`, `crates/core/src/explain/suggest.rs`
- Test: `crates/core/src/explain/classify_tests.rs`, `crates/core/src/explain/suggest_tests.rs`, fixtures in `crates/core/src/explain/fixtures/*.json` (extended JSON explain documents captured from a real server: collscan, ixscan, sort, inefficient, covered, or_multi_ixscan, sbe, eof, plan_only)
- Modify: `crates/core/src/lib.rs` (add `pub mod explain;`)

**Interfaces:**
- Produces:
  - `pub enum VerdictKind { IndexUsed, CoveredByIndex, CollectionScan, SortNotCovered, InefficientIndex, NoFilter, EmptyCollection }`
  - `pub struct Reason { pub key: &'static str, pub params: Vec<(&'static str, String)> }`
  - `pub struct SuggestedIndex { pub keys: Vec<(String, i32)> }` with `pub fn command(&self, collection: &str) -> String` (a `db.<collection>.createIndex({...})` string)
  - `pub struct Verdict { pub kind: VerdictKind, pub index_name: Option<String>, pub keys_examined: u64, pub docs_examined: u64, pub returned: u64, pub plan_only: bool, pub reasons: Vec<Reason>, pub suggestion: Option<SuggestedIndex> }`
  - `pub fn classify(explain: &Document, query: &Query) -> Result<Verdict, CoreError>` (error: `CoreError::Database` when the document has no recognisable plan)
  - `pub fn suggest(query: &Query) -> Option<SuggestedIndex>`

- [ ] **Step 1: Write failing tests** in `classify_tests.rs`, one per fixture, asserting `kind` (and `index_name`, `plan_only`): `classify_collscan_is_collection_scan`, `classify_ixscan_close_to_returned_is_index_used`, `classify_sort_stage_is_sort_not_covered`, `classify_many_keys_few_returned_is_inefficient`, `classify_projection_covered_is_covered`, `classify_or_with_two_ixscans_is_index_used`, `classify_sbe_query_plan_wrapper_is_read`, `classify_eof_is_empty_collection`, `classify_empty_filter_without_sort_is_no_filter`, `classify_empty_filter_with_uncovered_sort_is_sort_not_covered`, `classify_id_only_filter_is_index_used`, `classify_query_planner_only_sets_plan_only`, `classify_garbage_document_errors`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core explain` and confirm FAIL (module missing).
- [ ] **Step 3: Implement `classify`** in `classify.rs`: collect all stage names by walking `inputStage`/`inputStages`/`queryPlan`; precedence `NoFilter` (empty filter, no sort, `COLLSCAN`) → `EmptyCollection` (`EOF`) → `SortNotCovered` (`SORT` present) → `CollectionScan` → `InefficientIndex` → `CoveredByIndex` → `IndexUsed`. Inefficient means `max(keys, docs examined) >= 100` and `>= 10 x max(returned, 1)` (named constants). `plan_only` is true when `executionStats` is absent, and then counts are 0 and `InefficientIndex` is never produced.
- [ ] **Step 4: Implement `suggest`** in `suggest.rs` with the equality, sort, range rule: top-level filter fields compared by plain value or `$eq`/`$in` come first (in filter order), then sort fields (direction kept), then fields with `$gt/$gte/$lt/$lte/$ne/$regex` comparisons; duplicates dropped; `None` for an empty filter and no sort. Tests: `suggests_equality_then_sort_then_range`, `deduplicates_fields`, `none_for_empty_query`, `command_renders_create_index`.
- [ ] **Step 5: Run** `cargo test -p curlybrace-core explain` → PASS. `cargo clippy -p curlybrace-core --all-targets` clean.
- [ ] **Step 6: Commit** `feat(explain): classify explain output into a verdict with a suggested index`

### Task 2: `DocumentStore::explain_find`

**Files:**
- Modify: `crates/core/src/store.rs` (trait + `Explain` type), `crates/core/src/mongo_store.rs`, `crates/core/src/read_only_store.rs`, `crates/core/src/test_support/memory_store.rs`
- Test: `crates/core/src/read_only_store_tests.rs`, `crates/core/src/test_support/memory_store.rs` (inline tests)

**Interfaces:**
- Consumes: `Query`, `Namespace` (`store.rs`).
- Produces:
  - `pub struct Explain { pub document: Document, pub plan_only: bool }`
  - `async fn explain_find(&self, ns: &Namespace, q: &Query) -> Result<Explain>` with a default body returning `CoreError::Unsupported("this store cannot explain queries".into())`
  - `MemoryStore::with_explain(self, ns: &Namespace, document: Document) -> Self` to script the answer.

- [ ] **Step 1: Write failing tests:** `read_only_store_forwards_explain_find` (a read-only wrapper over a scripted memory store returns the scripted document), `memory_store_unscripted_explain_is_unsupported`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core explain_find` → FAIL.
- [ ] **Step 3: Implement** the trait method, the memory-store scripting, and the explicit forward in `ReadOnlyStore` (next to `index_stats`).
- [ ] **Step 4: Implement `MongoStore::explain_find`:** build the find command the same way `find` does (filter, sort, projection, `skip`, `limit` via `fetch_limit`), wrap as `{explain: <find>, verbosity: "executionStats", maxTimeMS: 2000}` and `run_command`; on a server time-limit error (code 50, `MaxTimeMSExpired`) re-run with `verbosity: "queryPlanner"` and `plan_only: true`. Extract command building into a small function shared with `find` where it avoids duplication, in a new file `crates/core/src/mongo_explain.rs` to keep `mongo_store.rs` small.
- [ ] **Step 5: Run** `cargo test -p curlybrace-core` → PASS.
- [ ] **Step 6: Commit** `feat(explain): add a read-only explain_find to the document store`

### Task 3: Runtime command and event

**Files:**
- Modify: `crates/core/src/runtime/mod.rs` (`Command::ExplainFind`, `Event::Explain`, `Event::session`), `crates/core/src/runtime/handlers.rs`
- Test: `crates/core/src/runtime/explain_tests.rs`

**Interfaces:**
- Consumes: `explain_find` (Task 2), `classify` (Task 1).
- Produces:
  - `Command::ExplainFind { session: SessionId, ns: Namespace, query: Query }`
  - `Event::Explain { session: SessionId, ns: Namespace, query: Query, outcome: Result<Verdict, CoreError> }`

- [ ] **Step 1: Write failing tests:** `explain_find_answers_with_a_verdict` (memory store scripted with the collscan fixture → `Event::Explain` with `CollectionScan`), `explain_failure_is_an_outcome_not_a_failed_event` (unscripted store → `outcome == Err(CoreError::Unsupported(..))` and no `Event::Failed`), `explain_find_is_allowed_on_read_only_session`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core runtime::explain` → FAIL.
- [ ] **Step 3: Implement** the handler: call `explain_find`, then `classify(&explain.document, &query)` and set `plan_only` from `Explain`; any error becomes `outcome: Err(..)` (it must never become `Event::Failed`, which raises the banner). Views: if explain returns the server error for views, the same `Err` path applies.
- [ ] **Step 4: Run** `cargo test -p curlybrace-core` → PASS.
- [ ] **Step 5: Commit** `feat(explain): explain command and event in the runtime`

### Task 4: Reducer state and wiring

**Files:**
- Create: `crates/ui/src/app/explain_actions.rs`
- Modify: `crates/ui/src/app/state.rs` (field `verdict: VerdictState`), `crates/ui/src/app/actions.rs` (`find`, `open`), `crates/ui/src/app/reducer.rs` (`Event::Explain`)
- Test: `crates/ui/src/app/explain_tests.rs`

**Interfaces:**
- Consumes: `Command::ExplainFind`, `Event::Explain` (Task 3); `explain_enabled: bool` from Task 6 (until Task 6 lands, a constant `true`).
- Produces:
  - `pub enum VerdictState { None, Pending { query: Query }, Ready { query: Query, verdict: Verdict }, Unavailable { query: Query, error: CoreError } }` (default `None`)
  - `AppState::verdict: VerdictState`

- [ ] **Step 1: Write failing tests:** `run_issues_find_and_explain_and_sets_pending`; `paging_keeps_the_verdict_and_issues_no_explain`; `clear_and_open_reset_the_verdict_without_explaining`; `explain_for_the_current_run_is_stored`; `explain_for_an_older_run_is_dropped` (run A, run B, then A's event → state still Pending for B); `explain_for_another_collection_is_dropped`; `explain_error_becomes_unavailable_without_a_banner`; `explain_disabled_issues_only_find` (uses Task 6's flag; add in Task 6).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui explain` → FAIL.
- [ ] **Step 3: Implement.** `FindAction::Run(query)` emits `[Command::Find, Command::ExplainFind]` for the same `ns` and `query` and sets `Pending { query }`. A verdict belongs to a run by its query with `skip` ignored: store the run's `Query` and compare filter, sort, projection and limit when an event arrives. `Prev`/`Next` do not touch `verdict`; `Clear` and `open` set `VerdictState::None`. `Event::Explain` is accepted only when `state.selected_ns() == Some(&ns)` and the query matches the pending one.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui` → PASS.
- [ ] **Step 5: Commit** `feat(explain): keep the verdict of the current run in app state`

### Task 5: Verdict badge, strings and accessibility

**Files:**
- Create: `crates/ui/src/viewmodel/verdict.rs`, `crates/ui/src/views/verdict_badge.rs`
- Modify: `crates/ui/locales/en.json`, `crates/ui/src/i18n_usage_tests.rs` (add `"query."` to `PREFIXES`), `crates/ui/src/views/mod.rs`, the results area that renders `show_documents_keyed` (call the badge above the documents)
- Test: `crates/ui/src/viewmodel/verdict_tests.rs`, `crates/ui/tests/accessibility_verdict.rs`, a snapshot case beside the existing ones

**Interfaces:**
- Consumes: `VerdictState` (Task 4), `Verdict`/`Reason`/`SuggestedIndex` (Task 1), `I18n::t` / `t_with`.
- Produces:
  - `pub struct BadgeModel { pub tone: Tone, pub icon: &'static str, pub title_key: &'static str, pub reasons: Vec<Reason>, pub suggestion: Option<String>, pub plan_only: bool }`, `pub enum Tone { Good, Warn, Neutral, Unavailable, Checking }`
  - `pub fn badge(state: &VerdictState, collection: &str) -> Option<BadgeModel>` (`None` renders nothing)
  - `pub fn show_verdict_badge(ui: &mut egui::Ui, model: &BadgeModel, i18n: &I18n) -> Option<VerdictAction>` where `VerdictAction::CopySuggestion(String)`

- [ ] **Step 1: Write failing tests:** `badge_for_each_kind_has_distinct_title_and_tone`, `pending_is_checking`, `unavailable_shows_reason_key_for_unsupported_and_database_errors`, `suggestion_command_uses_collection_name`, `plan_only_adds_the_estimate_note`; accessibility test: the badge exposes its title and reasons as text with an icon, not colour alone, and the copy button has an accessible label.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui verdict` → FAIL.
- [ ] **Step 3: Implement** the viewmodel and view. Keys under `query.verdict.*` (titles per kind, `checking`, `unavailable`, `plan_only`, reasons such as `collscan`, `sort_blocking`, `inefficient` with `{examined}` and `{returned}` params, `suggestion`, `copy`) added alphabetically to `en.json`. The badge renders in the results header region without moving the documents list; copy uses the existing clipboard mechanism.
- [ ] **Step 4: Run** `cargo test --workspace` (includes `i18n_usage_tests`) → PASS; list the new keys.
- [ ] **Step 5: Commit** `feat(explain): show the verdict badge with reasons and a suggested index`

### Task 6: Global opt-out setting

**Files:**
- Create: `crates/ui/src/app/explain_settings.rs`
- Modify: `crates/ui/src/app/state.rs`, `crates/ui/src/app/mod.rs` (load at startup like `sampling_settings`), `crates/ui/src/app/explain_actions.rs`, the find bar menu in `crates/ui/src/views/findbar.rs`, `crates/ui/locales/en.json`
- Test: `crates/ui/src/app/explain_settings_tests.rs`

**Interfaces:**
- Consumes: `settings_file::load`, `atomic_write`, `PersistGuard` (follow `sampling_settings.rs`).
- Produces:
  - `pub struct ExplainSettings { pub automatic: bool }` (default `true`), `ExplainSettings::load(path, backup) -> Startup<ExplainSettings>`, `save`, file `explain.json` beside `sampling.json`
  - `AppState::explain_enabled() -> bool`

- [ ] **Step 1: Write failing tests:** `defaults_to_automatic`, `missing_file_is_default`, `corrupt_file_is_backed_up_and_default`, `round_trips_false`, `disabled_run_issues_only_find` (reducer).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui explain_settings` → FAIL.
- [ ] **Step 3: Implement** the settings file by the `sampling_settings.rs` pattern and a checkbox "Check index use automatically" (key `query.verdict.automatic`) in the find bar; replace Task 4's constant with `explain_enabled()`.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(explain): setting to turn the automatic verdict off`

### Task 7: Real-server tests

**Files:**
- Create: `crates/core/tests/local_explain.rs`
- Modify: `scripts/integration-tests.sh` (add the new file to the target list)

**Interfaces:**
- Consumes: `MongoStore::explain_find`, `classify`, the `crates/testkit` local harness (`gate`, throwaway `cbit_<id>` databases) as used by `local_compare_store.rs`.

- [ ] **Step 1: Write the tests** (all `#[ignore]` and gated by `CURLYBRACE_IT=1`): `real_collscan_is_collection_scan`, `real_indexed_equality_is_index_used`, `real_sort_on_unindexed_field_is_sort_not_covered`, `real_compound_index_wrong_order_is_inefficient` (index `{b:1,a:1}`, query on `a` equality with a range on `b`, enough documents to exceed the thresholds), `real_covered_projection_is_covered`, `real_view_is_an_error_outcome`, `real_empty_collection_is_empty`, and one that records the explain document of each into `crates/core/src/explain/fixtures` when `CURLYBRACE_CAPTURE=1` (so Task 1 fixtures can be refreshed).
- [ ] **Step 2: Run** `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test local_explain -- --ignored` against the local mongod → PASS. Fix thresholds or the classifier if a real document disagrees with a fixture, and update the fixture.
- [ ] **Step 3: Add** the file to `scripts/integration-tests.sh`; run the script once.
- [ ] **Step 4: Commit** `test(explain): verdicts against a real server`

---

## Self-review

- **Spec coverage:** verdict kinds and reasons (Tasks 1, 5); suggestion with copy button (1, 5); explain at `executionStats` with timeout fallback and plan-only note (2, 5); read-only forwarding (2, 3); non-blocking, stale-safe, once per Run, paging keeps verdict (3, 4); unavailable states without a banner (3, 4, 5); opt-out global setting (6); neutral empty-filter note and `_id` edge cases (1); i18n and accessibility (5); real-server tests (7). Deferred by the spec: hint/collation (#75), prefilled builder, everything in release B.
- **Type consistency:** `Verdict`, `Reason`, `SuggestedIndex`, `Explain`, `VerdictState`, `BadgeModel` and `ExplainSettings` are each defined once and referenced by the same names. `Event::Explain.outcome` is `Result<Verdict, CoreError>` in Tasks 3 and 4.
- **Proportion:** seven tasks, no function bodies except where the spec fixes an algorithm (precedence order, thresholds, suggestion rule).
