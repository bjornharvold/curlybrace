# Confirmed `$out` / `$merge` run (slice 2b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox syntax.

**Goal:** Run an aggregation pipeline that ends in `$out` or `$merge` for real, behind a typed confirmation, without ever weakening the read-only preview.

**Architecture:** Core builds a run plan (pure, tested) and a new `DocumentStore::aggregate_write` re-checks its shape; a new write `Command::AggregateRun` goes through the existing `mutation(.., Guard::Confirmed, ..)` helper like a bulk write; the aggregate view model gets a run dialog state; views add a Run button and a typed-confirmation dialog.

**Tech Stack:** Rust 1.87 (no let-chains), egui, mongodb driver, existing runtime/command/event plumbing.

**Spec:** `docs/superpowers/specs/2026-11-14-aggregation-slice-2-design.md` (slice 2b section).

## Global Constraints

- Every user-facing string an i18n key under `aggregate.run.*` in `crates/ui/locales/en.json` (nested); report new keys.
- Files under 400 lines (800 max); immutable style; real objects over mocks; TDD; coverage of new code at least 80%.
- `AGGREGATE_RUN_MAX_TIME` = 60 s server side (constant in `crates/core/src/pipeline.rs`); the client waits a little longer (see how `PREVIEW_MAX_TIME_MS` bounds the preview in `runtime/handlers.rs` around line 722).
- No undo snapshot, no cancel, no `allowDiskUse`/options (slice 4), no write stage that is not last, no second write stage.
- The preview command and `aggregate_preview` keep refusing every write stage; the two paths never mix.
- Never log pipeline text; error messages do not echo server text beyond the existing mapped messages.
- Never connect to any MongoDB on this machine. The one integration test uses a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`), deleted afterwards (pattern: `crates/core/tests/local_aggregate.rs`).
- `cargo +1.87.0`; run git as plain separate Bash calls; conventional commits ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push.

## Review Focus

- A preview never forwards a write stage (memory store records what it receives); a run never reaches the store without the typed confirmation state.
- `$out` / `$merge` target resolution: `"coll"`, `{ db, coll }`, `$merge: "coll"`, `$merge: { into: "coll" }`, `$merge: { into: { db, coll } }`; anything else (empty, non-string, missing) is a refusal naming nothing from the pipeline text.
- Read-only: the Run button is disabled with a reason; a connection locked after the dialog opened is refused (UI) and refused again by `ReadOnlyStore` (before anything else).
- A write stage inside `$facet` / `$lookup.pipeline` / `$unionWith.pipeline`, a write stage not last, two write stages, a disabled trailing write stage (last *enabled* stage decides), an empty pipeline: refused, nothing sent.
- Typed confirmation: exact match of the target collection name (case-sensitive, no trimming surprises: compare the trimmed input to the name), always, production or not; changing the pipeline, collection or connection while the dialog is open closes it.
- Timeout and lost connection report an unknown outcome, never "failed" alone; success shows the target's count after.

### Task 1: core run plan

**Files:** Modify `crates/core/src/pipeline.rs` (keep under 400 lines; split to `pipeline_run.rs` + tests if needed), tests in `pipeline_tests.rs` or a new `pipeline_run_tests.rs`.

**Interfaces — Produces:**
- `pub const AGGREGATE_RUN_MAX_TIME_MS: u64 = 60_000;`
- `pub struct RunPlan { pub pipeline: Vec<Document>, pub stage: String /* "$out" | "$merge" */, pub target: RunTarget }`, `pub struct RunTarget { pub db: Option<String>, pub collection: String }` (`db: None` means the source database).
- `pub fn run_pipeline(stages: &[Stage]) -> Result<RunPlan, PipelineError>`: enabled stages only, all of them, no `$limit` added, the last enabled stage must be a write stage, no other write stage (stage position, `$facet`, `$lookup`/`$unionWith` pipelines).
- New `PipelineError` variants: `NoWriteStage`, `TargetUnresolved { index }` (reuse `WriteStageNotLast`, `NestedWriteStage`, `Empty`, `NotAStage`).
- `pub fn ensure_run_shape(pipeline: &[Document]) -> Result<(), CoreError>`: exactly one write stage at stage position and it is the last stage, none nested. Used by the store layer (defence in depth).

- [ ] Failing tests: each target form in Review Focus; refusals (not last, two, nested, empty, no write stage, unresolvable target, disabled trailing write stage means no write stage); no `$limit` appended; `ensure_run_shape` accepts a good pipeline and refuses the same cases with a message that names the stage, never the text.
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core pipeline`; commit `feat(aggregate): plan a confirmed write run`.

### Task 2: store, command, handler

**Files:** Modify `crates/core/src/store.rs` (`aggregate_write`), `mongo_store.rs` + `mongo_pipeline.rs` (driver call: `ensure_run_shape`, then aggregate with `max_time`, drain the cursor, `classify_driver_error`), `read_only_store.rs` (`self.guard()?` first), `test_support/memory_store.rs` + `memory_aggregate.rs` (record received pipelines, scriptable result), `runtime/mod.rs` (`Command::AggregateRun { session, ns, pipeline, run }`, `Event::AggregateRan { session, ns, target, outcome }`), `runtime/command.rs` (`is_write` true, name `"aggregate_run"`, session lookup), `runtime/handlers.rs`.

