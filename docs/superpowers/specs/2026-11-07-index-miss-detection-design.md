# Index-miss detection — Design

Date: 2026-11-07
Status: Draft, awaiting user review
Issue: #116 (`feat(indexes): find queries that miss an index, from apps and from the query bar`)

## Intent

The author runs MongoDB locally while developing and wants to catch index problems before code reaches
staging. The most common problem is a query that does not use an index, or uses one badly (wrong field
order, sort not covered). Today the Indexes tab shows whether an index is used, but not which queries miss
one. Finding out means reading the server log, which the author described as cumbersome.

The last time this came up, the author had just typed a query in the query bar and wanted to know whether
it hit an index. The answer should be next to the results, not in a log file.

Two sources of queries, in this order of importance:

1. **Queries typed in the query bar.** An automatic verdict on every run.
2. **Queries the author's own apps send** to the local MongoDB, read from the server profiler, grouped by
   shape and kept over time next to the index usage history.

## Release plan

This ships in two releases, both under #116, each its own branch and PR:

- **Release A: the query-bar verdict.** Explain on every typed query, a verdict badge, a suggested index.
  No profiler, no new storage, works on read-only connections.
- **Release B: app queries and history.** Profiler enable/read, query shapes, the "Queries" section of the
  Indexes tab, and storage in `history.sqlite`.

Release A must be useful on its own and must not depend on anything in B.

## Goals

- After each query run in the query bar, show a verdict for how the server executed it:
  - **index used** (good);
  - **full collection scan**;
  - **sort not covered by the index** (blocking in-memory sort);
  - **index used, but inefficient** (many more keys or documents examined than returned, which is how a
    poor field order shows up);
  - **not applicable / unavailable** (views, errors, timeouts) with the reason.
- For every non-good verdict, say why in plain words and suggest a key order for an index that would fix it.
- (B) List queries that apps send, grouped by shape, with run count, slowest time, and verdict, misses first.
- (B) Keep verdict history per shape so a bottleneck is visible over 24 hours, 7 days and 30 days, using the
  same retention and size-cap approach as the index usage history.
- Read-only connections keep working: explain and profiler reads are reads. Turning the profiler on is a
  write and is refused on read-only connections.
- Every user-facing string is an i18n key in `en.json`; accessibility tests pass.

## Non-goals

