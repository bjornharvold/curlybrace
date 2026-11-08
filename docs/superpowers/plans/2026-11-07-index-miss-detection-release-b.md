# Index-miss detection, release B (app queries and history) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the queries the author's own apps send to the local MongoDB, grouped by shape with a verdict per shape, from the server profiler, and keep that history (with the typed-query verdicts from release A) in `history.sqlite` next to the index usage history.

**Architecture:** A pure shape normaliser and a pure profile-entry classifier in `curlybrace-core`; three new `DocumentStore` methods (read profile, read profiling level, set profiling level — the last a write refused on read-only connections); a v2 history migration with shape tables sharing the 100 MB cap; a background profile poller per session beside the existing index sampler; a "Queries" view in the Indexes tab with an explicit consent step before the profiler is enabled.

**Tech Stack:** Rust workspace (`curlybrace-core`, `curlybrace-ui` egui, `curlybrace-testkit`), `mongodb` driver, `rusqlite` (existing), `sha2` (existing), `en.json` i18n.

**Spec:** `docs/superpowers/specs/2026-11-07-index-miss-detection-design.md` (release B sections: Profiler, Query shapes, History tables, UI (release B)). Release A is in PR #127; this branch (`feat/116-index-miss-detection-b`) is based on it and is rebased onto `main` once #127 merges.

## Global Constraints

- No hardcoded user-facing strings: every label, reason, tooltip, accessible label and the consent text is a key in `crates/ui/locales/en.json` (nested, alphabetical), new keys under `indexes.queries.*`; report the new keys. Core returns i18n key + params, never English.
- Literal query values are NEVER stored, logged or put into events: shapes keep field names, operators, sort keys and projection keys only. Log variant names, never message text.
- The profiler is never enabled or disabled without an explicit user action that showed the exact command first; the app never changes the level on its own, including on exit. `set_profiling_level` is a write: refused by `ReadOnlyStore` (`CoreError::ReadOnly`), covered by the UI write guard (`WriteKind::SetProfilingLevel`), and listed in `Command::is_write`.
- Profiler level 2 on the chosen database only; the capped `system.profile` size is not changed.
- Own traffic is excluded from app queries: entries whose `appName` ends with the marker `curlybrace`, entries for `system.profile`, and the sampler's `$indexStats` aggregations.
- Read-only connections still read the profiler (reads) and write only to the local SQLite file.
- v2 history migration only adds tables, never alters v1 tables. Retention follows the existing scheme (raw 48 hours, hourly 90 days, daily 2 years) and shares the one 100 MB cap with usage samples, oldest rows first with equal priority; deleting a connection deletes its shape rows (including `source = typed`); writes honour `is_purged`.
- Immutable data patterns; files under about 800 lines and functions under about 50 (`runtime/mod.rs` is already 808 lines and `memory_store.rs` 841: do not grow them; extract as Task 6 says and put new code in new files); TDD; at least 80% coverage on new core code; real BSON in tests, not mocks.
- Real-MongoDB tests are opt-in (`CURLYBRACE_IT=1`) and new files are added by hand to `scripts/integration-tests.sh` / `scripts/container-tests.sh`.
- Verify with `cargo test --workspace`, `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (no new warnings).
- Commits use conventional format with the `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` trailer; never `git add -A` (unrelated `site/` working-tree changes exist) and never commit `.superpowers/`.

## Review Focus

1. **No literal values leak.** Profile `command` documents hold real values; shapes, logs, events and history rows must contain none. (Tasks 1, 3, 7)
2. **Consent is real.** Nothing calls `set_profiling_level` except the explicit confirm action; read-only and mongos/unauthorised/Atlas-shared cases produce a clear reason, never a silent no-op. (Tasks 4, 6, 8, 9)
3. **Own traffic and noise.** The app's own explain/find/sampler traffic and `system.profile` reads must not appear as app queries, including when the URI already had an `appName`. (Tasks 2, 5, 7, 9)
4. **Capped-collection gaps.** Entries rolling off the 1 MB `system.profile` between polls must be recorded and shown as "may have missed entries", never hidden; a profiler turned off or a restarted server must not produce negative or duplicated counts (dedupe by `ts` watermark). (Tasks 7, 9)
5. **Shared cap and purge.** With both table sets full, the cap thins oldest-first without starving either; deleting a connection leaves no shape rows and a late poll cannot resurrect them. (Task 3)

---

### Task 1: Query shapes (`core::shape`)

**Files:**
- Create: `crates/core/src/shape/mod.rs`, `crates/core/src/shape/normalize.rs`, `crates/core/src/shape/suggest.rs`
- Test: `crates/core/src/shape/normalize_tests.rs`, `crates/core/src/shape/suggest_tests.rs`
- Modify: `crates/core/src/lib.rs` (`pub mod shape;`)

**Interfaces:**
- Consumes: `store::Query`, `explain::SuggestedIndex` (release A).
- Produces:
  - `pub struct Shape { /* canonical form, private */ }` with `pub fn json(&self) -> String` (stable canonical JSON, no literal values), `pub fn hash(&self) -> String` (64-char hex SHA-256 of `json()`), `pub fn from_json(json: &str) -> Result<Shape, CoreError>`, `Debug + Clone + PartialEq`
  - `pub fn from_query(q: &Query) -> Shape` (filter, sort, projection keys; `skip`/`limit` ignored)
  - `pub fn from_profile_command(op: &str, command: &Document) -> Option<Shape>` (`None` for ops without a filter or unsupported)
  - `pub fn suggest_for_shape(shape: &Shape) -> Option<SuggestedIndex>` (same equality, sort, range rule as release A's `suggest`, derived from operators in the shape)

- [ ] **Step 1: Write failing tests** `normalize_tests.rs`: `same_shape_for_different_values`, `different_operator_is_a_different_shape`, `field_order_in_filter_is_irrelevant`, `sort_order_is_significant`, `in_arrays_ignore_arity_and_values`, `or_children_are_order_insensitive`, `regex_pattern_is_masked`, `nested_and_elemmatch_masked`, `no_literal_value_survives` (property-style: build filters with distinctive literals like `"SECRET-7f3a"` and assert the string never appears in `json()`), `hash_is_stable_across_runs` (pin a literal expected hash for one shape), `profile_find_aggregate_count_distinct_update_delete_findandmodify_commands_map`, `profile_getmore_and_unsupported_ops_are_none`; `suggest_tests.rs`: `suggest_for_shape_matches_release_a_rule`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core shape` → FAIL.
- [ ] **Step 3: Implement `normalize`:** every scalar literal becomes the placeholder `"?"`; arrays under `$in`/`$nin` become `["?"]`; `$regex` pattern/options masked; keys and operators kept; filter keys sorted; `$and/$or/$nor` children sorted by their canonical string; sort document order kept (with directions); projection keeps only field names. `from_profile_command` reads the filter from the right place per op (`find.filter`, `count.query`, `distinct.query`, `aggregate` leading `$match` + `$sort`, `update` `q`, `delete` `q`, `findAndModify.query`).
- [ ] **Step 4: Implement `suggest_for_shape`:** reuse release A's field classification (extract a shared helper from `explain/suggest.rs` if needed without changing its behaviour; its tests must stay green).
- [ ] **Step 5: Run** `cargo test -p curlybrace-core shape explain` → PASS; clippy clean.
- [ ] **Step 6: Commit** `feat(shape): normalise queries into value-free shapes with a stable hash`

