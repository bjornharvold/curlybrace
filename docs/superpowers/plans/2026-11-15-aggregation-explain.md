# Aggregation explain (slice 3b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** An Explain section on the Aggregate tab showing the server's plan for the enabled stages up to the selected one, as a typed tree with hints, tabs Preview | Plan | Raw.

**Architecture:** Core: `DocumentStore::aggregate_explain`, a typed parser of the aggregate explain document (`AggPlan`), `Command::AggregateExplain` / `Event::AggregateExplained` numbered like the preview. UI: per-session explain state next to the preview, actions, a Plan tab and a Raw tab, i18n `aggregate.explain.*`.

**Tech Stack:** Rust 1.87 (no let-chains), `bson`, egui. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-15-aggregation-slice-3-design.md` (slice 3b section). Model on `aggregate_preview` (store, read-only store, runtime handler, `Command::AggregatePreview`, `Event::AggregatePreviewed`, UI numbering and stale-answer rules) and on `explain_find` / `explain/` (`classify`, `Verdict`, hint vocabulary).

## Global Constraints

- Every user-facing string an i18n key under `aggregate.explain.*` in `crates/ui/locales/en.json` (nested); report new keys.
- Files 200-400 lines (800 max); immutable style; real objects over mocks; TDD; coverage of new code at least 80%. No new dependency.
- Read only. The pipeline sent is the preview plan's enabled stages up to and including the selected one, **without** a write stage and **without** the preview `$limit`; the store refuses a write stage at any depth again (as `aggregate_preview` does). `ReadOnlyStore` passes it through. Never `allPlansExecution`.
- Verbosity: `QueryPlanner` (default, never executes) or `ExecutionStats` (on request, with a visible cost note: it runs the pipeline up to `AGGREGATE_EXPLAIN_MAX_TIME_MS` = 10 000 server side). The client waits slightly longer.
- Explain documents hold user data: never logged; errors name the variant, not content. Raw tab shows the document as received through the existing pretty JSON path (find how `explain_find` raw/JSON is shown if any, else the document viewer's extended JSON).
- Numbered requests, stale answers ignored, never a banner (inline outcome like the preview). A connection loss marks the explain as failed like the preview.
- `cargo +1.87.0`; run git as plain separate Bash calls; conventional commits (`feat(aggregate): ...`) ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never connect to any MongoDB on the machine; integration tests only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted afterwards (use the existing local_* integration test pattern, `CURLYBRACE_IT`).

## Review Focus

- Shapes: old `stages` array with a leading `$cursor`, newer top-level `queryPlanner` (pipeline fully pushed down, no `stages`), sharded (`shards` map), `executionStats` and `queryPlanner` only, `$lookup` / `$facet` sub-pipelines, an explain with an error `ok: 0`, missing fields everywhere. The parser never panics and never indexes blindly; unknown shapes degrade to the raw tab with a "plan not understood" state, not an error.
- A write stage anywhere (also nested in `$facet`/`$lookup`) is refused before anything is sent; read-only connections can explain.
- Stale answers (older request number, other session, other collection) never overwrite a newer state; a draft edit after the request marks the plan as out of date but keeps showing it.
- Hint wording: collection scan, in-memory sort (`SORT` stage / `$sort` with `usedDisk`), docs examined far above returned; reuse `explain::classify` where the winning-plan shape matches.
- Layout works at 800x500; Plan tree keyboard reachable; all controls named.

### Task 1: core plan model, parser, store, runtime

**Files:** create `crates/core/src/explain/agg_plan.rs` (+ `agg_plan_tests.rs`), fixtures `crates/core/tests/fixtures/explain_aggregate/*.json` (hand-written, realistic: collscan, ixscan, group, lookup, facet, sharded, old `stages` form, new top-level `queryPlanner` form, executionStats, malformed); modify `explain/mod.rs`, `store.rs` (`aggregate_explain` default refusing `Unsupported` like peers), `read_only_store.rs`, `mongo_store.rs` / `mongo_pipeline.rs` (the real call: run `explain` command with `aggregate` + `pipeline` + `cursor: {}` and `maxTimeMS`, verbosity mapped), `test_support/memory_store.rs` (records the call, returns a canned explain), `runtime/command.rs`, `runtime/event.rs` (or where Event lives), `runtime/handlers.rs`, `runtime/aggregate_explain.rs` (+ tests), and `pipeline.rs` helper `explain_pipeline(stages, upto) -> Result<Vec<Document>, PipelineError>` (preview plan minus `$limit`, write stage refused, same helpers as `preview_pipeline`).

**Interfaces — Produces:**
- `pub enum ExplainVerbosity { QueryPlanner, ExecutionStats }`; `pub const AGGREGATE_EXPLAIN_MAX_TIME_MS: u64 = 10_000`.
- `pub struct AggPlan { pub summary: AggSummary, pub input: Option<InputPlan>, pub stages: Vec<AggStage>, pub shards: Vec<(String, AggPlan)>, pub hints: Vec<AggHint> }`; `AggStage { name, detail: Option<String> (short, derived, no user data beyond field names), keys_examined, docs_examined, n_returned: Option<u64>, millis: Option<u64> }`; `InputPlan { stage_kind (COLLSCAN/IXSCAN/..), index: Option<String>, keys_examined, docs_examined, n_returned }`; `AggSummary { verbosity_seen, total_millis, n_returned, keys_examined, docs_examined }`; `AggHint` an enum (`CollectionScan`, `InMemorySort`, `DiskSort`, `ExaminedMuchMoreThanReturned`, ...) with stable `key()` for i18n.
- `pub fn parse_aggregate_explain(doc: &Document) -> AggPlanOutcome` where `AggPlanOutcome::{Plan(AggPlan), NotUnderstood}` (never panics).
- `DocumentStore::aggregate_explain(&self, ns: &Namespace, pipeline: Vec<Document>, verbosity: ExplainVerbosity, max_time: Duration) -> Result<Explain>` (same `Explain` type `explain_find` returns).
- `Command::AggregateExplain { session, request: u64, ns, stages: Vec<Document>, verbosity }` (a read; not `is_write`), `Event::AggregateExplained { session, ns, request, verbosity, outcome: Result<Explain-doc> }`; check how `AggregatePreview`/`AggregatePreviewed` carry session, request number and ns and mirror it exactly.

- [ ] Failing tests: parser per fixture (stage names, counters, index, hints), `NotUnderstood` for garbage and `ok: 0`, store refuses write stages (top-level, nested in `$facet` and `$lookup`) with nothing sent (memory store records), read-only store passes it, runtime handler returns numbered event, explain failure maps to the event's error like the preview, `explain_pipeline` drops `$limit` and refuses a write stage.
- [ ] Implement; integration test `crates/core/tests/local_aggregate_explain.rs` against a throwaway mongod (skipped unless `CURLYBRACE_IT`); `cargo +1.87.0 test -p curlybrace-core`; commit `feat(aggregate): explain a pipeline in core` (split into several commits is fine).

### Task 2: UI state, actions, views, i18n, docs

**Files:** create `crates/ui/src/viewmodel/aggregate_explain.rs` (+ tests), `app/aggregate_explain_actions.rs` (+ tests), `views/aggregate_explain.rs` (+ tests, Plan tab, Raw tab, hints), modify `viewmodel/aggregate.rs` (keep under 400 lines; put new state in the new file), `AggregateAction` (new `Explain(ExplainAction)` variant dispatched like `Code`), `views/aggregate.rs` (Preview | Plan | Raw tabs under the preview; Explain button in the toolbar; keep file under 400 lines, split if needed), reducer dispatch for `Event::AggregateExplained` and loss handling (follow `reducer_lost.rs`), `locales/en.json` (`aggregate.explain.*`), `crates/ui/tests/` (accessibility at 1000x700 and 800x500 with the Plan tab shown; a light/dark snapshot of the Plan tab with a canned plan), `README.md` (a short bullet; roadmap wording if #69 is listed).

**Interfaces — Consumes:** Task 1. **Produces:** `ExplainUi { state: Idle | Loading{request} | Done{plan: AggPlanOutcome, raw: Document, verbosity, for_draft: Fingerprint} | Failed{reason}, verbosity: ExplainVerbosity, tab: ResultTab }`; `ExplainAction::{ Run, SetVerbosity(ExplainVerbosity), SelectTab(ResultTab), CopyRaw }`. Explain is disabled with the preview's reason when no enabled stage up to the selected one parses, when there is none, or when a write stage lies in the explained range; a request already in flight is superseded by number (follow the preview's queue/in-flight bookkeeping, `aggregate_flight_tests.rs`). A plan whose `for_draft` fingerprint differs from the current draft shows an "out of date" mark. executionStats shows the cost note next to its toggle. Plan tab: summary line, input stage row (index used or "collection scan"), one row per stage with counters/time, hints list in words, sharded plans one collapsible group per shard; unknown shape shows a "not understood, see Raw" message.

- [ ] Failing tests: reducer (run sends the numbered command with the exact stages, stale answer ignored, failure inline, session loss, draft edit marks out of date), views (named controls, tabs keyboard operable, works at 800x500), disabled matrix.
- [ ] Implement; `cargo +1.87.0 fmt --all`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`; commits `feat(aggregate): add the explain section` and `docs(readme): describe pipeline explain`.
- [ ] The site is updated in a separate docs PR after the release, not here.
