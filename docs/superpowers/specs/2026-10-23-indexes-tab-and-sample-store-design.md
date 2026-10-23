# Indexes tab and usage sample store — Design

Date: 2026-10-23
Status: Draft, awaiting user review
Issue: #68 (`feat(indexes): list, create, hide and drop indexes with usage statistics`)

## Intent

The author runs MongoDB locally while developing and wants to find index problems before code reaches
staging. The Indexes tab answers two questions, in this order of importance:

1. **Is each index used, and how does its use change over time?** MongoDB only reports a cumulative
   counter (`$indexStats`) that resets on server restart and is per node. The app must keep its own
   history to show a trend.
2. **Can I create and change an index comfortably?** A good builder UI that shows the exact command
   before it runs.

The usage history is built on a general-purpose **sample store** (local SQLite) so later monitoring work
can reuse it: #59 live server statistics, #60 slow query profiler, and the follow-up "query trace" project
(see Non-goals).

## Goals

- An **Indexes** tab per collection: every index with definition, size, usage, last used, and a usage
  trend (layout B below).
- Background sampling of `$indexStats` while a connection is open, stored locally, so trends survive app
  restarts and server restarts.
- Create an index with a key builder and options; edit an index by creating a changed copy.
- Hide / unhide, drop with typed-name confirmation, copy definition as JSON.
- Respect read-only connections: create, drop and hide are refused through the existing read-only `Store`
  wrapper; viewing and sampling still work.
- Container-backed integration tests for each index type; accessibility tests pass; every user-facing
  string is an i18n key in `en.json`.

## Non-goals

