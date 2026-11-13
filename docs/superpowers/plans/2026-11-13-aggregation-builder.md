# Aggregation builder (slice 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An **Aggregate** collection tab where a pipeline is built stage by stage (stage list or text mode) and the documents after the selected stage are previewed, read-only and time-bounded.

**Architecture:** Core owns the pipeline model (parse, validate, build the preview pipeline with write stages removed/refused) and one new read, `DocumentStore::aggregate_preview`. One runtime command/event pair carries a numbered preview request. The UI keeps a per-session `AggregateState` (stages, selection, mode, last preview) and a new view reusing the Tree/Table/JSON document views for the preview.

**Tech Stack:** Rust 1.87.0, egui, `mongodb`/`bson`, tokio, `egui_kittest`.

**Spec:** `docs/superpowers/specs/2026-11-13-aggregation-builder-design.md`

## Global Constraints

- CI pins Rust 1.87.0: `cargo +1.87.0` `fmt --all --check`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`. No let-chains.
- No new dependency; nothing persisted in slice 1.
- Every user-facing string an i18n key in `crates/ui/locales/en.json` (nested, prefix `aggregate.`; add the prefix to `PREFIXES` in `crates/ui/src/i18n_usage_tests.rs`); report new keys.
- Files < 400 lines (800 max); immutable updates; explicit error handling; never log pipelines (log `error_variant` only).
- Never connect to a MongoDB already on the machine; real-server checks use a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped and deleted afterwards. Never print passwords.
- Do not edit `CHANGELOG.md` or the spec. Do not regenerate snapshot baselines except in Task 9.
- Constants in core `crates/core/src/pipeline.rs`: `PREVIEW_LIMIT = 20`, `PREVIEW_MAX_TIME_MS = 10_000`, `WRITE_STAGES = ["$out", "$merge"]`.
- Conventional commits (lowercase imperative subject); trailers `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`.

## Review Focus

1. **No write ever leaves a preview.** `$out`/`$merge` at the end are dropped and reported "not run"; anywhere else, including inside `$facet`, `$lookup`, `$unionWith` sub-pipelines, the plan is refused; `aggregate_preview` itself re-checks and refuses at any depth; the in-memory store records received stages. Test each.
2. A stale answer (newer request, edited/moved/toggled stage, other session) is dropped; a timeout or error keeps the last good preview and says why.
3. Switching tab, session or mode keeps the draft; invalid text blocks leaving text mode and shows line/column.
4. Preview works on read-only and production sessions without any confirmation, and the command is not a write (`Command::is_write` false).
5. Empty pipeline, all stages disabled, a stage with two keys, a stage not starting with `$`, an array that is not an array of objects, deeply nested JSON: each gives a clear error, never a panic or a sent request.

---

### Task 1: Core pipeline model

**Files:** Create `crates/core/src/pipeline.rs`, `crates/core/src/pipeline_tests.rs`; modify `crates/core/src/lib.rs`.

**Interfaces — Produces:**
- `pub struct Stage { pub text: String, pub enabled: bool }` (derive Clone, Debug, PartialEq).
- `pub enum PipelineError` (thiserror-free, same style as `CoreError` use): `Syntax { line: usize, column: usize, message: String }`, `NotAnArray`, `NotAStage { index: usize }` (not exactly one `$`-key object), `WriteStageNotLast { index: usize, name: String }`, `NestedWriteStage { index: usize, name: String }`, `Empty`; `PipelineError` converts to `CoreError::InvalidJson`/`InvalidDocument` where the UI needs the shared taxonomy (provide `fn into_core(self) -> CoreError`).
- `pub fn parse_pipeline_text(text: &str) -> Result<Vec<Stage>, PipelineError>`: JSON array of objects → stages (each stage text pretty-printed 2-space JSON); `Syntax` carries serde's line/column.
- `pub fn pipeline_text(stages: &[Stage]) -> String`: enabled and disabled stages both, as a JSON array (disabled ones included only in a stage-list round trip; document the rule in the doc comment: text mode shows ALL stages, disabled ones are lost on a text round trip, so the UI blocks nothing but warns in the view later — keep the function pure).
- `pub fn stage_name(text: &str) -> Option<String>`: the single `$key` when the text parses.
- `pub struct PreviewPlan { pub pipeline: Vec<bson::Document>, pub not_run: Vec<String> }`.
- `pub fn preview_pipeline(stages: &[Stage], upto: usize, limit: i64) -> Result<PreviewPlan, PipelineError>`: enabled stages with index <= `upto`, each parsed (`NotAStage` if not one `$`-key object); a trailing `$out`/`$merge` is dropped and named in `not_run`; a write stage elsewhere → `WriteStageNotLast`; a write stage nested at any depth under `$facet`/`$lookup`/`$unionWith` (walk arrays/documents) → `NestedWriteStage`; no stages left → `Empty`; append `{ "$limit": limit }`.
- `pub fn contains_write_stage(pipeline: &[bson::Document]) -> Option<String>` (any depth) for Task 2's defence in depth.

- [ ] Write failing tests first (`pipeline_tests.rs`): syntax error line/column; not an array; element that is not an object; stage with two keys / without `$`; trailing `$out` and `$merge` dropped and named; `$merge` in the middle refused; `$out` inside a `$facet` branch and inside a `$lookup` pipeline and `$unionWith` refused; disabled stages skipped; `upto` respected; `$limit` appended last and equal to `limit`; empty and all-disabled → `Empty`; deeply nested JSON (e.g. 200 levels) returns an error or a plan, never panics; `stage_name`; text round trip.
- [ ] Run to see them fail; implement; run to green. Use `serde_json` → `Bson::try_from` like `query::parse_doc` (canonical extended JSON); reuse its recursion-limit approach.
- [ ] Commit `feat(pipeline): parse a pipeline and build a read-only preview`.

### Task 2: Store read `aggregate_preview`

**Files:** Modify `crates/core/src/store.rs`, `mongo_store.rs` (or a new `mongo_pipeline.rs` if it grows), `read_only_store.rs`, `read_only_store_tests.rs`, `test_support/memory_store.rs`, plus a tests file.

**Interfaces — Consumes:** `contains_write_stage`, `PREVIEW_MAX_TIME_MS`. **Produces:** `async fn aggregate_preview(&self, ns: &Namespace, pipeline: Vec<Document>, max_time: Duration) -> Result<Vec<Document>>` on `DocumentStore`, default `Err(CoreError::Unsupported)` like `explain_plan`.

- [ ] Failing tests: the trait default is `Unsupported`; the Mongo path is covered in Task 8; `ReadOnlyStore` forwards it and does **not** refuse it (it is a read) — add it to the forwarded list and a test; the memory store: add `with_aggregate(results)` scripted answer and `received_pipelines()` recording; a pipeline containing a write stage at any depth returns `CoreError::InvalidDocument`-style refusal **before** any call and the recording store shows nothing received.
- [ ] Implement. Mongo: `collection.aggregate(pipeline).max_time(max_time)` with `allow_disk_use(false)`, `try_collect`; map errors with `db_err`; server code 50 → the existing timeout mapping. Re-check `contains_write_stage` first (defence in depth).
- [ ] Commit `feat(core): add a read-only aggregate preview to the store`.

### Task 3: Runtime command and event

**Files:** Modify `runtime/mod.rs`, `command.rs`, `dispatch.rs`, `handlers.rs`; create `runtime/aggregate_tests.rs`.

**Interfaces — Produces:** `Command::AggregatePreview { session: SessionId, request: u64, ns: Namespace, pipeline: Vec<Document> }`; `Event::AggregatePreviewed { session, request: u64, outcome: Result<Vec<Document>, CoreError> }`. Not a write (`is_write` false). Update `request()`, `context()`, `session()`, `Event::session` (exhaustive matches).

- [ ] Failing tests (model on `runtime/guard_tests.rs`): a scripted store returns docs → `AggregatePreviewed` Ok with the same request; a store error → Ok event carrying `Err` (not a banner, not `Unreachable`); a read-only session still previews; a pipeline with `$out` is refused with `Err` and nothing reaches the store; a client-side timeout becomes an `Err` outcome with the settle-by-ping skipped (mirror the explain/guard special case at `handlers.rs` ~253-269); a superseded session drops it.
- [ ] Implement in `data_command`; use `PREVIEW_MAX_TIME_MS` and the config time limit.
- [ ] Commit `feat(runtime): run an aggregate preview as a numbered read`.

### Task 4: UI state and pure view model

**Files:** Create `crates/ui/src/viewmodel/aggregate.rs` (+ `aggregate_tests.rs`), `crates/ui/src/app/aggregate_state.rs`; modify `viewmodel/indexes.rs` (`CollectionTab::Aggregate`, `label_key`), `views/collection_tabs.rs` (`TABS` len 3), `app/session.rs` (`SessionView`, `take_view`, `restore_view`), `app/state.rs`.

**Interfaces — Produces:** `AggregateState { stages: Vec<Stage>, selected: usize, mode: Mode, text: String, text_error: Option<TextError>, preview: PreviewState, request: u64 }`; `Mode { Stages, Text }`; `PreviewState { Idle, Loading, Shown { docs, not_run }, Failed { error, last: Option<..> } }`. Pure immutable ops, each returning a new state: `add_stage(name)` (templates for the menu names), `remove(i)`, `move_up(i)`, `move_down(i)`, `toggle(i)`, `edit(i, text)`, `select(i)`, `to_text()`, `from_text()` (blocks on error, keeping state + error), `begin_preview() -> (Self, Option<(u64, Vec<Document>)>)` (bumps `request`, builds the plan via `preview_pipeline`; a `PipelineError` becomes `Failed` without a request), `arrived(request, outcome) -> Self` (stale request ignored), `STAGE_MENU: [&str; N]`.

- [ ] Failing tests for every op and for staleness, selection clamping after remove, move at the edges, toggle leaving the selection valid, mode switching with invalid text, template bodies parse as stages.
- [ ] Implement; wire `AggregateState` into `SessionView` (and its exhaustive `take_view`/`restore_view`), default per session.
- [ ] Commit `feat(aggregate): add the pipeline draft state`.

### Task 5: Reducer wiring

**Files:** Create `crates/ui/src/app/aggregate_actions.rs` (+ tests); modify `app/mod.rs` (`Input::Aggregate(AggregateAction)`), `reducer.rs`, `index_actions.rs` (`switch_tab` for the new variant).

**Interfaces — Produces:** `AggregateAction { AddStage(String), Remove(usize), MoveUp(usize), MoveDown(usize), Toggle(usize), Edit(usize, String), Select(usize), SetMode(Mode), EditText(String), Refresh }`. Each that changes the pipeline or selection returns `Command::AggregatePreview` via `begin_preview` (debounce for `Edit`/`EditText` is a UI frame concern: the view sends `Refresh`-style intent after idle; the reducer sends immediately on every action it is given). `Event::AggregatePreviewed` handled in `on_session_event` (add it to the exhaustive match and the session-less no-op list as needed): stale dropped, error → `PreviewState::Failed` with the translated error key, never a banner.

- [ ] Failing tests (`update(state, Input::..)` style): each action returns the right command and state; a stale event is ignored; an error keeps the last good preview; a session switch keeps each draft; read-only and production sessions return the command with no confirmation; selecting the Aggregate tab on a collection with no selection does nothing odd.
- [ ] Implement; commit `feat(aggregate): drive the pipeline draft from the reducer`.

### Task 6: Views and i18n

**Files:** Create `crates/ui/src/views/aggregate.rs` (+ `aggregate_stage.rs`, `aggregate_text.rs` if over 400 lines); modify `app/browse_panel.rs` (route `CollectionTab::Aggregate`), `locales/en.json`, `i18n_usage_tests.rs`.

- [ ] Stage list: **Add stage** menu (names from `STAGE_MENU`), per stage a card with enable checkbox, position, stage name, multi-line JSON editor (reuse `views/shell_editor.rs` patterns or the plain multiline `TextEdit` with `json_highlight`), Move up/down, Remove, and a one-line valid/error status; the selected card is marked. Text mode toggle with one editor and inline `line:column message`. Preview pane under/right: `show_documents_switchable` over a `Page` built from the preview docs with edit/delete disabled (`tree.projected = true`, `copy_blocked` as for other read-only views), plus status lines: loading, "N stages not run: $out", timeout/error text, "showing the first 20".
- [ ] Every string an `aggregate.*` key; controls named for accessibility; 32 pt controls; headers dense (one wrapped row for Add stage, mode toggle, Refresh).
- [ ] Commit `feat(aggregate): the Aggregate tab`.

### Task 7: Stage-name completion

**Files:** Modify `crates/ui/src/viewmodel/shell_complete_ops.rs` (expose the stage-name subset of `OPERATORS`), `views/aggregate_stage.rs`; test in `shell_complete_tests` style.

- [ ] Failing test: at an empty stage text or a top-level `{ "$m` the popup offers `$match`, `$merge`; elsewhere the existing operator list; never inside strings. Reuse `complete`'s popup widget; no new dependency. Commit `feat(aggregate): complete stage names`.

### Task 8: Real-server tests

**Files:** Create `crates/core/tests/local_aggregate.rs`; modify `scripts/integration-tests.sh`.

- [ ] On a throwaway mongod (see Global Constraints), with `cbit_<id>` databases: `$group`, `$lookup`, `$unwind`, `$facet` previews return the expected documents capped at the limit; a trailing `$out` is not run (target collection absent afterwards) and a `$merge` likewise; a nested `$out` is refused before the server; `maxTimeMS` expiry yields the timeout error (fail point as in `local_explain.rs`, or skip with a clear message if not available); a read-only store still previews. Commit `test(aggregate): check previews against a real server`.

### Task 9: Accessibility, snapshots, docs

**Files:** Create `crates/ui/tests/accessibility_aggregate.rs`, `tests/support/aggregate_shots.rs`, snapshot entries in `tests/snapshots.rs`; modify README (replace the "aggregation builder is not a focus" line at ~272 and add a short **Aggregate** section) — nothing in CHANGELOG.

- [ ] kittest: every control has a role/name; Tab order; stage add/remove/move work; text mode error shows line/column; the preview pane lists documents. Snapshots (light + dark) of the tab with a three-stage pipeline and a preview, then regenerate baselines for any page whose chrome changed (`CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots -- --include-ignored`) and review them visually. Run fmt, clippy, the full workspace tests. Commit `test(aggregate): accessibility, snapshots and README`.
