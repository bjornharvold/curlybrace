# Per-stage counts (slice 3c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** A **Count stages** button on the Aggregate tab that shows, on every stage card, how many documents that stage lets through ("N documents", or "at least N" when capped), computed on demand, one stage at a time.

**Architecture:** Core: `count_pipeline(stages, upto, cap)` next to `preview_pipeline` (the enabled stages `0..=upto`, then `{$limit: cap}` and `{$count: "n"}`), a small `Command::AggregateCount` / `Event::AggregateCounted` pair that reuses `DocumentStore::aggregate_preview` for the transport (a `$count` pipeline is a read) but is **separate from the preview command** so a count answer can never touch the preview page or its numbering. UI: a sequencer in the aggregate view model that sends one command at a time, a per-stage result on each card, a Count stages / Cancel button.

**Deviation from the spec (my call):** the spec said no new runtime command (reuse `AggregatePreview`). `Event::AggregatePreviewed` writes into the preview state and its request numbering, so counts would clobber the preview; a tiny dedicated command/event avoids that and keeps the runtime handler a thin wrapper of the preview one.

**Tech Stack:** Rust 1.87 (no let-chains), `bson`, egui. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-15-aggregation-slice-3-design.md` (slice 3c section). Model on `aggregate_preview` (core `pipeline.rs::preview_pipeline`, `mongo_pipeline.rs`, runtime `aggregate.rs` handler, `Command::AggregatePreview`/`Event::AggregatePreviewed`), the explain slice just merged (`viewmodel/aggregate_explain.rs`, `app/aggregate_explain_actions.rs`, how `AggregateAction::Explain` is dispatched before the draft clone, numbering and stale rules, `reducer_lost.rs`) and `aggregate_flight_tests.rs`.

## Global Constraints

- Every user-facing string an i18n key under `aggregate.counts.*` in `crates/ui/locales/en.json` (nested); report new keys.
- Files 200-400 lines (800 max); immutable style (no throwaway deep clones); real objects over mocks; TDD; coverage of new code at least 80%. No new dependency.
- `COUNT_CAP = 100_000` (public const in core). Each count runs `stages[0..=i]` (enabled stages only, as the preview plan sends them; **no write stage ever**: a write stage in the range disables counting for stages at and after it, a trailing write stage is not counted) then `{$limit: COUNT_CAP}` then `{$count: "n"}`; `maxTimeMS` = the preview's (`PREVIEW_MAX_TIME_MS`), `allowDiskUse` false like the preview. A result equal to `COUNT_CAP` is shown as "at least 100 000" (locale-formatted via the existing number formatting if there is one, else plain digits).
- One bounded aggregation per enabled stage, **one after another, never in parallel**; a stage that fails or times out shows its error on that card and the sequence goes on to the next stage; a newer draft (any stage edit/toggle/move/add/remove, collection or connection change, connection loss) cancels the sequence and discards results (stale answers ignored by sequence number and stage index); results already shown stay visible but are marked out of date when the draft changes after they were computed (same idea as the explain's out-of-date mark).
- Read only; allowed on read-only connections. Never automatic, never while typing. Pipeline text, counts and errors never logged (log request numbers/variants only).
- A `$count` pipeline over a huge collection may scan a lot: `$limit: COUNT_CAP` before `$count` bounds the output of the scanned stage chain but a `$match` without index can still scan; `maxTimeMS` bounds the time and a timeout is shown on the card as "timed out", not as a count.
- `cargo +1.87.0`; run git as plain separate Bash calls; conventional commits (`feat(aggregate): ...`) ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. NEVER connect to any MongoDB already on the machine (the user has others, incl. 27017); integration tests only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted afterwards (existing `local_*` pattern, `CURLYBRACE_IT`).

## Review Focus

- A write stage anywhere in the counted range (also nested in `$facet`/`$lookup`) is never sent; stages before it count, stages from it on are marked "not counted" with a reason.
- Disabled stages are skipped (no count shown on them); a stage that does not parse shows its parse error and the sequence stops for later stages that depend on it (they need the earlier stages to parse).
- Sequencer: only one request in flight; cancelling (draft edit, Cancel button, connection loss, session switch) never leaves a stuck "counting" state or lets a late answer write to a newer draft; a double click on Count stages does not start two sequences.
- Counts of `0` show as "0 documents" (an empty `$count` result means 0: the server returns no document); the cap, singular/plural ("1 document"), timeouts and errors are worded and keyed.
- Layout at 800x500, controls named, keyboard operable; count labels do not shift stage-card layout noticeably (reserve height).

### Task 1: core and runtime

**Files:** modify `crates/core/src/pipeline.rs` (or a new `pipeline_count.rs` + tests to keep files small) with `count_pipeline(stages: &[Stage], upto: usize, cap: i64) -> Result<Vec<Document>, PipelineError>` (shares `read_stages`; appends `$limit` then `$count`; a trailing write stage dropped as the explain does? **No**: counting stage `i` where a write stage is at index <= i is refused with `PipelineError::...` the same variants the preview uses), `COUNT_CAP`, `pub fn count_of(docs: &[Document]) -> u64` reading `n` (empty = 0, i32/i64/double tolerated, anything else an error); `runtime/command.rs` + event: `Command::AggregateCount { session, request: u64, ns, stage_index: usize, pipeline: Vec<Document> }` (a read, not `is_write`, context `"aggregate_count"`) and `Event::AggregateCounted { session, request, ns, stage_index, outcome: Result<u64, CoreError> }`; handler in `runtime/aggregate_count.rs` (+ tests) calling `store.aggregate_preview(ns, pipeline, PREVIEW_MAX_TIME)` and `count_of`, with the preview's client timeout rule; a lost connection comes back like the preview's `RequestFailed` with context and request number. The store layer is **unchanged** except that `aggregate_preview` must keep refusing write stages (it does).

- [ ] Failing tests: `count_pipeline` shape (limit then count, enabled stages only, disabled skipped, write stage refused, parse errors), `count_of` (empty, i32, i64, double, garbage), handler (success, failure inline, numbered, read-only passes, nothing sent for a refused pipeline), cap boundary.
- [ ] Implement; integration test `crates/core/tests/local_aggregate_count.rs` (throwaway mongod, `CURLYBRACE_IT`): counts for a match/group/limit pipeline equal the true counts, cap reached on a collection bigger than a small test cap, an empty result gives 0, a timed-out count errors. `cargo +1.87.0 test -p curlybrace-core`; commits `feat(aggregate): count the documents a stage lets through`.

### Task 2: UI sequencer, views, i18n, docs

**Files:** create `crates/ui/src/viewmodel/aggregate_counts.rs` (+ tests), `app/aggregate_counts_actions.rs` (+ tests), `views/aggregate_counts.rs` (+ tests); modify `viewmodel/aggregate.rs` (state field in the new file, keep under 400 lines), `AggregateAction::Counts(CountsAction)` (dispatched like `Explain`, before the draft clone), the stage-card view (a count line on each card; reserve space), the toolbar (Count stages / Cancel counting), reducer dispatch for `Event::AggregateCounted` and connection loss (`reducer_lost.rs`), `locales/en.json` (`aggregate.counts.*`), `crates/ui/tests/` (accessibility at 1000x700 and 800x500 with counts shown; a light/dark snapshot of the Aggregate tab with counts on cards), `README.md` (a short bullet; roadmap wording for per-stage counts).

**Interfaces — Produces:** `CountsUi { run: Idle | Running{seq, next: usize, stages: Vec<usize>} , results: Vec<(stage_key, StageCount)>, for_draft: Arc<..> }` with `StageCount::{ Exact(u64), AtLeast(u64), Failed(CoreError-key), NotCounted(reason), TimedOut }`; `CountsAction::{ Start, Cancel }`; one command in flight at a time, the next stage sent when the previous answer (or failure) arrives; an answer with the wrong seq/stage index/session/ns is ignored. Count stages is disabled with the preview's reason when no enabled stage exists or nothing parses (the same reason source as Explain); results out of date when the draft's stage list differs from the one counted.

- [ ] Failing tests: reducer (Start sends stage 0 then 1.. in order, one at a time; a failure on one stage does not stop the next; stale answer ignored; draft edit cancels and marks results; Cancel; connection loss; session switch; double Start ignored; write stage -> not counted), views (named controls, 800x500, count labels singular/plural/cap/error/timed out), disabled matrix.
- [ ] Implement; `cargo +1.87.0 fmt --all`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`; commits `feat(aggregate): add per-stage counts` and `docs(readme): describe per-stage counts`.
- [ ] The site is updated in a separate docs PR after the release, not here.