- **Query trace** (detecting queries that miss an index or use a poor field order, from queries typed in
  the app and from the author's own apps via the profiler). This is a separate project, designed after
  this one. It builds on #60 (profiler), #69 (explain) and #72 (query history). This design only leaves a
  place for it in the tab.
- Server-wide statistics, current operations, profiler viewer (#59, #60).
- Atlas Search / Vector Search indexes (#78).
- Sampling while the app is closed.
- Cross-machine sync or export of history.

## Anti-goals

- Drawing a line across periods with no data. Gaps (app closed) and counter resets (server restart) must
  be visible as such, never as a false zero or a false drop.
- Silently changing the user's server. Creating, dropping and hiding happen only on explicit action, with
  the exact command shown first.
- A history file that grows without bound.

## Constraints

- Rust workspace: `curlybrace-core` (driver, connections, runtime), `curlybrace-ui` (egui),
  `curlybrace-app`, `curlybrace-testkit`. macOS only. See
  `docs/superpowers/specs/` for the earlier core-browser design.
- All database access goes through the `Store` trait (`core/src/store.rs`; real implementation in
  `mongo_store.rs`; read-only wrapper in `read_only_store.rs`; in-memory fake in `test_support`). Index
  operations are added to this seam.
- Existing local persistence is JSON files in the per-user config directory with atomic writes and 0600
  permissions (`ui/src/app/persistence.rs`). The history store follows the same location and permission
  rules.
- Project rules: no hardcoded user-facing strings (i18n keys, new keys reported); immutable data
  patterns; files small and focused; TDD with at least 80% coverage; prefer real domain objects over
  mocks in tests.

## Decisions made with the author

- **Sampling scope: every collection** in every user database on the connection, so history exists the
  first time any Indexes tab is opened. `admin`, `local` and `config` are skipped. The user can exclude
  further databases in a setting. No "pin to watch" feature.
- **Storage: SQLite** for the history (the author chose this over append-only JSON files).
- **Sampling runs in the background** while a connection is open, not only while the tab is visible.
- **Layout B** for the tab: index list on the left (name + usage count), detail on the right (definition,
  size, chart, last used, "tracked since", actions). Chart ranges: 24 hours / 7 days / 30 days.
- **Builder** as mocked: ordered key rows (field, type: ascending, descending, text, 2dsphere, wildcard),
  options (name, TTL, unique, sparse, hidden, collation, partial filter), live preview of the exact
  `createIndexes` command, warning on large collections, disabled with a message on read-only
  connections.
- **"Edit" means edit a copy.** MongoDB cannot change an index definition in place (only hide and TTL via
  `collMod`). The flow builds the new index first, then asks whether to drop the old one; the old index
  keeps serving queries until then.
- The usage chart marks **server restarts** (counter reset) and **no-data gaps** distinctly.

## Design

### Data shown per index
From `listIndexes` (definition and options), `$indexStats` (`accesses.ops`, `accesses.since`, per node),
and collection stats (size). The UI states that usage is per node and resets on restart.

### Sample store (`core`)
- A SQLite database in the app's config directory, 0600, one file for all connections, rows keyed by
  connection id, database, collection and index name.
- A sample row: timestamp, `ops` counter, `since` value. Usage over an interval is the difference between
  consecutive samples. If `since` changed or the counter went down, the server restarted: start a new
  segment instead of computing a negative delta.
- Retention with thinning: full-resolution samples for a short window, then hourly buckets, then daily
  buckets, with a hard size cap. Exact numbers are in "Left to the builder".
- A sampler task in the core runtime, started per open connection and stopped on disconnect.
- Gaps: if the time between two samples is much longer than the interval, the delta is shown as "uses
  during a period with no samples", not spread over time.
- Read access to history goes through a small repository interface so the UI and tests do not touch SQL.

### Index operations (`Store` trait)
`list_indexes` (with stats and size), `create_index`, `drop_index`, `set_index_hidden`. The read-only
wrapper refuses the three write operations. The in-memory fake implements them for UI tests.

### UI (`ui`)
New Indexes tab in the collection view. View-model first (immutable state, pure update functions), then
views. The builder validates before enabling Create: non-empty key list, no duplicate fields, valid JSON
for the partial filter, valid TTL (positive integer, single-field ascending/descending key). Drop requires
typing the index name. Large-collection warning above a document-count threshold.

## Testing
- Unit tests for delta/segment/gap computation and retention thinning (include property tests for the
  restart and gap cases).
- Container tests (`scripts/container-tests.sh`) for create, hide, drop and stats on each index type:
  single, compound, text, 2dsphere, wildcard, unique, TTL, partial, collation.
- UI tests with the in-memory store; accessibility tests for the new tab and builder; snapshot tests for
  the layout.

## Calls made by the design author (not discussed with the user; flagged for review)

- **Nodes.** `$indexStats` reports one row per node (`host`). Rows are stored with a `host` column from the
  start, so replica sets and sharded clusters need no later migration. The tab shows the node the
  connection reads from; a node picker is out of scope for the first version.
- **Index identity.** A series is identified by index name plus a hash of its definition (key and options, except
  `hidden` and the TTL, which change in place through `collMod` and leave the index and its history the same).
  Recreating a name with a different definition starts a new series; the trend never joins two different
  indexes. History of dropped indexes is kept until normal retention removes it and is not listed by
  default. A dropped or renamed collection is handled the same way: its rows age out.
- **Restart rule.** A sample belongs to a new segment when `since` changes or the counter decreases. The
  first sample of a segment has no delta; its counter value is shown as "uses since restart". A restart
  that happens during a gap is marked as both a gap and a restart.
- **Thinning.** Hourly and daily buckets store the sum of the deltas plus a restart count and the seconds
  covered by no data, so restarts and gaps stay visible in the 7 and 30 day views. A bucket may span a
  restart or a gap; the flags say so.
- **Sampler plumbing.** The sampler reads through a new read-only `Store` call (`index_stats`) that the
  read-only wrapper allows. If the call fails (privileges, lost connection), it logs, backs off, and the
  tab shows a "sampling paused" status with the reason as an i18n key. It never fails silently.
- **Edit a copy.** One guided flow with two writes: create the copy, then ask whether to drop the original.
  If the build fails nothing is dropped; if the user declines, both indexes remain. The default name for
  the copy gets a numeric suffix and is checked against existing names. Changing only the TTL or the
  hidden flag uses `collMod` directly instead of a copy.
- **Index spec type.** `list_indexes` returns a typed `IndexSpec` (keys, options, collation, partial filter,
  plus the raw definition). Key types the builder does not offer (hashed, 2d) are listed, shown, copied as
  JSON and preserved when editing a copy, but cannot be added from the builder.
- **Builds.** Create runs asynchronously in the core runtime; the UI stays responsive and shows "building".
  The large-collection warning is informational and requires confirming before the build. Cancel is
  deferred.
- **Cleanup.** Deleting a connection deletes its history rows. Each row also records the server identity
  (host) it came from, so a connection edited to point at another server starts a new series.
- **Tunables.** The sample interval and retention numbers below are constants in the first version, not
  user settings.

## Left to the builder
- SQLite crate and bundling choice (a bundled build is preferred so no system library is needed).
- Schema details, migrations, and indexes on the history tables.
- Chart rendering approach in egui.
- Exact sample interval, retention windows, size cap, large-collection threshold, and gap threshold. Starting
  point (author's agent's call, not agreed): sample every 60 seconds; keep raw samples 48 hours, hourly
  buckets 90 days, daily buckets 2 years; cap around 100 MB; warn above 1 million documents; call
  something a gap after 3 missed intervals.
- How the sampler discovers collections and databases as they appear and disappear, and how the
  excluded-databases setting is stored.
- Build progress and cancel for long index builds, where the server supports it (`currentOp` /
  `killOp`); acceptable to land after the first version.
- i18n key names and the exact wording of warnings.