### Task 2: Profile entries and their verdict (`core::profile`)

**Files:**
- Create: `crates/core/src/profile/mod.rs`, `crates/core/src/profile/entry.rs`, `crates/core/src/profile/classify.rs`
- Test: `crates/core/src/profile/entry_tests.rs`, `crates/core/src/profile/classify_tests.rs`, fixtures `crates/core/src/profile/fixtures/*.json` (hand-written realistic `system.profile` documents: collscan, ixscan, sort, inefficient, own-traffic, getmore, system.profile read, aggregate; Task 9 validates them against a real server)
- Modify: `crates/core/src/lib.rs`, `crates/core/src/explain/verdict.rs` (add `VerdictKind::severity(self) -> u8`, higher = worse, and `Ord`-free helper `worst(a, b)`)

**Interfaces:**
- Produces:
  - `pub const OWN_APP_NAME: &str = "curlybrace";`
  - `pub struct ProfileEntry { pub ts_secs: i64, pub ns: String, pub op: String, pub millis: u64, pub plan_summary: String, pub keys_examined: u64, pub docs_examined: u64, pub returned: u64, pub has_sort_stage: bool, pub app_name: Option<String>, pub command: Document }`
  - `pub fn parse_entry(doc: &Document) -> Result<ProfileEntry, CoreError>` (tolerates Int32/Int64/Double numbers, missing optional fields; `CoreError::Database` for a document without `ts`/`ns`)
  - `pub fn is_own_traffic(entry: &ProfileEntry) -> bool` (appName ends with `OWN_APP_NAME` on a word boundary, or `ns` ends with `.system.profile`, or the command is `aggregate` with a `$indexStats` stage)
  - `pub fn classify_entry(entry: &ProfileEntry, has_filter: bool) -> VerdictKind` (COLLSCAN plan summary → `CollectionScan` unless no filter and no sort → `NoFilter`; `hasSortStage` → `SortNotCovered`; IXSCAN with examined ≥ 100 and ≥ 10× max(returned,1) → `InefficientIndex` using the SAME named constants as release A's `classify` (extract to `explain` if they are private); else `IndexUsed`)

- [ ] **Step 1: Write failing tests** for each fixture's `parse_entry` + `classify_entry` + `is_own_traffic` (`entry_tests.rs`: `parses_numbers_of_every_width`, `missing_ts_is_an_error`; `classify_tests.rs`: `collscan_is_collection_scan`, `empty_filter_collscan_is_no_filter`, `sort_stage_is_sort_not_covered`, `ixscan_with_many_keys_is_inefficient`, `good_ixscan_is_index_used`, `own_traffic_by_app_name_suffix`, `own_traffic_with_user_app_name_suffix`, `system_profile_reads_are_own_traffic`, `index_stats_aggregation_is_own_traffic`, `users_own_app_is_not_own_traffic`), and `severity_orders_kinds` in the explain tests.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core profile` → FAIL.
- [ ] **Step 3: Implement** per the signatures above; share the inefficiency constants with release A rather than duplicating them.
- [ ] **Step 4: Run** `cargo test -p curlybrace-core` → PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(profile): parse profile entries, classify them and recognise our own traffic`

### Task 3: History v2 — shape tables, recording, reading, shared cap, purge

**Files:**
- Create: `crates/core/src/history/shape_schema.rs`, `crates/core/src/history/shapes.rs` (record/read API), `crates/core/src/history/shape_thin.rs` (rolling and cap steps)
- Modify: `crates/core/src/history/db_schema.rs` (`SCHEMA_VERSION = 2`, `V2` const with only `CREATE TABLE`/`CREATE INDEX`, `if version < 2 { migrate_to_v2 }` — no repeat of the `auto_vacuum` pragma, sets `user_version` in its own transaction), `history/db.rs` (`purge_connection` also deletes shape rows in the same locked section), `history/thin.rs` (`thin` rolls shape buckets; `enforce_cap` gains shape steps), `history/mod.rs`
- Test: `crates/core/src/history/shapes_tests.rs`, `shape_thin_tests.rs`, `db_schema_v2_tests.rs` (new files; `db_tests.rs` is at 776 lines)

**Interfaces:**
- Consumes: `shape::Shape` (Task 1 hash/json), `explain::VerdictKind` + `severity` (Task 2), `ChartRange`, the existing `HistoryDb` error type.
- Produces:
  - `pub enum ShapeSource { App, Typed }`
  - `pub struct ShapeKey { pub connection: Uuid, pub host: String, pub db: String, pub collection: String, pub hash: String, pub source: ShapeSource }`
  - `pub struct ShapeObservation { pub key: ShapeKey, pub shape_json: String, pub at_secs: i64, pub executions: u64, pub misses: u64, pub total_millis: u64, pub max_millis: u64, pub worst: VerdictKind, pub gap_secs: u64 }`
  - `pub struct ShapeBucket { pub start_secs: i64, pub width_secs: i64, pub executions: u64, pub misses: u64, pub gap_secs: u64 }`
  - `pub struct ShapeSummary { pub hash: String, pub shape_json: String, pub source: ShapeSource, pub executions: u64, pub misses: u64, pub total_millis: u64, pub max_millis: u64, pub worst: VerdictKind, pub buckets: Vec<ShapeBucket> }`
  - `HistoryDb::record_shapes(&self, rows: &[ShapeObservation]) -> Result<…>` (one transaction; skips purged connections under the connection lock; additive upsert per 10-minute raw bucket: sums for counts/millis, max for `max_millis`, severity-max for `worst`)
  - `HistoryDb::shapes_for(&self, connection: Uuid, db: &str, collection: &str, range: ChartRange, now_secs: i64) -> Result<Vec<ShapeSummary>>`
  - `HistoryDb::record_gap(&self, connection: Uuid, host: &str, db: &str, at_secs: i64, gap_secs: u64) -> Result<()>` (marks entries-may-have-been-missed time on all shapes of that db for the chart's gap drawing; implementer may model as a per-db gap table)

- [ ] **Step 1: Write failing tests** (all in-memory db): `v1_file_migrates_to_v2_without_touching_v1_rows`; `newer_than_v2_is_refused_as_before`; `observations_aggregate_into_one_bucket`; `same_shape_different_source_are_separate_rows`; `literal_values_are_not_stored` (observation built from a Task 1 shape of a filter with a distinctive literal; dump every text column of every table and assert the literal is absent); `purged_connection_is_not_resurrected_by_a_late_record`; `purge_connection_deletes_shape_rows_including_typed`; `thin_rolls_raw_into_hourly_and_daily_preserving_sums_max_and_worst`; `retention_expires_old_rows`; `cap_thins_oldest_first_across_usage_and_shapes` (fill both table sets past `with_max_bytes`, assert neither set is emptied while the other keeps its oldest rows, and the file ends under the cap); `shapes_for_respects_range_and_reports_gaps`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core history` → FAIL.
- [ ] **Step 3: Implement the migration and tables:** `shape(id, connection, host, db, collection, hash, source, shape_json, UNIQUE(connection, host, db, collection, hash, source))`, `shape_bucket(shape_id → shape ON DELETE CASCADE, tier, start_secs, executions, misses, total_millis, max_millis, worst, gap_secs, PRIMARY KEY(shape_id, tier, start_secs))` with tier 0 = raw 10-minute buckets (48 h), 1 = hourly (90 d), 2 = daily (2 y), plus an index for `(connection, db, collection)` lookups.
- [ ] **Step 4: Implement record/read/roll/cap** using `GROUP BY` rolling (counts and millis summed, max of max, max severity of worst, gap summed). In `enforce_cap`, alternate the usage and shape deletes at each existing step (future-only raw, oldest daily, oldest hourly, oldest raw), 500 rows at a time, re-checking `used_bytes` after each batch so neither set starves.
- [ ] **Step 5: Run** `cargo test -p curlybrace-core history` → PASS; clippy clean.
- [ ] **Step 6: Commit** `feat(history): v2 migration with query shape tables sharing the size cap`

### Task 4: Profiler store methods

**Files:**
- Create: `crates/core/src/mongo_profile.rs`, `crates/core/src/test_support/memory_profile.rs`
- Modify: `crates/core/src/store.rs` (types + three default-bodied methods returning `CoreError::Unsupported`), `crates/core/src/mongo_store.rs` (delegate only), `crates/core/src/read_only_store.rs` (forward the two reads; `set_profiling_level` calls `self.guard()?`), `crates/core/src/test_support/memory_store.rs` (three delegating methods only — the file is over 800 lines), `crates/core/src/lib.rs`
- Test: `crates/core/src/mongo_profile_tests.rs`, `crates/core/src/read_only_store_tests.rs`, `crates/core/src/test_support/memory_profile.rs` (inline)

**Interfaces:**
- Consumes: `profile::{ProfileEntry, parse_entry}` (Task 2).
- Produces:
  - `pub struct ProfilingState { pub level: i32, pub slow_ms: i64 }`
  - `pub struct ProfileRead { pub entries: Vec<ProfileEntry>, pub oldest_ts_secs: Option<i64> }` (`oldest_ts_secs` is the oldest entry currently in `system.profile`, for roll-off detection)
  - `async fn profiling_level(&self, db: &str) -> Result<ProfilingState>` (command `{profile: -1}`)
  - `async fn read_profile(&self, db: &str, since_ts_secs: i64) -> Result<ProfileRead>` (`system.profile` entries with `ts` strictly greater than `since`, sorted by `ts`, at most 1000 per call, unparseable entries skipped and counted in a `log::warn!` of the count only)
  - `async fn set_profiling_level(&self, db: &str, level: i32) -> Result<ProfilingState>` (command `{profile: level}`; `level` must be 0, 1 or 2 else `CoreError::Unsupported`; returns the new state)
  - `InMemoryStore::with_profile(self, db: &str, entries: Vec<ProfileEntry>) -> Self`, `with_profiling_level(self, db: &str, level: i32) -> Self`, `with_profile_error(self, db: &str, error: CoreError) -> Self`

- [ ] **Step 1: Write failing tests:** pure builder tests for the three commands (`profile_level_command_is_minus_one`, `set_level_command_carries_level`, `since_filter_uses_ts_gt_and_sorts_by_ts`, `level_out_of_range_is_rejected`); `read_only_store_forwards_the_profile_reads` (recording store, locked wrapper); `read_only_store_refuses_set_profiling_level_with_read_only_error` and proves the inner store was not called; memory-store tests (scripted entries filtered by `since`, level changes after `set_profiling_level`, scripted error).
- [ ] **Step 2: Run** `cargo test -p curlybrace-core profile read_only` → FAIL.
- [ ] **Step 3: Implement `mongo_profile.rs`** (pure command builders + runners; mongos detected via the existing `topology()` → `CoreError::Unsupported("profiler not available through mongos")`; server errors via `classify_driver_error`, so privilege failures surface as `Unauthorized`; apply the connection's selection criteria like `mongo_explain.rs` does for the two reads, but `set_profiling_level` always targets the primary).
- [ ] **Step 4: Run** `cargo test -p curlybrace-core` → PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(profile): read and set the profiler through the document store`

### Task 5: Mark our own connections (`appName`)

**Files:**
- Modify: `crates/core/src/client.rs` (apply the marker in `build_client_options`)
- Test: `crates/core/src/client_tests.rs` (or the existing client test file)

**Interfaces:**
- Consumes: `profile::OWN_APP_NAME` (Task 2).
- Produces: `pub fn with_own_app_name(options: ClientOptions) -> ClientOptions` — absent → `curlybrace`; present → `"<theirs> curlybrace"` (truncated from the user's part so the whole is at most 128 bytes, the driver limit).

- [ ] **Step 1: Write failing tests:** `sets_the_marker_when_absent`, `appends_the_marker_to_a_user_supplied_app_name`, `keeps_the_whole_value_within_128_bytes`, `does_not_double_append_when_already_marked`, `uri_with_app_name_bob_at_laptop_round_trips` (existing URI test value), and `build_client_options_applies_the_marker`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core client` → FAIL.
- [ ] **Step 3: Implement** and call it from `build_client_options` after the URI is parsed (every connect, test-connection, compare and copy path goes through it; verify with a grep for other `Client::with_options` callers and fix any that bypass it).
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(client): mark our own connections with an appName so the profiler can skip them`

### Task 6: Runtime commands, events and room in `runtime/mod.rs`

**Files:**
- Create: `crates/core/src/runtime/command.rs` (move the `impl Command` blocks: `is_write`, `context`, `session`, request helpers — a pure move, no behaviour change, done as its own first commit), `crates/core/src/runtime/profiler_handlers.rs`
- Modify: `crates/core/src/runtime/mod.rs` (new `Command`/`Event` variants, intercepts), `crates/core/src/runtime/handlers.rs` (dispatch to `profiler_handlers`), `crates/ui/src/app/reducer.rs` (minimal ignoring arms so the workspace compiles; real handling is Task 8)
- Test: `crates/core/src/runtime/profiler_tests.rs`, `crates/core/src/runtime/read_only_tests.rs` (command table: bump `COMMAND_VARIANTS`, classify the new commands)

**Interfaces:**
- Consumes: `profiling_level`/`set_profiling_level`/`shapes_for` (Tasks 3, 4).
- Produces:
  - `Command::ProfilingState { session: SessionId, db: String }`, `Command::SetProfilingLevel { session: SessionId, db: String, level: i32 }` (`is_write() == true`), `Command::QueryShapes { session: SessionId, ns: Namespace, range: ChartRange, request: u64 }`
  - `Event::ProfilingState { session: SessionId, db: String, outcome: Result<ProfilingState, CoreError> }` (both profiler commands answer with it; failures are an `outcome: Err`, never `Event::Failed`), `Event::QueryShapes { session: SessionId, ns: Namespace, range: ChartRange, request: u64, shapes: Vec<ShapeSummary> }`, and a failed `QueryShapes` is an `Event::RequestFailed` like `IndexHistory`
  - `Event::session()` and every exhaustive match updated

- [ ] **Step 1: Do the pure move** of the `impl Command` blocks into `command.rs`; `cargo test --workspace` stays green; commit `refactor(runtime): move command helpers out of mod.rs`.
- [ ] **Step 2: Write failing tests** in `profiler_tests.rs`: `profiling_state_answers_with_the_level`, `set_profiling_level_updates_and_answers`, `set_profiling_level_is_refused_on_a_read_only_session_and_leaves_the_level`, `profiler_errors_are_outcomes_not_failed_events` (unauthorised, mongos-style `Unsupported`), `timeout_is_an_outcome_not_a_failure` (hanging store + short `command_timeout`, mirroring `explain_timeout_is_an_outcome_not_a_failed_event`), `query_shapes_answers_from_history_for_the_namespace`, `query_shapes_without_history_is_empty`, and the read-only command table entries.
- [ ] **Step 3: Run** `cargo test -p curlybrace-core runtime::profiler` → FAIL.
- [ ] **Step 4: Implement** the variants and handlers (follow `ExplainFind`/`IndexHistory` for the timeout/settle behaviour; `SetProfilingLevel` is the only write and keeps the normal write gate).
- [ ] **Step 5: Run** `cargo test --workspace` → PASS; keep `runtime/mod.rs` under 800 lines.
- [ ] **Step 6: Commit** `feat(profile): profiler and query-shape commands in the runtime`

### Task 7: The profile poller and typed-verdict recording

**Files:**
- Create: `crates/core/src/runtime/profiler.rs` (poller task and pure fold), `crates/core/src/runtime/profiler_status.rs`
- Modify: `crates/core/src/runtime/indexes.rs` or a new `runtime/profiler_start.rs` (start/stop beside `start_sampler`), `crates/core/src/runtime/session.rs` (a poller handle slot with the same generation rule as `attach_sampler`), `crates/core/src/runtime/mod.rs` (`Event::Profiler`), `crates/core/src/runtime/handlers.rs` (`ExplainFind` records the typed shape after a successful classify)
- Test: `crates/core/src/runtime/profiler_poll_tests.rs`, `crates/core/src/runtime/typed_record_tests.rs`

**Interfaces:**
- Consumes: Tasks 1–5 (shape, entry classify, history, store reads, own-traffic).
- Produces:
  - `pub enum ProfilerStatus { Idle, Reading { databases: usize }, MissedEntries { db: String }, Paused(SamplerPause) }` and `Event::Profiler { session: SessionId, status: ProfilerStatus }` (forwarded through the same epoch-checked `forward()` as `Event::Sampler`)
  - `pub fn fold_entries(connection: Uuid, host: &str, db: &str, entries: &[ProfileEntry], bucket_secs: i64) -> Vec<ShapeObservation>` (pure: drops own traffic, builds the shape per entry via `from_profile_command`, classifies, aggregates per shape and bucket; entries without a shape are skipped)
  - `pub fn spawn(store: Arc<dyn DocumentStore>, history: Arc<HistoryDb>, connection: Uuid, cfg: watch::Receiver<SamplerConfig>, status: UnboundedSender<ProfilerStatus>) -> JoinHandle<()>` — per tick, for every non-system, non-excluded database: `profiling_level`; for level > 0 `read_profile(db, last_ts)`; fold; `record_shapes`; advance the per-db `last_ts` watermark; when `oldest_ts_secs > last_ts` call `record_gap` and report `MissedEntries`; errors pause through `pause_reason`/`backoff` exactly like the sampler; the task never calls `set_profiling_level`

- [ ] **Step 1: Write failing tests:** pure `fold_entries`: `groups_by_shape_and_bucket`, `own_traffic_is_dropped`, `counts_misses_and_worst_verdict`, `no_literal_value_reaches_an_observation`; poller (InMemoryStore + paused tokio clock + in-memory history): `reads_only_databases_with_the_profiler_on`, `never_enables_the_profiler` (assert the store saw no `set_profiling_level`), `watermark_prevents_double_counting_across_ticks`, `roll_off_records_a_gap_and_reports_missed_entries`, `read_only_connection_still_reads_and_records`, `unauthorised_pauses_with_not_authorized_and_recovers`, `excluded_and_system_databases_are_skipped`, `stops_on_disconnect_and_ignores_a_stale_generation`; typed: `successful_explain_records_a_typed_shape_with_its_verdict`, `failed_explain_records_nothing`, `typed_record_works_on_a_read_only_session`, `typed_record_stores_no_literals`, `purged_connection_is_not_recorded`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core runtime::profiler runtime::typed` → FAIL.
- [ ] **Step 3: Implement** the pure fold first, then the poller reusing `SamplerConfig`'s interval/excluded/timeout, `pause_reason`, `backoff` and `blocking` from the sampler; start it from the same call site as `start_sampler` and attach it with the generation check; record typed shapes in the `ExplainFind` handler off the blocking pool, ignoring (but `log::warn!`-ing the variant name of) history errors so a history problem never affects the verdict.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS; clippy clean.
- [ ] **Step 5: Commit** `feat(profile): background profile poller and typed-query shape recording`

### Task 8: UI state, reducer and consent flow

**Files:**
- Create: `crates/ui/src/viewmodel/queries.rs` (list/summary models), `crates/ui/src/app/queries_actions.rs`
- Modify: `crates/ui/src/viewmodel/indexes.rs` (`IndexesView { Indexes, Queries }` toggle in `IndexesState`; keep `has_prompt()` true while the consent prompt is open), `crates/ui/src/app/index_actions.rs` (route), `crates/ui/src/app/write_guard.rs` (`WriteKind::SetProfilingLevel` + warning wording keys), `crates/ui/src/app/reducer.rs` (`Event::ProfilingState`, `Event::QueryShapes`, `Event::Profiler`), `crates/ui/src/app/state.rs`/`session.rs` (park per-session profiler state like `verdict`)
- Test: `crates/ui/src/app/queries_tests.rs`, `crates/ui/src/viewmodel/queries_tests.rs`

**Interfaces:**
- Consumes: Task 6 commands/events, `shape::suggest_for_shape` (Task 1), release A's `VerdictKind` view helpers.
- Produces:
  - `pub struct QueriesState { pub shapes: Vec<ShapeSummary>, pub profiling: ProfilingView, pub status: Option<ProfilerStatus>, pub consent: Option<ProfilerConsent>, pub request: u64, pub range: ChartRange }`, `pub enum ProfilingView { Unknown, Loading, Known(ProfilingState), Unavailable(CoreError) }`, `pub struct ProfilerConsent { pub db: String, pub level: i32 }` with `fn command_text(&self) -> String` (the exact mongosh text, e.g. `db.getSiblingDB("shop").setProfilingLevel(2)`, db name escaped with the same JS-string escaping as release A)
  - `pub enum QueriesAction { Open, Refresh, SetRange(ChartRange), AskEnable, ConfirmEnable, CancelEnable, AskDisable, ConfirmDisable }`
  - `pub fn rows(state: &QueriesState, collection: &str) -> Vec<QueryRow>` where `QueryRow { shape_json: String, source: ShapeSource, executions: u64, misses: u64, max_millis: u64, worst: VerdictKind, suggestion: Option<String> }` sorted misses first (worst severity desc, then misses desc, then max millis desc)

- [ ] **Step 1: Write failing tests:** `opening_the_queries_view_asks_for_profiling_state_and_shapes`; `enable_asks_consent_and_sends_nothing`; `confirm_sends_set_profiling_level_2_for_the_namespace_db`; `cancel_sends_nothing_and_closes_the_prompt`; `read_only_connection_refuses_enable_with_the_read_only_banner_and_no_command`; `production_session_parks_the_write_warning_first`; `stale_query_shapes_answer_is_dropped` (by request/ns/range, like `IndexHistory`); `profiler_unavailable_outcomes_keep_the_view_usable` (Unauthorized, Unsupported/mongos); `disable_requires_confirmation`; `rows_sort_misses_first`; `consent_command_text_escapes_the_database_name` (quotes, `)`, U+2028).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui queries` → FAIL.
- [ ] **Step 3: Implement** the state, actions and routing through the existing `actions::mutation` path with `Guard::Warn` (as `set_hidden` does), in-flight tracking so the buttons disable while a write is pending.
- [ ] **Step 4: Run** `cargo test --workspace` → PASS.
- [ ] **Step 5: Commit** `feat(queries): state, actions and consent flow for the Queries view`

### Task 9: Queries view, chart and strings

**Files:**
- Create: `crates/ui/src/views/queries.rs`, `crates/ui/src/views/queries_consent.rs`, `crates/ui/src/viewmodel/queries_chart.rs`
- Modify: `crates/ui/src/views/indexes.rs` (the Indexes/Queries toggle in the toolbar; keep it under 800 lines), `crates/ui/locales/en.json` (`indexes.queries.*`)
- Test: `crates/ui/src/viewmodel/queries_chart_tests.rs`, `crates/ui/tests/accessibility_queries.rs`, `crates/ui/tests/accessibility_queries_consent.rs`

**Interfaces:**
- Consumes: `QueriesState`, `QueryRow`, `QueriesAction` (Task 8); the existing chart model/view conventions (`viewmodel/index_chart.rs`, `views/index_chart.rs`) and `ChartRange`.
- Produces: `pub fn miss_chart_model(summaries: &[ShapeSummary], range: ChartRange, now_secs: i64) -> ChartModel`-style struct (bars = misses per bucket summed over shapes, gaps drawn as gaps, same x convention as the index chart); `pub fn show_queries(ui, state: &QueriesState, collection: &str, i18n: &I18n) -> Option<QueriesAction>`; `pub fn show_profiler_consent(ctx, consent: &ProfilerConsent, busy: bool, read_only: bool, production: bool, i18n: &I18n) -> Option<QueriesAction>` (modal: explains what level 2 does and its overhead, shows the exact command in monospace with a copy button, Cancel is default and focused, buttons disabled while busy or read-only, same production notice pattern as the drop-index dialog)

- [ ] **Step 1: Write failing tests:** `miss_chart_counts_misses_per_bucket_and_marks_gaps`; accessibility with the REAL English locale for: profiler off (shows the explain-and-enable call to action), profiler on with rows (miss rows have icon + text, suggestion with an accessible copy button), empty state, unavailable reasons (unauthorised, mongos), "entries may have been missed" note, the consent modal (command text visible, Cancel focused, Confirm disabled when read-only) — each asserting no `⟦` marker and the expected resolved text; the layout does not jump when the status line changes.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui queries` → FAIL.
- [ ] **Step 3: Implement** the views and add the keys alphabetically; every key referenced in production code (`i18n_usage_tests` with the existing `indexes.` prefix). Escape/click-away dismisses any popup you add; the consent dialog behaves like `show_drop_confirm`.
- [ ] **Step 4: Run** `cargo test --workspace` (includes `i18n_usage_tests`) → PASS; list the new keys in the report.
- [ ] **Step 5: Commit** `feat(queries): Queries view with miss trend and the profiler consent dialog`

### Task 10: Real-server tests

**Files:**
- Create: `crates/core/tests/local_profiler.rs`
- Modify: `scripts/integration-tests.sh` (add the target by hand); `crates/core/src/profile/fixtures/` (replace hand-written fixtures with scrubbed real captures, `real_<name>.json`, via a `CURLYBRACE_CAPTURE=1` mode as `local_explain.rs` does); a docker runtime test `crates/core/tests/docker_profiler_poller.rs` + `scripts/container-tests.sh` entry

**Interfaces:**
- Consumes: `MongoStore` profile methods, `fold_entries`, the runtime with a real `history_path`; harness conventions from `local_explain.rs` (`SERIAL` lock, `local_enabled`, throwaway `cbit_<id>` databases) and `docker_index_sampling.rs`.

- [ ] **Step 1: Write the tests** (opt-in, `#[ignore]`, gated by `CURLYBRACE_IT=1`, serialized): `real_profiling_level_roundtrip` (0 → 2 → 0 on a throwaway db), `real_read_profile_returns_entries_after_queries_with_the_expected_fields`, `real_collscan_and_ixscan_entries_classify_as_expected`, `real_own_app_name_marker_is_excluded` (connect with and without a URI `appName`), `real_system_profile_reads_are_excluded`, `real_set_profiling_level_is_refused_on_a_read_only_store_and_level_unchanged`, `real_unauthorised_user_gets_a_clear_error` (only if a restricted user can be created on the throwaway server cheaply; otherwise document why not), `real_roll_off_detected` (shrink the profile collection of the throwaway db via `profile` + recreate capped at a tiny size, in that throwaway db only, write more entries than fit, assert the poller records a gap), and the docker runtime test: `poller_records_app_queries_into_history_and_survives_restart` (queries via a second client → shapes appear in `shapes_for`; no `set_profiling_level` call from the poller; typed explain recorded as `source = typed`).
- [ ] **Step 2: Start a throwaway `mongod`** on port 27018 with a fresh dbpath in the scratchpad directory and `--setParameter enableTestCommands=1`, run the new file with `CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI=mongodb://127.0.0.1:27018 cargo test -p curlybrace-core --test local_profiler -- --ignored`, fix what the real server disagrees with (entry field names/types, `planSummary` forms, `hasSortStage`, the `command` shape per op) with RED-first unit tests on the REAL captured fixtures, and record every production change in the report. Stop the `mongod` and verify port and process are gone when done.
- [ ] **Step 3: Add** both new files to the scripts by hand; run `scripts/integration-tests.sh` for the new target; `cargo test --workspace` and `scripts/check-no-db-in-ci.sh` stay green.
- [ ] **Step 4: Commit** `test(profile): profiler and poller against a real server` (production fixes, if any, in a separate `fix(profile): ...` commit).

---

## Self-review

- **Spec coverage:** profiler enable with consent, level 2 per database, never silent (Tasks 4, 6, 8, 9); read-only reads allowed, enable refused (4, 6, 8); own traffic excluded via `appName` + `system.profile` + `$indexStats` (2, 5, 7); polling at the sampler cadence with a `ts` watermark and roll-off gap (7); shapes without literals + stable hash (1); v2 tables, retention, shared cap, purge, typed rows (3, 7); Queries section with misses first, run count, slowest time, verdict, suggestion, trend chart over 24h/7d/30d with gaps (8, 9); mongos reason (4, 8, 9); i18n + accessibility (9); real-server and docker tests (10).
- **Type consistency:** `Shape`, `ProfileEntry`, `ProfilingState`, `ProfileRead`, `ShapeObservation`, `ShapeKey`, `ShapeSummary`, `ShapeBucket`, `ShapeSource`, `ProfilerStatus`, `QueriesState`, `QueryRow` are each defined once and used by the same names in later tasks; `VerdictKind::severity` (Task 2) is what Task 3's `worst` and Task 8's sort use.
- **Proportion:** ten tasks for the larger half of the issue; signatures and tests are pinned, bodies are not.