- Aggregation pipelines (no pipeline builder exists yet, #67).
- The visual explain plan (#69), the slow query viewer (#60), query history and saved queries (#72). This
  design does not depend on them; it only reads the same server facts. When #60 is built it should reuse
  the profiler read path from release B.
- Creating the suggested index with one click. Release A shows the suggestion as text with a copy button.
  Opening the index builder prefilled is a follow-up.
- Live alerts or notifications while an app runs. The Queries section is a list the author opens.
- Profiling through `mongos` on sharded clusters (the profiler is per `mongod`).
- Cross-machine sync of history.

## Anti-goals

- Turning the profiler on, or changing anything on the server, without an explicit action showing the exact
  command first.
- Delaying or blocking the query results while the verdict is computed.
- False alarms. A query with an empty filter and no sort reads everything by definition; it gets a neutral
  "no filter, reads everything" note, not a red miss.
- Storing the author's data. Stored shapes keep field names and operators, never literal values.
- Silent failure. When a verdict or the profiler cannot be read, the UI says so and gives the reason.
- Counting the app's own traffic as an app query (see "Profiler" below).

## Constraints

- Rust workspace: `curlybrace-core` (driver, connections, runtime), `curlybrace-ui` (egui),
  `curlybrace-app`, `curlybrace-testkit`. macOS only.
- All database access goes through the `DocumentStore` trait (`crates/core/src/store.rs`; real
  implementation `mongo_store.rs`; read-only wrapper `read_only_store.rs`; in-memory fake in
  `test_support`). The read-only wrapper forwards every method by hand: a new method that is not added
  there silently falls back to the trait default, so each new method needs a wrapper entry and a test.
- History lives in `history.sqlite` (`crates/core/src/history`), schema version 1, with a 100 MB cap and
  oldest-first thinning. New tables arrive through a v2 migration.
- Project rules: no hardcoded user-facing strings (new keys in `crates/ui/locales/en.json`, nested,
  alphabetical, reported at the end); immutable data; files under about 800 lines and functions under
  about 50; TDD with at least 80% coverage; real BSON values in tests over mocks.
- Real-MongoDB tests are opt-in (`CURLYBRACE_IT=1`) and new files must be added by hand to the lists in
  `scripts/integration-tests.sh` / `scripts/container-tests.sh`.

## Decisions made with the author

- **Typed queries get an automatic verdict on every run.** Not on demand.
- **Typed-query verdicts need no profiler or server setup.** They come from an explain run alongside the query.
- **Split into two releases** as above.
- **App queries are shown as a list you open** (grouped, misses first), not as live warnings.
- **The profiler is never enabled silently.** An explicit button, with the exact command and an explanation.
- **History is kept over time** next to index usage, using the existing store.

## Design

### Verdict (core, release A)

A pure function `classify(explain: &Document) -> Verdict` in a new `crates/core/src/explain/` module.
Input is the server's explain output; output is a typed `Verdict` (kind, index name if any, keys examined,
docs examined, documents returned, a list of reasons as i18n key + params, and a `SuggestedIndex` key
list). Being pure, it is tested against explain documents captured from a real server.

Judging a miss, from the winning plan:

- **Full collection scan:** a `COLLSCAN` stage in the winning plan. Exception: empty filter and no sort,
  which is the neutral "reads everything" note.
- **Sort not covered:** a `SORT` stage (blocking sort) above the scan or index stage.
- **Inefficient index use:** the plan uses an `IXSCAN`, but keys or docs examined exceed documents returned
  by a threshold ratio (starting point in "Left to the builder"). This is the signal for a wrong field
  order, because the planner picked the index but could not narrow with its prefix.
- **Good:** an `IXSCAN` with examined counts close to returned, and no blocking sort. A covered query
  (`PROJECTION_COVERED`) is noted as such.

Suggested index: apply the equality, sort, range rule to the filter and sort of the query. Equality fields
first, then sort fields, then range fields. The suggestion is a key list and a ready-to-copy
`createIndex` command; it is advice and is never run.

### Explain call (`DocumentStore`, release A)

A new default-bodied method `explain_find(ns, query) -> Document` (the raw explain), implemented in
`MongoStore` by wrapping the same find that the query bar sends in `{explain: <find>, verbosity:
"executionStats", maxTimeMS: ...}`. There is a ready pattern in `crates/core/tests/local_compare_store.rs`.
It is a read, so `ReadOnlyStore` forwards it. The in-memory fake returns canned explain documents.

What the explain mirrors: the query bar today sends filter, sort, projection, skip and limit (no `hint` or
collation yet; #75 adds them). Explain is built from the same `Query`. When #75 lands, explain must pass
the hint and collation too, and a query with a user-supplied `hint` is not flagged as a miss for the index
it was told to use (follow-up, noted here so it is not forgotten).

Edge cases for `classify`: an empty filter with no sort is the neutral note; an empty filter *with* a sort
the index does not cover is a real "sort not covered"; an `_id`-only filter uses the `_id_` index and is
simply good.

### Runtime and UI wiring (release A)

- **When explain runs:** once per Run action in the query bar. Paging ("load more") re-uses the verdict of
  the run it belongs to and does not explain again; changing the filter, sort or limit is a new run. No
  cache.

- A new `Command::ExplainFind` / `Event::Explain` pair, correlated to the run by the same session and query
  identity the `Results` event uses. The command is issued right after `Command::Find` is issued, but runs
  independently; results render first.
- A newer run cancels or supersedes an older explain; a stale `Event::Explain` is dropped, never shown
  against newer results.
- The badge sits with the results header: icon plus text (not colour alone), a "why" expandable line, and
  the suggestion with a copy button. A pending verdict shows a neutral "checking" state.
- A setting turns the automatic verdict off, default on.

### Profiler (core, release B)

A profile entry already carries what the verdict needs (`planSummary`, `keysExamined`, `docsExamined`,
`nreturned`, `hasSortStage`, `millis`, `command`, `appName`, `ns`, `ts`), so app queries are classified
from the entry without running explain again.

- **Reading:** a new read method on the store reads `system.profile` for a database since a timestamp.
  `ReadOnlyStore` forwards it. Missing privileges or a missing profile collection produce a typed error
  shown as "profiler unreadable" with the reason.
- **Enabling (write):** a new `set_profiling_level` method. `ReadOnlyStore` refuses it. The UI shows the
  exact command first. The author's choice of level is not asked; the call below picks level 2 for the
  chosen database so fast misses are captured on a dev server.
- **Not enabled silently, not disabled silently:** the section shows current state (off / level / which
  database) and a "turn off" button. The app never changes the level on its own, including on exit.
- **Which database:** the profiler is per database. The section lives in a collection's Indexes tab and
  acts on that collection's database. Watching several databases means enabling each from its own tab; a
  database is a first-class scope in the stored data (it already is in the series key).
- **Own traffic excluded:** the app does not set `appName` today (only a user's own URI can). Release B
  makes the app set `appName=curlybrace` when the URI has none, and append ` curlybrace` to a URI-supplied
  value (e.g. `myapp curlybrace`), so its own entries are recognisable by that suffix. Entries with that
  marker, entries for `system.profile`, and the sampler's `$indexStats` aggregations are excluded from
  the app-queries list. Entries from the author's own apps are never excluded.
- **Polling:** a background reader polls on the same cadence as the index sampler for each connection with
  the profiler on, remembering the last `ts` per database. The profile collection is capped, so under heavy
  load entries can roll off between polls; when the oldest unread entry is newer than the last seen `ts`,
  the gap is recorded and shown as "entries may have been missed", never hidden.

### Query shapes (core, release B)

A pure `normalize(command) -> Shape` replaces literal values with placeholders and keeps field names,
operators, sort keys and projection keys, so `{userId: 5}` and `{userId: 9}` are one shape. A shape has a
stable hash. Literal values are never stored.

### History tables (release B)

A v2 migration adds tables for shapes and their verdict history, with a `source` column (`app` for
profiler, `typed` for the query bar) keyed by connection, host, database, collection and shape hash.
Per bucket: executions, misses, total and max millis, worst verdict. Retention follows the existing scheme
(raw 48 hours, hourly 90 days, daily 2 years) and shares the one size cap with the usage samples; when the
cap is hit, oldest rows go first across both. Deleting a connection deletes its shape rows. Read-only
connections write only to the local SQLite file, never to the server.

Typed-query verdicts from release A start being recorded here in release B. Release A's `Verdict` and
`Event::Explain` do not carry a shape; in B the shape is computed from the run's `Query` at the point of
recording, so `normalize` lives entirely in B and A stays independent. Recording typed verdicts does not
need the profiler.

The v2 migration only adds tables; it does not alter the v1 tables. Under the shared cap, usage samples
and shape rows are thinned oldest-first with equal priority. Deleting a connection deletes its rows from
both sets, including `source = typed`.

### UI (release B)

A "Queries" section in the collection's Indexes tab: shapes for that collection, misses first, each with
run count, slowest time, last verdict and the suggestion. A trend chart of misses over the existing
24 hour / 7 day / 30 day ranges, with no-data gaps drawn as gaps. The profiler enable/disable control lives
here, per database, with the consent step.

## Testing

- **Unit (offline):** `classify` against real explain documents for each verdict (collection scan, index
  used, blocking sort, inefficient index, covered, empty filter, view unsupported); suggestion ordering;
  `normalize` (same shape for different values, different shape for different operators); history tables,
  retention and shared cap; poll gap detection.
- **Stores:** the in-memory fake covers explain and profile reads; the read-only wrapper has tests that
  explain and profile reads pass and `set_profiling_level` is refused.
- **Integration (real mongod, opt-in):** each verdict against a real server; profiler enable, read, own
  traffic excluded, disable; read-only refusal. Added to the script lists by hand.
- **UI:** viewmodel tests for pending / stale / each verdict; accessibility tests for the badge and the
  Queries section; snapshot tests for the layout; `i18n_usage_tests` prefixes updated.

## Calls made by the design author (not discussed with the user; flagged for review)

- **Explain verbosity.** Explain runs at `executionStats`, with a `maxTimeMS`, because the examined-versus-
  returned ratio is the only reliable evidence of a poor field order. It executes the query a second time.
  If it times out, fall back to `queryPlanner` and mark the verdict "from the plan only".
- **Small collections are not excused.** A collection scan is flagged regardless of size, since the point
  is to catch it while the dev data is still small.
- **Profiler level 2.** Enabling uses level 2 on the chosen database so that fast misses are seen. It adds
  overhead on that database; the consent text says so.
- **Profiler scope.** Enable and disable are per database; the profile collection's size is not changed.
- **Pipelines and sharded clusters** are out of scope, and a `mongos` connection shows "profiler not
  available through mongos".
- **Own traffic** is excluded by `appName`, which means setting a distinct `appName` on connections.
- **Suggestion is advice only** in this issue.
- **Opt-out setting** for the automatic verdict exists even though the default is on. It is a single
  global setting (not per connection).
- **`appName` marker** (set when absent, suffixed when present) is how own traffic is recognised.
- **Explain once per Run,** not per page; no cache.
- **Typed queries recorded in B** under `source = typed`.

## Left to the builder

- Exact thresholds: the keys-examined-to-returned ratio and minimum counts that mean "inefficient", the
  explain `maxTimeMS` (starting point 2000 ms), and the poll interval (start with the sampler's).
- Minimum MongoDB server versions to support for explain and profile fields, checked against the versions
  the container tests use.
- Schema details and indexes for the v2 tables; how the shared size cap is applied across both table sets.
- How shapes are stored (JSON text, canonical form) and hashed.
- i18n key names and wording (suggested prefixes: `query.verdict.*`, `indexes.queries.*`; add them to
  `PREFIXES` in `i18n_usage_tests.rs`).
- Badge placement and the chart reuse in egui.