**Interfaces — Produces:**
- `async fn aggregate_write(&self, ns: &Namespace, pipeline: Vec<Document>, max_time: Duration) -> Result<()>` (default: `Unsupported`).
- `Event::AggregateRan { session, ns /* source */, target: Namespace, outcome: Result<u64, CoreError> /* estimated count of the target after */ }`. A pipeline or server failure and a timeout are an `Err` outcome shown inline, never `Event::Failed`; a lost connection is returned as the command's failure so the existing write-interruption path (ping settle, "outcome unknown" wording as for an interrupted bulk save) applies. The handler computes `target` from the pipeline's last stage with the same resolver as Task 1 (export it from `pipeline.rs`).

- [ ] Failing tests: handler runs the pipeline on the in-memory store and answers with the count; a preview command never forwards a write stage (existing tests stay green); read-only session answers `Event::Failed { error: ReadOnly }`; extend `read_only_tests.rs::is_write_agrees_with_what_the_read_only_store_refuses` with `AggregateRun`; `ReadOnlyStore::aggregate_write` refuses before the inner store sees it; store refuses a malformed shape; `Command::AggregateRun.is_write()`.
- [ ] Integration test `crates/core/tests/local_aggregate_run.rs` on a throwaway mongod: `$out` replaces the target and `$merge` merges; source untouched; count returned.
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core`; commit `feat(aggregate): run a write pipeline through the store`.

### Task 3: view model and reducer

**Files:** Create `crates/ui/src/viewmodel/aggregate_run.rs` (+ tests), `crates/ui/src/app/aggregate_run_actions.rs` (+ tests); modify `viewmodel/aggregate.rs` (hold `RunUi`, keep under 400 lines), `app/aggregate_actions.rs` (dispatch), `app/write_guard.rs` (`WriteKind::AggregateRun`, wording arm), the event handler that receives `Event::AggregateRan`.

**Interfaces — Produces:**
- `RunUi { dialog: Option<RunDialog>, status: RunStatus }`; `RunDialog { target: RunTarget, source: Namespace, typed: String, target_count: Option<u64> }`; `RunStatus { Idle, Running, Done { target: Namespace, count: u64 }, Failed(CoreError), Unknown }`.
- `AggregateAction::{ RunClicked, RunTyped(String), RunConfirmed, RunDismissed }`; `AggregateState::can_run() -> RunAvailability { Hidden, Disabled(reason), Ready }` (Hidden unless the last enabled stage is `$out`/`$merge`; Disabled for read-only, parse errors, no connection).
- `RunConfirmed` builds the command with `core::pipeline::run_pipeline`, then `mutation(state, None, WriteKind::AggregateRun, Guard::Confirmed, build)`; it requires `typed.trim() == target.collection`, the selected ns equal to the dialog's source, and a connection that is not read-only; otherwise nothing is sent. `RunClicked` also sends `Command::EstimatedCount` for the target (same db rules) to fill `target_count`.
- On `Event::AggregateRan`: `Done`/`Failed`/`Unknown` status, dialog closed, collection list refreshed when the target is new, target count shown.

- [ ] Failing tests: availability matrix; typed mismatch sends nothing; match sends exactly one `AggregateRun` and no `Guard::Warn` parking on a production connection; read-only refused with the banner; editing the pipeline, switching collection or session closes the dialog; success, error, timeout and interruption statuses.
- [ ] Implement; commit `feat(aggregate): confirm and run a write pipeline`.

### Task 4: views, i18n, layout, docs

**Files:** Create `crates/ui/src/views/aggregate_run.rs` (+ tests); modify `views/aggregate.rs` (Run button next to Refresh, status line), `crates/ui/locales/en.json` (`aggregate.run.*`), `crates/ui/tests/` (accessibility at 1000x700 and 800x500; light/dark snapshot of the dialog; regenerate only baselines that legitimately change), `README.md` (Aggregate section: confirmed `$out`/`$merge` run now exists; update "Not yet").

**Interfaces — Consumes:** Task 3 actions and state. The dialog states: source collection, `db.collection` target, what the stage does (`$out` replaces the whole target collection; `$merge` inserts, merges, replaces or fails per its options), the target's current count, "cannot be undone by the app", the production badge on a production connection, a name field (focus lands there, Enter confirms only when the name matches, Escape cancels), the Run button disabled until it matches.

- [ ] Failing kittest tests: Run hidden for a read pipeline; disabled with a reason on read-only; dialog texts and named controls; keyboard operation; no clipping at 800x500.
- [ ] Implement; `cargo +1.87.0 fmt --all`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`; commit `feat(aggregate): add the run button and its confirmation` and `docs(readme): describe the confirmed run`.
- [ ] The site (`site/index.html`) is updated in a separate docs PR after the release, not here.
