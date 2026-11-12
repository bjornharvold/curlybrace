# Index-miss guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Before the query bar or the Shell tab sends a query that the server would answer with a collection scan or an uncovered sort, warn and let the author run it anyway.

**Architecture:** One core module (`guard`) explains a batch of queries plan-only, one at a time under an overall time budget, and returns the ones that miss an index. One runtime command/event pair carries it. The query bar and the Shell tab each park their run in a "checking" state, then either continue or show a notice with Run anyway / Cancel. The Shell tab gets a pure text reader that pulls literal collection calls out of a script.

**Tech Stack:** Rust 1.87.0, egui, the `mongodb`/`bson` crates, tokio, `egui_kittest` for UI tests.

**Spec:** `docs/superpowers/specs/2026-11-12-index-miss-guard-design.md`

## Global Constraints

- CI pins Rust 1.87.0: verify with `cargo +1.87.0` (`fmt --all --check`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`). No let-chains (`if a && let Some(x) = y`).
- No new dependency. Nothing added to `history.sqlite`. Only persisted change: a `guard` field in `explain.json`.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested); report new keys.
- Files stay under 400 lines (800 max); immutable updates; errors handled explicitly; error text and logs never include query values (log `error_variant` only).
- Tests use real objects, a scripted fake store, and `egui_kittest`. Never connect to a MongoDB that is already on the machine; real-server checks use a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped and deleted afterwards. Never print passwords.
- Do not edit `CHANGELOG.md` (release-please) or the spec.
- Constants (core, `crates/core/src/guard.rs`): `GUARD_MIN_DOCS = 1000`, `GUARD_MAX_TARGETS = 20`, `GUARD_BUDGET_MS = 10_000`.
- Conventional commits, lowercase imperative subject; trailers `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`.

## Review Focus

1. A guard check that fails, times out or never answers must not strand the Run: the find is sent (query bar) or the script runs (Shell). Test: `GuardChecked` with `Err(..)` continues; a lost connection while checking drops the state.
2. A stale answer (another Run, another collection, reconnect, Clear in between) is ignored and never sends a find; **Run anyway** sends exactly one find. Test with a superseded `request`.
3. Edge counts: a collection of exactly 1,000 documents warns, 999 does not (non-production); production warns at any size; a failing `estimated_count` warns.
4. Query shapes explain cannot plan (`$text` without a text index, `$where`, invalid operator) count as "not checked", never as a miss and never as a failure.
5. Shell reader: calls inside comments, strings, template literals and regex literals are ignored; a filter with a variable is "not checked"; chained `.limit(..)`, `.sort({..})`, `.toArray()` after `find` are accepted; more than 20 calls leave the rest "not checked"; a selection run reads the selection only.
6. The notice text never shows more of the query than the filter the author wrote; no log line or error message carries filter values.

---

### Task 1: Core `guard` module and plan-only explain

**Files:**
- Create: `crates/core/src/guard.rs`, `crates/core/src/guard_tests.rs`
- Modify: `crates/core/src/lib.rs` (export `guard`), `crates/core/src/store.rs` (trait method), `crates/core/src/mongo_store.rs`, `crates/core/src/mongo_explain.rs`, the in-memory/fake store(s) in `crates/core/src/test_support` that implement `DocumentStore`
- Test: `guard_tests.rs`, `mongo_explain_tests.rs`

**Interfaces:**
- Consumes: `explain::{classify, Verdict, VerdictKind, Reason, SuggestedIndex}`, `store::{DocumentStore, Namespace, Query}`, `DocumentStore::estimated_count(&Namespace) -> Result<u64>`.
- Produces:
  - `DocumentStore::explain_plan(&self, ns: &Namespace, q: &Query) -> Result<Explain>`: default returns `CoreError::Unsupported`; the Mongo store runs the existing `explain_command(ns, q, Verbosity::QueryPlanner)` and returns `Explain { document, plan_only: true }`. It never executes the query.
  - `pub struct GuardTarget { pub ns: Namespace, pub query: Query }` (Clone, Debug, PartialEq)
  - `pub struct GuardFinding { pub ns: Namespace, pub query: Query, pub kind: VerdictKind, pub reasons: Vec<Reason>, pub suggestion: Option<SuggestedIndex> }` (Clone, Debug)
  - `pub struct GuardReport { pub findings: Vec<GuardFinding>, pub unchecked: usize }` (Clone, Debug, Default)
  - `pub fn warns(kind: VerdictKind) -> bool` (true only for `CollectionScan` and `SortNotCovered`)
  - `pub async fn check(store: &dyn DocumentStore, targets: Vec<GuardTarget>, min_docs: Option<u64>, budget: std::time::Duration) -> GuardReport`

- [ ] **Step 1: Write failing tests in `guard_tests.rs`** against a scripted fake store (explain document per namespace, count per namespace, optional per-call error or delay):
  - `warns_only_for_scan_and_uncovered_sort` over every `VerdictKind::ALL`.
  - `a_scan_on_a_collection_of_exactly_min_docs_is_reported` and `..._one_below_is_not` (`min_docs = Some(1000)`, counts 1000 and 999).
  - `min_docs_none_reports_a_scan_at_any_size` (count 3).
  - `a_failing_count_still_reports_the_scan`.
  - `an_explain_error_counts_as_unchecked_and_is_not_a_finding`.
  - `targets_beyond_max_targets_count_as_unchecked` (25 targets, 20 explained, `unchecked == 5`).
  - `targets_not_reached_in_the_budget_count_as_unchecked` (fake store sleeps; budget 50 ms; tokio `time::pause`).
  - `a_target_without_filter_and_sort_is_skipped_without_an_explain` (not counted as unchecked).
- [ ] **Step 2: Run** `cargo +1.87.0 test -p curlybrace-core guard` and confirm FAIL (module missing).
- [ ] **Step 3: Implement** `guard.rs` as specified. Explain one target at a time; check the elapsed budget before each; classify with `explain::classify`; call `estimated_count` only for a scan/sort verdict and only when `min_docs` is `Some`. Add `explain_plan` to the trait, the Mongo store and the fakes. Add `mongo_explain::explain_plan(db, ns, q)`.
- [ ] **Step 4: Add a test in `mongo_explain_tests.rs`** that `explain_command(.., QueryPlanner)` carries `"verbosity": "queryPlanner"` and the `maxTimeMS` bound (reuse the file's existing assertions' style).
- [ ] **Step 5: Run** `cargo +1.87.0 test -p curlybrace-core` and `clippy -D warnings`; expect PASS.
- [ ] **Step 6: Commit** `feat(guard): plan-only explain and a batch check for index misses`.

### Task 2: Runtime command and event

**Files:**
- Modify: `crates/core/src/runtime/mod.rs`, `command.rs`, `handlers.rs`
- Test: the runtime's existing handler/command test files (follow how `ExplainFind` is tested)

**Interfaces:**
- Consumes: Task 1's `guard::{check, GuardTarget, GuardReport, GUARD_BUDGET_MS}`.
- Produces:
  - `Command::GuardCheck { session: SessionId, request: u64, targets: Vec<GuardTarget>, min_docs: Option<u64> }`: a read, allowed on read-only connections, session-scoped like `ExplainFind`.
  - `Event::GuardChecked { session: SessionId, request: u64, outcome: Result<GuardReport, CoreError> }`. A failure is an `Err` outcome, never `Event::Failed`, never a banner, no ping settle (same rule as `Explain`).

- [ ] **Step 1: Write failing tests**: the command is allowed on a read-only session; its `session()` and name (`"guard_check"`) are right; with a scripted store the handler returns `GuardChecked` with the report; a store whose explain errors gives a report with `unchecked == targets.len()`; a handler-level timeout gives `Err` in the outcome and no `Failed`.
- [ ] **Step 2: Run** the runtime tests, confirm FAIL.
- [ ] **Step 3: Implement** the variants and the handler arm calling `guard::check(store, targets, min_docs, Duration::from_millis(GUARD_BUDGET_MS))`. Extend the timed-out special case in `handlers.rs` (the `explain_target` branch) so a timed-out `GuardCheck` yields `GuardChecked { outcome: Err(Database(COMMAND_TIMEOUT_MESSAGE)) }`.
- [ ] **Step 4: Run** all core tests and clippy; expect PASS. **Step 5: Commit** `feat(runtime): guard check command and event`.

### Task 3: The guard setting

**Files:**
- Modify: `crates/ui/src/app/explain_settings.rs`, `crates/ui/src/app/actions.rs` (`FindAction::SetGuard(bool)`), the find bar view where the "Check index use automatically" checkbox lives, `crates/ui/locales/en.json`
- Test: `explain_settings_tests.rs`, the find-bar accessibility test

**Interfaces:**
- Consumes: existing `ExplainSettings { automatic }`, `set_automatic`, `settings_file` rules.
- Produces: `ExplainSettings { automatic: bool, guard: bool }` (guard default `true`); `explain.json` `{"automatic": .., "guard": ..}` with a missing `guard` meaning `true` and a non-bool meaning corrupt; `AppState::guard_enabled() -> bool`; `FindAction::SetGuard(bool)` handled like `SetAutomaticVerdict` (global, needs no connection, saved atomically 0600, same backup/refuse-to-save recovery).

- [ ] **Step 1: Write failing tests**: parse of `{}` gives `guard == true`; `{"guard": false}` round-trips; `{"guard": "no"}` is corrupt and backed up; `SetGuard(false)` saves and `guard_enabled()` is false with no connection.
- [ ] **Step 2: Run, confirm FAIL. Step 3: Implement.** Add the checkbox next to the existing one, key `query.guard.setting` ("Warn before queries that would scan a collection") with a tooltip key `query.guard.setting_hint`.
- [ ] **Step 4: Run** `cargo +1.87.0 test -p curlybrace-ui` (settings, accessibility, i18n usage tests). **Step 5: Commit** `feat(guard): setting to warn before queries that scan a collection`.

### Task 4: Query-bar guard flow

**Files:**
- Create: `crates/ui/src/app/guard_actions.rs`, `crates/ui/src/app/guard_tests.rs`
- Modify: `crates/ui/src/app/state.rs` (field), `actions.rs` (`FindAction::Run`, new `FindAction::GuardRunAnyway`, `FindAction::GuardCancel`), `reducer.rs` (`Event::GuardChecked`), the places that reset find state on collection change, reconnect and `FindAction::Clear`
- Test: `guard_tests.rs`

**Interfaces:**
- Consumes: Task 2's command/event; `explain_actions::run(state, query, explain_enabled)`; `AppState::session_production(id)`; `guard::{warns, GUARD_MIN_DOCS, GuardReport, GuardTarget}`.
- Produces:
  - `pub enum GuardState { None, Checking { request: u64, query: Query }, Warn { query: Query, report: GuardReport } }` (Default `None`), field `AppState.guard: GuardState` and a `guard_request: u64` counter.
  - `FindAction::Run(query)`: when `guard_enabled()`, a collection is selected, and the query has a non-empty filter or a sort, set `Checking` and send `Command::GuardCheck { targets: vec![GuardTarget{..}], min_docs: if production { None } else { Some(GUARD_MIN_DOCS) } }` instead of the find; otherwise behave as today.
  - `Event::GuardChecked`: ignored unless `request` matches the current `Checking`; `Err(_)` or an empty `findings` continues with `explain_actions::run` (find plus badge explain); otherwise `Warn`.
  - `GuardRunAnyway` sends the find once through `explain_actions::run`, state `None`, never re-guarding. `GuardCancel` sets `None`.
  - `guard_actions::dropped(state) -> AppState` clears the state; call it on collection change, reconnect, session close and `Clear`.

- [ ] **Step 1: Write failing tests** (reducer-level, like `explain_tests.rs`): guard off sends the find as today; guard on with a filter sends only `GuardCheck` (no `Find`) and sets `Checking`; no filter and no sort sends the find; production uses `min_docs None`, other uses `Some(1000)`; an `Err` outcome sends `Find` + `ExplainFind`; empty findings the same; findings give `Warn` and send nothing; `GuardRunAnyway` sends exactly one `Find` and a second `GuardRunAnyway` sends none; a `GuardChecked` with an old `request` after a second Run changes nothing; collection change, reconnect and `Clear` drop `Checking` and `Warn`; read-only connection still checks.
- [ ] **Step 2: Run, confirm FAIL. Step 3: Implement. Step 4: Run** `cargo +1.87.0 test -p curlybrace-ui app::`; clippy. **Step 5: Commit** `feat(guard): check the query bar's query before it runs`.

### Task 5: Query-bar notice

**Files:**
- Create: `crates/ui/src/views/guard_notice.rs`
- Modify: the view that draws the find bar results area, `crates/ui/locales/en.json`, `crates/ui/tests/support/` snapshot scenes and `crates/ui/tests/snapshots.rs`, new `crates/ui/tests/accessibility_guard.rs`
- Test: `accessibility_guard.rs`, snapshots

**Interfaces:**
- Consumes: `GuardState::{Checking, Warn}`; `viewmodel::verdict` helpers for reason text and the suggested-index command with its copy button (reuse, do not rewrite).
- Produces: a notice under the find bar: while `Checking` a "Checking how the query runs…" line (no buttons); in `Warn` the problem title, the reasons, the suggestion with copy, and buttons **Run anyway** and **Cancel** emitting `FindAction::GuardRunAnyway` / `GuardCancel`. Keys under `query.guard.*`: `checking`, `title_collection_scan`, `title_sort_not_covered`, `run_anyway`, `cancel`.

- [ ] **Step 1: Write failing accessibility tests** (labelled buttons, keyboard activation of Run anyway and Cancel, notice announced as a status/alert like the syntax notice) and snapshot scenes `guard_notice.png` (scan with suggestion) in light and dark as the suite does.
- [ ] **Step 2: Run, confirm FAIL. Step 3: Implement the view and keys. Step 4: Regenerate and review only the new baseline; run the full snapshot suite and `cargo +1.87.0 test -p curlybrace-ui`.** **Step 5: Commit** `feat(guard): notice with run anyway before a query that scans`.

### Task 6: Shell literal parser and call reader (pure)

**Files:**
- Create: `crates/ui/src/viewmodel/shell_literal.rs`, `shell_literal_tests.rs`, `shell_queries.rs`, `shell_queries_tests.rs`
- Modify: `crates/ui/src/viewmodel/mod.rs`
- Reuse: `shell_scan::scan_code` (code/non-code classification)

**Interfaces:**
- Consumes: `shell_scan`, `bson::Document`, `Query`.
- Produces:
  - `pub fn parse_literal(text: &str) -> Option<bson::Document>` for an object literal: bare or quoted keys, single/double-quoted strings, numbers, booleans, null, nested objects and arrays, `ObjectId("<24 hex>")`, `ISODate("<ISO 8601>")`; anything else (identifiers as values, function calls, spreads, template strings, computed keys) is `None`. Depth limit 64.
  - `pub struct ShellCall { pub db: String, pub collection: String, pub method: &'static str, pub query: Query, pub range: Range<usize> }`
  - `pub struct Reading { pub calls: Vec<ShellCall>, pub unchecked: usize }`
  - `pub fn read_calls(text: &str, start_db: &str) -> Reading` finding `db.<name>.<method>(<literal filter>…)` and `db.getCollection("<name>").<method>(…)` for methods `find`, `findOne`, `countDocuments`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`, with an optional literal projection second argument and, after `find`/`findOne`, a chained `.sort(<literal>)`; other chained calls are skipped over. `use <name>` and `db.getSiblingDB("<name>")` change the database for later calls when the name is a literal. A matching call whose filter is not a literal counts in `unchecked`. Calls in comments, strings, templates and regex literals are ignored.

- [ ] **Step 1: Write failing tests**: each literal form above; each rejected form; `db.users.find({age: {$gt: 30}}).sort({age: 1}).limit(5)` gives one call with filter and sort; `db.users.find(filter)` gives `unchecked == 1`; a call inside `// ...`, `"..."`, `` `...` `` and `/.../` gives nothing; `use shop` then `db.orders.deleteMany({status: 'x'})` gives db `shop`; `db.getSiblingDB("a").users.find({})` gives db `a`; `db.getSiblingDB(name)` makes later calls unchecked; 30 calls give 30 `calls` (the cap is applied by Task 7); an unterminated script does not panic or loop (fuzz a handful of broken inputs).
- [ ] **Step 2: Run, confirm FAIL. Step 3: Implement** with a small hand-written scanner over `scan_code`'s code mask; keep each file under 400 lines. **Step 4: Run** the viewmodel tests; clippy. **Step 5: Commit** `feat(shell): read literal collection calls from a script`.

### Task 7: Shell guard flow

**Files:**
- Modify: `crates/ui/src/viewmodel/shell.rs` (`ShellStatus::Checking { request: u64 }`, `ShellNotice::IndexMiss { report: GuardReport, ran: Option<Range<usize>> }`, state methods), `crates/ui/src/app/shell_actions.rs`, `reducer.rs` (route `Event::GuardChecked` to a shell when its `request` matches), the text-change handler that drops the syntax notice (drop `IndexMiss` too)
- Create: `crates/ui/src/app/shell_guard_tests.rs`

**Interfaces:**
- Consumes: Tasks 2 and 6; `shell_actions::run(state, id, Check)`; `guard::{GUARD_MAX_TARGETS, GUARD_MIN_DOCS}`.
- Produces:
  - `Check` gains `Guard` handling: order of a Run is syntax check, existing read-only / tool checks, then the guard, then the production confirmation. **Run anyway** on an `IndexMiss` notice runs exactly the text that was checked and skips the guard (and the syntax check) once, keeping every other guard.
  - With the guard on and at least one readable call: send `Command::GuardCheck` (first 20 calls; `unchecked = reading.unchecked + calls beyond 20`), status `Checking`; the Run button area shows the checking state with **Cancel**; Cancel and a script change return to `Idle` and ignore the late answer.
  - `GuardChecked`: `Err` or no findings runs the script as today (production confirmation still follows); findings set `IndexMiss` with `unchecked` added from the reading. A script with no readable call runs as today with no notice and no `Checking`.
  - The notice's `unchecked` is the reading's unchecked count plus the calls beyond the cap plus the report's own.
  - `start_db` is the connection's default database name as the Shell tab uses it (find where the Shell tab starts its database; a selection-only run starts from the same one).

- [ ] **Step 1: Write failing tests** (like `shell_check_app_tests.rs`): guard off runs as today; guard on with no readable call runs as today; with a readable call the first command is `GuardCheck`, not `ShellRun`; the selection is what is read; clean report then `ShellRun` (and on production the write warning parks it); findings then `IndexMiss` and no `ShellRun`; **Run anyway** then `ShellRun` of the checked text, once, and on production the confirmation follows; an `Err` outcome runs; a stale `request` is ignored; Cancel returns to `Idle` and the late answer changes nothing; editing the script drops `IndexMiss`; read-only connection is still refused first; 25 calls send 20 targets and report 5 unchecked.
- [ ] **Step 2: Run, confirm FAIL. Step 3: Implement. Step 4: Run** `cargo +1.87.0 test -p curlybrace-ui`; clippy. **Step 5: Commit** `feat(shell): check a script's queries before it runs`.

### Task 8: Shell notice, strings, README, snapshots

**Files:**
- Create: `crates/ui/src/views/shell_guard.rs`
- Modify: `crates/ui/src/views/shell_notice.rs` / `shell.rs`, `crates/ui/locales/en.json`, `README.md`, `crates/ui/tests/accessibility_shell_editor.rs` (or a new `accessibility_shell_guard.rs`), snapshot scenes
- Test: accessibility, snapshots

**Interfaces:**
- Consumes: `ShellNotice::IndexMiss`, `ShellStatus::Checking`.
- Produces: the notice lists each finding (collection, call text, problem title, suggestion with copy) and "N calls could not be checked" when `unchecked > 0`, with **Run anyway** and **Cancel** (`ShellAction::RunAnyway` and a new `ShellAction::DismissNotice` if none exists); a "Checking queries…" line with **Cancel** while `Checking`. Keys under `shell.guard.*`: `checking`, `title`, `finding`, `unchecked`, `run_anyway`, `cancel`.

- [ ] **Step 1: Write failing accessibility tests** (labels, keyboard, status role) and a snapshot `shell_guard_notice.png`; check the 800×500 window with History open keeps the output pane usable (assert its height is not smaller than the existing syntax notice case).
- [ ] **Step 2: Run, confirm FAIL. Step 3: Implement; add one README line. Step 4: Regenerate and review only the new baseline; run all gates on `cargo +1.87.0`.** **Step 5: Commit** `feat(shell): notice with run anyway before a script that scans`.

### Task 9: Real-server verification and final gates

**Files:**
- Create: `crates/core/tests/local_guard.rs` (ignored by default like `local_explain.rs`; follows the repo's policy that database tests do not run on GitHub)

- [ ] **Step 1:** On a throwaway `/opt/homebrew/bin/mongod` (spare localhost port, temp dbpath), write tests that: a filter on an unindexed field of a 2,000-document collection is reported as `CollectionScan`; after `createIndex` it is not; `explain_plan` does not execute the query (assert `executionStats` absent); a `$text` query without a text index is "not checked"; a read-only user can run the check. Stop and delete the instance afterwards.
- [ ] **Step 2:** Run `cargo +1.87.0 fmt --all --check`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`, and the snapshot suite; all green.
- [ ] **Step 3: Commit** `test(guard): check the guard against a real server`.
