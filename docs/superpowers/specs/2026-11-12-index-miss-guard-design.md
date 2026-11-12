# Index-miss guard — Design

Date: 2026-11-12
Status: Draft, awaiting user review
Issue: #116 (the guard is the part of the original request that was never built; see "Background")

## Intent

The author found out about a query without an index in production, from the Atlas Query Profiler.
"A non-indexed query should not be allowed on production." Curlybrace cannot stop the author's own
applications, but it can stop its own: before the query bar or the Shell tab sends a query that the
server would answer with a full collection scan or a blocking in-memory sort, the app says so and
lets the author choose to run it anyway.

## Background (what already exists)

- The query bar already explains every typed query and shows a verdict badge
  (`crates/core/src/explain`, `Command::ExplainFind`, `crates/ui/src/app/explain_actions.rs`). Today the
  find and the explain are sent together and the verdict never delays or blocks the results.
- The Indexes tab already lists app queries from the profiler with their history (`crates/core/src/profile`,
  `crates/core/src/shape`, `crates/core/src/history`).
- A production-flagged connection already asks for confirmation before a Shell run
  (`WriteKind::ShellRun`, `crates/ui/src/app/write_guard.rs`) and before every write.
- The Shell tab already checks syntax before a run and shows a notice with **Run anyway**
  (`ShellNotice::SyntaxProblem`).

## Decisions made with the author

- The guard is a **warning with an override**, never a hard block.
- It applies to **every connection**, not only production-flagged ones.
- It covers **the query bar and the Shell tab**.
- For the Shell tab the check **reads the script text** and explains the queries it can recognise;
  a query it cannot read is shown as **not checked**. No dry run through `mongosh`.

## Goals

- Query bar: before a Run sends the find, ask the server to explain it. If the verdict is a full
  collection scan or a sort not covered by an index, do not send the find yet: show a notice
  naming the problem (the existing verdict reasons and suggested index) with **Run anyway** and
  **Cancel**. Any other verdict, or an explain that is unavailable, sends the find as today.
- Shell tab: before a Run, read the script text, find the plain collection calls whose filter is
  written out literally, explain each, and show one notice listing the ones that would scan or sort
  without an index, plus how many calls could not be checked. **Run anyway** sends the script and
  keeps every other guard (read-only, mongosh checks, the production confirmation).
- The guard's explain is **plan-only** (`queryPlanner`): it does not run the query. The existing
  badge explain uses `executionStats`, which executes the query for up to 2 s (`EXPLAIN_MAX_TIME_MS`),
  so it must not be the guard's check. After the find is sent the badge works as today.
- The check never changes data and works on read-only connections (explain is a read).

## Non-goals

- Blocking queries sent by other applications.
- Aggregation pipelines (the query bar has none; #67).
- A dry run of a script through `mongosh`.
- Checking filters built from variables, loops or function calls.
- A per-collection or per-connection exemption list.

## Anti-goals

- A guard that fails closed: when the explain is unavailable, times out or errors, the query runs
  (the badge says "unavailable" as today). The guard must never become the reason a query cannot run.
- Nagging on development data: a collection too small for an index to matter must not warn on
  every query.
- Any change to what is stored: nothing new is persisted except one setting, and nothing is added to
  history.

## Behavior

### Which verdicts warn

Warn on `CollectionScan` and `SortNotCovered`. Do not warn on `IndexUsed`, `CoveredByIndex`,
`NoFilter`, `EmptyCollection`, or an unavailable verdict. `InefficientIndex` is judged from execution
statistics, which a plan-only explain does not have, so it never warns; the badge still shows it
after the run, as today.

### Small collections (author's call to confirm: my default)

On a production-flagged connection the guard warns regardless of size. On any other connection it
warns only when the collection holds at least **1,000 documents**; below that there is no notice.
The count comes from `DocumentStore::estimated_count` (a metadata read, one call per collection per
Run, no caching); if it fails, warn. The threshold is a named constant. It applies to the query bar
and to every Shell call alike.

### Settings

One global switch "Warn before queries that would scan a collection", on by default, saved in
`explain.json` beside the existing `automatic` field (same atomic 0600 write and recovery rules).
When it is off, both the query bar and the Shell tab behave exactly as today. The existing "Check index use
automatically" switch keeps controlling the badge; when the guard is on, the guard's explain runs
even if the badge switch is off, and the badge is shown for that run.

### Query bar flow

1. Run with the guard on: send a plan-only explain first, do not send the find. "A Run" means
   `FindAction::Run` only (the Run button and Enter in the filter). Paging, page size, refresh, sort
   clicks and opening a collection send their finds as today and are not guarded; **Run anyway** is
   not remembered.
2. Verdict warns: show the notice (reasons, suggested index, copy button as in the badge), keep the
   previous page on screen. **Run anyway** sends the find once, without explaining again.
   **Cancel**, a new Run, a collection change or a reconnect drops the notice.
3. Verdict does not warn, or the explain fails: send the find and show the verdict as today.
4. The guard's explain has the existing 2 s max-time bound, so Run is delayed by at most that.

### Shell tab flow

1. Run (selection if any, else the whole script), after the syntax check and the existing
   read-only and tool checks.
2. Read the text for these calls on a collection of the current database:
   `find`, `findOne`, `countDocuments`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`,
   with a literal filter object (and a literal `sort` chained after `find`). The filter is parsed
   as a literal only: keys (bare identifiers or quoted), single- or double-quoted strings, numbers,
   booleans, null, nested objects and arrays, `ObjectId("<24 hex>")` and `ISODate("<ISO 8601>")`.
   Anything else is "not checked". (The query bar accepts only strict JSON, so the Shell reader
   needs its own small literal parser.)
3. Explain each readable call one after another (at most 20 per run; the rest count as "not
   checked"), with an overall cap of 10 s; calls not reached by then, and any explain that fails or
   times out, count as "not checked". The Shell tab shows a "checking queries" state with Cancel. A write call is
   explained as a `find` with the same filter; the write itself is never sent.
4. If any call warns, show one notice listing each (collection, call, problem) and the not-checked
   count, with **Run anyway** and **Cancel**. Otherwise run as today. A script with no readable
   call runs as today with no notice.
5. On a production connection the production confirmation still follows **Run anyway**.
6. The database of a call is the one the script runs against at that point (the builder finds how the
   Shell tab picks its starting database; a selection-only run starts from that same database); a `use x` line or
   `db.getSiblingDB("x")` changes it only when `x` is a string literal, otherwise the later calls
   are "not checked".

## Constraints

- No new dependency; no change to history, secrets or `history.sqlite`.
- CI-pinned Rust 1.87.0: no let-chains.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json`; new keys are reported.
- Accessibility tests for the new notices; snapshot baselines for the two notices.
- Tests use the fake mongosh and a throwaway `mongod`; never a MongoDB already on the machine.

## Left to the builder

- Where the Shell text reader lives (suggested: `crates/ui/src/viewmodel/shell_queries.rs`, next to
  `shell_check.rs` and `shell_scan.rs`, reusing the code/non-code scan).
- Whether the literal parser reuses the query bar's filter parser.
- The exact runtime command for a batch of explains in the Shell flow.

## Calls made by the design author (flagged for review)

- 1,000-document threshold for non-production connections; always warn on production.
- Fail-open when the explain is unavailable.
- `InefficientIndex` does not warn.
- 20 explains per Shell run, sequential, 10 s overall.
- Paging and refresh are not guarded; only `FindAction::Run` is.
- Writes (`updateMany`, `deleteMany`, ...) are explained as finds.
- Guard switch lives in `explain.json`; explain runs for the guard even when the badge switch is off.
