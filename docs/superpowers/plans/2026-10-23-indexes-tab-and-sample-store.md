# Indexes Tab and Usage Sample Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A per-collection Indexes tab that lists, creates, edits (as a copy), hides and drops indexes, and shows index usage over time from a local SQLite sample store filled by a background sampler.

**Architecture:** Index operations join the `DocumentStore` trait (`crates/core/src/store.rs`) so the `ReadOnlyStore` wrapper, the in-memory fake and the real `MongoStore` all carry them. A sampler task per open session reads `$indexStats` for every collection and writes rows to a SQLite `HistoryDb`. The UI adds a `CollectionTab` (Documents / Indexes) to `SessionView`/`AppState`, new `Command`/`Event` variants in the runtime, and pure view-models in front of thin egui views.

**Tech Stack:** Rust 2024, egui/eframe 0.32.3, `mongodb` 3.6 driver, `rusqlite` (bundled), `egui_plot` (the release matching egui 0.32), Docker tier-2 container tests.

**Spec:** `docs/superpowers/specs/2026-10-23-indexes-tab-and-sample-store-design.md`

## Global Constraints

- Sample interval 60 seconds; raw samples kept 48 hours; hourly buckets 90 days; daily buckets 2 years; history file size cap about 100 MB; large-collection warning above 1 million documents; a gap is declared after 3 missed intervals. All are constants in the first version, not settings.
- Sampled: every collection (kind `Collection` only) in every database except `admin`, `local`, `config` and the databases the user excluded.
- Chart ranges: 24 hours, 7 days, 30 days. Layout B (list left, detail right).
- Builder key types: ascending, descending, text, 2dsphere, wildcard. Hashed and 2d keys are listed, shown, copied as JSON and preserved when editing a copy, but never offered in the builder.
- Series identity: index name plus a hash of the definition (keys + unique, sparse, TTL, partial filter, collation, wildcard projection; NOT `hidden`, NOT the name).
- History file: config directory (`curlybrace`), mode 0600, rows carry connection id, host (node), database, collection, index name, definition hash.
- Restart rule: new segment when `accesses.since` changes or the counter decreases; the first sample of a segment has no delta. A restart inside a gap is marked as both.
- Project rules: every user-facing string is an i18n key in `crates/ui/locales/en.json` (report new keys); immutable state (reducers return new state); files under 800 lines; TDD; coverage at least 80%; real domain objects in tests via `crates/core/src/test_support/mock_objects.rs`, mocks only at boundaries.
- Commit messages follow `<type>(scope): description` and end with the `Co-Authored-By` trailer from the session attribution.
- Index writes (create, drop, hide, TTL) are write commands: they go through the existing read-only refusal and the production write warning exactly like Insert/Replace/Delete.

## Review Focus

Failure modes the spec implies that no feature task's main tests cover. Each has a test in the task named in brackets.

1. **Views and time-series collections** make `$indexStats` fail. The sampler must skip them, not pause. [Task 6]
2. **A user without the `indexStats` privilege** can still `listIndexes`. The tab must still list indexes and show "sampling paused: not authorized", never an empty tab or a banner loop. [Task 6, Task 9]
3. **`_id_`** cannot be dropped or hidden by the server. The UI must disable both actions instead of offering them. [Task 10]
4. **System clock moving backwards** must not produce negative spans or reordered points. [Task 4]
5. **Thousands of collections** must not make one sampling tick overrun the next. Concurrency is bounded and a tick that overruns skips, not stacks. [Task 6]

---

### Task 1: Index types, create command, validation

**Files:**
- Create: `crates/core/src/indexes/mod.rs`, `crates/core/src/indexes/spec.rs`, `crates/core/src/indexes/validate.rs`, `crates/core/src/indexes/tests.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod indexes;`), `crates/core/src/test_support/mock_objects.rs`

**Interfaces:**
- Produces (all `Clone, Debug, PartialEq`):
  - `enum KeyKind { Asc, Desc, Text, Geo2dSphere, Wildcard, Hashed, Geo2d }`
  - `struct IndexKey { field: String, kind: KeyKind }`
  - `struct IndexOptions { unique: bool, sparse: bool, hidden: bool, expire_after_seconds: Option<i64>, partial_filter: Option<Document>, collation: Option<Document>, wildcard_projection: Option<Document> }` (`Default`)
  - `struct IndexSpec { name: String, keys: Vec<IndexKey>, options: IndexOptions, raw: Document }`
  - `IndexSpec::from_document(doc: &Document) -> Result<IndexSpec>` (`CoreError` on a document without `key`/`name`)
  - `IndexSpec::create_command(&self, collection: &str) -> Document` (the `createIndexes` command; the UI preview and the store both call this one function)
  - `IndexSpec::definition_hash(&self) -> String` (hex SHA-256 over the identity fields above)
  - `IndexSpec::to_json(&self) -> String` (the definition as relaxed extended JSON, for "copy as JSON")
  - `enum IndexProblem { NoKeys, EmptyField, DuplicateField(String), TtlNeedsOneOrderedKey, TtlNotPositive }`
  - `validate_new(spec: &IndexSpec) -> std::result::Result<(), IndexProblem>`
  - `mock_objects::mock_index_spec() -> IndexSpec` and `mock_index_spec_named(name: &str, keys: &[(&str, KeyKind)]) -> IndexSpec`

- [ ] **Step 1: Write failing tests** in `indexes/tests.rs`, with these names and assertions:
  - `from_document_reads_keys_in_order_and_options`: `{key:{a:1,b:-1}, name:"a_1_b_-1", unique:true, expireAfterSeconds:60}` gives keys `[a Asc, b Desc]`, `unique`, TTL `Some(60)`.
  - `from_document_keeps_hashed_and_2d_keys`: `{key:{h:"hashed"}}` gives `KeyKind::Hashed`; `{key:{p:"2d"}}` gives `Geo2d`.
  - `create_command_matches_the_server_shape`: for `mock_index_spec()` the document equals `{createIndexes:"orders", indexes:[{key:{customerId:1, createdAt:-1}, name:"customerId_1_createdAt_-1"}]}`; options appear only when set (`unique`, `sparse`, `hidden`, `expireAfterSeconds`, `partialFilterExpression`, `collation`, `wildcardProjection`).
  - `definition_hash_ignores_name_and_hidden`, `definition_hash_changes_with_keys_order_or_options` (reordering keys changes it; toggling `unique` changes it).
  - `to_json_round_trips_through_from_document`.
  - `validate_new_rejects` one test per `IndexProblem`; `validate_new_accepts_a_wildcard_with_projection`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core indexes:: --locked`. Expected: FAIL (module missing).
- [ ] **Step 3:** Implement the types, `from_document`, `create_command`, `definition_hash` (reuse `sha2`, already a dependency), `to_json` (reuse `ejson.rs`) and `validate_new`. `raw` keeps the server's original document so unknown fields survive a copy.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit `feat(indexes): index spec types, create command and validation (#68)`.

### Task 2: Index methods on `DocumentStore`, read-only guard, in-memory fake

**Files:**
- Modify: `crates/core/src/store.rs`, `crates/core/src/read_only_store.rs`, `crates/core/src/read_only_store_tests.rs`, `crates/core/src/test_support/memory_store.rs`

**Interfaces:**
- Consumes: Task 1 types.
- Produces, on `DocumentStore` (defaults return `CoreError::Unsupported`):
  - `struct CollectionIndex { spec: IndexSpec, size_bytes: u64 }`
  - `struct IndexUsage { name: String, host: String, ops: i64, since_ms: i64, spec: IndexSpec }`
  - `async fn list_indexes(&self, ns: &Namespace) -> Result<Vec<CollectionIndex>>`
  - `async fn index_stats(&self, ns: &Namespace) -> Result<Vec<IndexUsage>>` (one row per node)
  - WRITE: `async fn create_index(&self, ns: &Namespace, spec: &IndexSpec) -> Result<()>`, `async fn drop_index(&self, ns: &Namespace, name: &str) -> Result<()>`, `async fn set_index_hidden(&self, ns: &Namespace, name: &str, hidden: bool) -> Result<()>`, `async fn set_index_ttl(&self, ns: &Namespace, name: &str, seconds: i64) -> Result<()>`
  - Memory store helper: `MemoryStore::with_index_usage(self, ns: &Namespace, usage: Vec<IndexUsage>) -> Self` (scripts `index_stats`) and `set_index_usage(&self, ns, usage)` for tests that change it between samples.

- [ ] **Step 1: Write failing tests.** In `read_only_store_tests.rs` add the four writes to the `WRITES` list so `every_write_method_is_covered` fails; add `read_only_refuses_create_drop_hide_and_ttl` (each returns `CoreError::ReadOnly` while locked and reaches the inner store when unlocked) and `read_only_forwards_list_indexes_and_index_stats`. In the memory store tests: `memory_store_creates_lists_hides_and_drops_an_index` and `memory_store_rejects_a_duplicate_index_name`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core read_only_store memory_store --locked`. Expected: FAIL.
- [ ] **Step 3:** Add the trait methods with the doc-comment pattern the file uses for write methods (point at `ReadOnlyStore` and the `WRITES` list). `ReadOnlyStore` forwards every method explicitly and calls `self.guard()?` first on the four writes. Implement them in the memory store (indexes per namespace in a map, `_id_` always present).
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit `feat(indexes): index operations on the document store seam (#68)`.

### Task 3: `MongoStore` implementation and container tests

**Files:**
- Modify: `crates/core/src/mongo_store.rs` (or add `crates/core/src/mongo_indexes.rs` and call it from `mongo_store.rs` if the file would pass 800 lines), `scripts/container-tests.sh`
- Create: `crates/core/tests/docker_mongo_indexes.rs`

**Interfaces:**
- Consumes: Task 2 trait methods. Produces: real implementations; `list_indexes` takes definitions from `listIndexes` and sizes from `collStats.indexSizes`; `index_stats` runs the `$indexStats` aggregation; `set_index_hidden`/`set_index_ttl` use `collMod` with an `index` document; `create_index`/`drop_index` use `createIndexes`/`dropIndexes`.

- [ ] **Step 1: Write failing container tests** (gated on `docker_enabled`, using `MongoServer` from `curlybrace-testkit`; one container shared by the file): `creates_and_lists_a_single_field_index`, `..._compound_index`, `..._text_index`, `..._2dsphere_index`, `..._wildcard_index_with_projection`, `..._unique_index`, `..._ttl_index`, `..._partial_index`, `..._collation_index`, `hide_sets_hidden_in_list_indexes`, `unhide_clears_it`, `set_ttl_changes_expire_after_seconds_via_collmod`, `drop_removes_the_index`, `dropping_id_is_refused_by_the_server_and_reported`, `index_stats_reports_ops_that_grow_after_a_query_uses_the_index`, `list_indexes_reports_a_size_for_every_index`.
- [ ] **Step 2:** Run `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test docker_mongo_indexes --locked -- --include-ignored`. Expected: FAIL (`Unsupported`).
- [ ] **Step 3:** Implement against the driver. Map driver errors with the existing `db_err`. Add the test binary to the `cargo test` line in `scripts/container-tests.sh`.
- [ ] **Step 4:** Run the same command. Expected: PASS (needs Docker).
- [ ] **Step 5:** Commit `feat(indexes): list, create, hide and drop indexes on MongoDB (#68)`.

### Task 4: Pure usage-series logic

**Files:**
- Create: `crates/core/src/history/mod.rs`, `crates/core/src/history/series.rs`, `crates/core/src/history/series_tests.rs`
- Modify: `crates/core/src/lib.rs` (add `pub mod history;`)

**Interfaces:**
- Produces:
  - `struct Sample { at_secs: i64, ops: i64, since_ms: i64 }`
  - `enum Point { Delta { at_secs: i64, uses: i64, span_secs: i64 }, Restart { at_secs: i64, uses_since_restart: i64 }, Gap { from_secs: i64, to_secs: i64, uses: i64 } }`
  - `struct Bucket { start_secs: i64, width_secs: i64, uses: i64, restarts: u32, gap_secs: i64 }`
  - `const SAMPLE_INTERVAL_SECS: i64 = 60; const GAP_INTERVALS: i64 = 3;`
  - `fn derive(samples: &[Sample]) -> Vec<Point>`; `fn bucketize(points: &[Point], width_secs: i64) -> Vec<Bucket>`

- [ ] **Step 1: Write failing tests** in `series_tests.rs`: `consecutive_samples_give_deltas`, `a_lower_counter_is_a_restart_with_no_delta`, `a_changed_since_is_a_restart`, `a_long_silence_is_a_gap_carrying_the_uses_in_it`, `a_restart_inside_a_gap_is_both` (a Gap point then a Restart point), `the_first_sample_has_no_point`, `bucketize_sums_deltas_and_counts_restarts_and_gap_seconds`, `a_gap_never_becomes_a_zero_delta`. Review-focus test: `a_sample_not_after_the_previous_one_is_ignored` (clock moved back). Property test (`proptest`, already a dev-dependency): `bucketing_preserves_total_uses_for_any_sample_sequence`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core history::series --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement `derive` (sort, drop non-increasing timestamps, classify each consecutive pair by the Global Constraints rule) and `bucketize` (a Gap's seconds are added to `gap_secs`, spread across the buckets it spans).
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit `feat(history): derive usage points with restart and gap handling (#68)`.

### Task 5: SQLite `HistoryDb`

**Files:**
- Create: `crates/core/src/history/db.rs`, `crates/core/src/history/db_tests.rs`
- Modify: `crates/core/Cargo.toml` (add `rusqlite` with the `bundled` feature), `crates/core/src/history/mod.rs`

**Interfaces:**
- Consumes: Task 4 types. Produces:
  - `struct SeriesKey { connection: Uuid, host: String, db: String, collection: String, index_name: String, definition_hash: String }`
  - `enum ChartRange { Day, Week, Month }` with `fn secs(self) -> i64` and `fn bucket_width_secs(self) -> i64` (600, 3600, 86400)
  - `HistoryDb::open(path: &Path) -> Result<(HistoryDb, Option<Recovery>)>` (`Recovery::SetAside(PathBuf)` when a corrupt file was moved to `<name>.corrupt` and a fresh one created); `HistoryDb::open_in_memory() -> Result<HistoryDb>`
  - `fn record(&self, key: &SeriesKey, sample: Sample) -> Result<()>`
  - `fn usage(&self, key: &SeriesKey, range: ChartRange, now_secs: i64) -> Result<Vec<Bucket>>`
  - `fn latest(&self, key: &SeriesKey) -> Result<Option<Sample>>`
  - `fn series_for(&self, connection: Uuid, db: &str, collection: &str) -> Result<Vec<SeriesKey>>`
  - `fn thin(&self, now_secs: i64) -> Result<()>` (raw older than 48 h becomes hourly rows, hourly older than 90 d becomes daily rows, daily older than 2 y is deleted, then the size cap drops the oldest rows)
  - `fn purge_connection(&self, connection: Uuid) -> Result<()>`

- [ ] **Step 1: Write failing tests** in `db_tests.rs` (use `open_in_memory`, plus `tempfile` for the file tests): `records_and_reads_back_a_series`, `usage_returns_buckets_of_the_range_width`, `a_recreated_index_with_a_different_definition_is_a_separate_series` (same name, different hash, usage never joins them), `thinning_keeps_restarts_and_gap_seconds_in_hourly_buckets`, `thinning_drops_daily_rows_older_than_two_years`, `the_size_cap_removes_the_oldest_rows_first`, `purge_connection_removes_only_that_connections_rows`, `a_different_host_is_a_separate_series`, `the_file_is_created_with_mode_0600` (unix), `a_corrupt_file_is_set_aside_and_a_fresh_one_opened`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core history::db --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement with a `Mutex<rusqlite::Connection>` (the runtime is async; every call is short and blocking, so call it from `spawn_blocking`). Schema and migrations are the builder's choice, but keep a `user_version` pragma for migrations. Re-bucketing for `usage` reuses `derive` and `bucketize` from Task 4.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit `feat(history): SQLite sample store with thinning and a size cap (#68)`.

### Task 6: Sampler

**Files:**
- Create: `crates/core/src/runtime/sampler.rs`, `crates/core/src/runtime/sampler_tests.rs`
- Modify: `crates/core/src/runtime/mod.rs` (module declarations only)

**Interfaces:**
- Consumes: Tasks 2, 4, 5; `crate::clock` for time. Produces:
  - `struct SamplerConfig { interval: Duration, excluded_dbs: BTreeSet<String>, max_concurrency: usize }` (`Default`: 60 s, empty, 4)
  - `enum SamplerPause { NotAuthorized, ConnectionLost, HistoryUnavailable }` and `enum SamplerStatus { Running, Paused(SamplerPause) }`
  - `async fn sample_once(store: &dyn DocumentStore, history: &HistoryDb, connection: Uuid, cfg: &SamplerConfig, now_secs: i64) -> Result<usize>` (number of series recorded)
  - `fn spawn(store: Arc<dyn DocumentStore>, history: Arc<HistoryDb>, connection: Uuid, cfg: watch::Receiver<SamplerConfig>, status: UnboundedSender<SamplerStatus>) -> JoinHandle<()>`

- [ ] **Step 1: Write failing tests** against `MemoryStore` and an in-memory `HistoryDb`: `it_samples_every_user_collection_and_skips_admin_local_config`, `it_skips_user_excluded_databases`, `it_skips_views_and_timeseries_collections` (Review Focus 1), `it_records_one_series_per_node`, `a_not_authorized_error_pauses_with_not_authorized_and_keeps_the_task_alive` (Review Focus 2), `a_connection_error_pauses_and_backs_off_up_to_ten_minutes`, `it_resumes_and_reports_running_after_a_good_tick`, `concurrency_never_exceeds_the_configured_limit` and `an_overrunning_tick_skips_the_next_instead_of_stacking` (Review Focus 5, with a store that sleeps), `the_task_stops_when_aborted`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core runtime::sampler --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement discovery with `list_databases` + `list_collection_kinds`, fetch with `index_stats`, bounded by `futures::stream::StreamExt::buffer_unordered`. Map errors with a function `pause_reason(&CoreError) -> SamplerPause`. Use `tokio::time::interval` with `MissedTickBehavior::Skip`. Run `HistoryDb::thin` once an hour. A failing history write pauses with `HistoryUnavailable`; nothing fails silently (log with `log::warn!`).
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit `feat(history): background index usage sampler (#68)`.

### Task 7: Runtime commands and events

**Files:**
- Modify: `crates/core/src/runtime/mod.rs`, `crates/core/src/runtime/handlers.rs`, `crates/core/src/runtime/session.rs` (sampler handle), `crates/core/src/runtime/read_only_tests.rs`
- Create: `crates/core/src/runtime/index_tests.rs`, `crates/core/tests/docker_index_sampling.rs`
- Modify: `scripts/container-tests.sh`

**Interfaces:**
- Consumes: Tasks 2, 5, 6. Produces:
  - Commands: `ListIndexes { session, ns }`, `IndexHistory { session, ns, range: ChartRange }`, `CreateIndex { session, ns, spec: IndexSpec }`, `DropIndex { session, ns, name: String }`, `SetIndexHidden { session, ns, name: String, hidden: bool }`, `SetIndexTtl { session, ns, name: String, seconds: i64 }`, `SetSamplingExcluded { databases: BTreeSet<String> }` (no session, no answer event, like `LockReadOnly`), `PurgeHistory { connection: Uuid }` (same).
  - Events: `Indexes { session, ns, indexes: Vec<CollectionIndex> }`, `IndexHistory { session, ns, range, series: Vec<IndexSeries> }` with `IndexSeries { index_name: String, definition_hash: String, host: String, buckets: Vec<Bucket>, latest: Option<Sample> }`, `IndexChanged { session, ns }` (answer to the four writes), `Sampler { session, status: SamplerStatus }` (spontaneous; document it as the second exception to "exactly one event per command", next to compare).
  - `RuntimeConfig.history_path: Option<PathBuf>` (default: `curlybrace/history.sqlite` in the config dir; `None` disables sampling).
  - `Command::is_write` is true for the four index writes; `context()` strings `list_indexes`, `index_history`, `create_index`, `drop_index`, `set_index_hidden`, `set_index_ttl`.

- [ ] **Step 1: Write failing tests** in `index_tests.rs` using `spawn_runtime_with_store` and `MemoryStore`: `list_indexes_answers_with_the_collections_indexes`, `index_history_answers_with_series_from_the_history_db`, `create_index_answers_index_changed_and_the_next_list_shows_it`, `a_read_only_session_refuses_all_four_index_writes_with_read_only`, `every_command_still_yields_exactly_one_event_except_sampler_and_compare`, `connecting_starts_the_sampler_and_disconnect_stops_it`, `purge_history_removes_the_connections_rows`, `set_sampling_excluded_reaches_a_running_sampler`. In `read_only_tests.rs` extend the existing write-command table with the four commands.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core runtime:: --locked`. Expected: FAIL (match arms missing, then missing behavior).
- [ ] **Step 3:** Implement in `handlers.rs`; start the sampler where a connect installs the store (`install_if_current`) and abort it on disconnect or replacement; share one `Arc<HistoryDb>` in `Shared`. Update every exhaustive `match` over `Command`/`Event` (`session()`, `is_write`, `context`).
- [ ] **Step 4:** Add the container test `docker_index_sampling.rs`: `a_server_restart_shows_as_a_restart_point_not_a_drop` (record samples, `docker restart`, sample again, assert a `Restart` point and no negative delta) and `a_dropped_and_recreated_index_starts_a_new_series`. Run `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test docker_index_sampling --locked -- --include-ignored`. Expected: PASS. Add it to `scripts/container-tests.sh`.
- [ ] **Step 5:** Run `cargo test -p curlybrace-core --locked`. Expected: PASS. Commit `feat(indexes): runtime commands, events and sampler wiring (#68)`.

### Task 8: UI state, tab switch and reducer wiring

**Files:**
- Create: `crates/ui/src/viewmodel/indexes.rs`, `crates/ui/src/app/index_actions.rs`, `crates/ui/src/app/index_tests.rs`
- Modify: `crates/ui/src/viewmodel/mod.rs`, `crates/ui/src/app/state.rs`, `crates/ui/src/app/session.rs` (`take_view` and `restore_view`), `crates/ui/src/app/reducer.rs`

**Interfaces:**
- Produces:
  - `enum CollectionTab { Documents, Indexes }` (`Default` = `Documents`; `label_key()` returns `indexes.tab` / `browser.tab_documents`)
  - `IndexesState { ns: Option<Namespace>, indexes: Vec<CollectionIndex>, series: Vec<IndexSeries>, selected: Option<String>, range: ChartRange, loading: bool, sampler: Option<SamplerStatus> }` with immutable updaters `with_indexes(..)`, `with_series(..)`, `selecting(name)`, `with_range(r)`
  - `enum IndexesAction { OpenTab, Refresh, Select(String), SetRange(ChartRange), CopyJson(String) }` and `Input::Indexes(IndexesAction)`; `Input::Tab(CollectionTab)`
  - `AppState.tab: CollectionTab` and `AppState.indexes: IndexesState`, both also in `SessionView`.

- [ ] **Step 1: Write failing tests** in `index_tests.rs`: `opening_the_tab_sends_list_indexes_and_index_history_for_the_selected_collection`, `a_result_for_another_collection_or_range_is_dropped`, `switching_collection_resets_the_indexes_state`, `each_session_keeps_its_own_tab_and_indexes_state` (parks and restores through `SessionView`), `the_sampler_event_updates_the_status_for_its_session_only`, `a_failure_in_list_indexes_sets_the_failure_banner`, `the_tab_is_gated_on_a_connected_session`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-ui index_tests --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement the view-model and reducer arms with the `gated` helper pattern from `actions.rs`. The exhaustive destructure in `restore_view` forces the new fields to be carried.
- [ ] **Step 4:** Run `cargo test -p curlybrace-ui --locked`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(indexes): indexes tab state and reducer wiring (#68)`.

### Task 9: Indexes tab views (layout B), chart, i18n

**Files:**
- Create: `crates/ui/src/views/indexes.rs`, `crates/ui/src/views/index_chart.rs`, `crates/ui/src/viewmodel/index_chart.rs`, `crates/ui/tests/accessibility_indexes.rs`, `crates/ui/tests/snapshots/indexes_tab.png` (generated)
- Modify: `crates/ui/Cargo.toml` (`egui_plot`), `crates/ui/src/app/panels.rs` (`show_collection` branches on `state.tab`), `crates/ui/src/views/mod.rs`, `crates/ui/locales/en.json`, `crates/ui/src/i18n_usage_tests.rs` (add the `"indexes."` prefix)

**Interfaces:**
- Consumes: Task 8 state. Produces:
  - `show_indexes(ui: &mut egui::Ui, state: &IndexesState, read_only: bool, i18n: &I18n) -> Option<IndexesAction>`
  - `chart_model(series: &IndexSeries, range: ChartRange, now_secs: i64) -> ChartModel { points: Vec<[f64; 2]>, restarts: Vec<f64>, gaps: Vec<(f64, f64)> }` (pure, unit tested)
  - i18n keys under `indexes.*`: tab label, list heading, per-node note ("usage is per node and resets when the server restarts"), usage columns, "never (since restart)", chart ranges, "restart", "no data", "hidden", "tracked since", every sampler pause reason, sampling status.

- [ ] **Step 1: Write failing tests:** `chart_model_marks_restarts_and_leaves_gaps_unplotted`, `chart_model_never_plots_a_gap_as_zero`; headless `egui_kittest` tests in `accessibility_indexes.rs`: `the_tab_lists_every_index_and_selecting_one_shows_its_detail`, `a_hidden_never_used_index_is_marked_hidden_and_zero`, `a_paused_sampler_shows_its_reason_and_the_list_still_renders` (Review Focus 2), `every_control_has_an_accessible_label`; snapshot test following the pattern in `crates/ui/tests/snapshots.rs`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-ui chart_model accessibility_indexes --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement the list (left, name + usage), the detail (definition line, chart with 24h/7d/30d switch, last used, tracked since, action buttons that report actions, wired in Task 10) and the chart with `egui_plot`. Add all keys to `en.json`; the i18n usage test must pass in both directions.
- [ ] **Step 4:** Run `cargo test -p curlybrace-ui --locked`. Expected: PASS. Generate the snapshot per `CONTRIBUTING.md` (`CURLYBRACE_SNAPSHOTS=1`).
- [ ] **Step 5:** Commit `feat(indexes): indexes tab with usage trend (#68)`.

### Task 10: Builder, edit-a-copy, hide, drop, copy JSON

**Files:**
- Create: `crates/ui/src/viewmodel/index_builder.rs`, `crates/ui/src/views/index_builder.rs`, `crates/ui/src/app/index_write_actions.rs`, `crates/ui/src/app/index_write_tests.rs`, `crates/ui/tests/accessibility_index_builder.rs`
- Modify: `crates/ui/src/app/write_guard.rs` (extend `WriteKind`), `crates/ui/src/viewmodel/indexes.rs`, `crates/ui/src/views/indexes.rs`, `crates/ui/locales/en.json`

**Interfaces:**
- Consumes: Tasks 1, 7, 8, 9. Produces:
  - `IndexBuilderState { ns, editing: Option<IndexSpec>, keys: Vec<IndexKey>, options: IndexOptions, partial_filter_text: String, name: String, problem: Option<IndexProblem> }` with immutable updaters (`adding_key`, `removing_key(i)`, `moving_key(i, to)`, `with_kind(i, kind)`, `with_field(i, text)`, `with_name`, `with_option..`), `fn spec(&self) -> Result<IndexSpec, IndexProblem>`, `fn preview_json(&self) -> String` (calls `IndexSpec::create_command`)
  - `fn copy_name(existing: &[CollectionIndex], base: &str) -> String` (`base_v2`, `base_v3`, never colliding)
  - Extends `IndexesAction` (Task 8) with the variants `OpenBuilder`, `EditCopy(String)`, `Builder(BuilderEdit)`, `Create`, `ConfirmDropOld`, `KeepOld`, `SetHidden(String, bool)`, `SetTtl(String, i64)`, `DropRequested(String)`, `DropTyped(String)`, `DropConfirmed`, where `BuilderEdit` is one variant per updater above. The Task 9 view already returns `Option<IndexesAction>`, so its action buttons need no change beyond emitting the new variants.
  - Edit-a-copy state machine: `Building` -> (`IndexChanged`) -> `AskDropOld` -> `Dropping`; a failed build drops nothing; declining keeps both.

- [ ] **Step 1: Write failing tests:**
  - Builder: `create_is_disabled_with_no_keys_or_duplicate_fields`, `the_preview_is_the_exact_command_the_store_receives`, `a_ttl_with_a_compound_key_is_rejected`, `a_partial_filter_that_is_not_json_is_rejected`, `field_names_with_dots_and_unicode_are_kept_verbatim`, `copy_name_never_collides`.
  - Flow: `editing_a_copy_creates_first_then_asks_before_dropping_the_old_index`, `a_failed_build_never_drops_the_original`, `declining_leaves_both_indexes`, `a_ttl_only_change_uses_set_index_ttl_not_a_copy`, `hiding_sends_set_index_hidden`, `dropping_requires_typing_the_exact_index_name`, `id_index_cannot_be_dropped_or_hidden` (Review Focus 3), `hashed_and_2d_keys_survive_editing_a_copy`, `copy_as_json_puts_the_definition_on_the_clipboard_action`.
  - Guards: `a_read_only_connection_disables_every_write_control_and_shows_why`, `dropping_on_a_production_connection_parks_for_the_production_warning`, `a_collection_over_one_million_documents_shows_the_build_warning` (estimated count comes from the existing `estimated_count`, requested when the builder opens).
  - Accessibility: `builder_controls_are_labelled_and_keyboard_reachable`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-ui index_write accessibility_index_builder --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement the view-model and reducer arms; writes go through `write_guard` exactly as Insert/Replace/Delete do. The builder view has, top to bottom: ordered key rows (field text, type picker, remove, reorder), the options (name, TTL seconds, unique, sparse, hidden, collation, partial filter), the command preview, the large-collection warning, and Create / Cancel; when editing a copy, a line explains that the old index stays until the new one is built.
- [ ] **Step 4:** Run `cargo test -p curlybrace-ui --locked`. Expected: PASS. Regenerate the snapshots.
- [ ] **Step 5:** Commit `feat(indexes): index builder, edit-a-copy, hide and drop (#68)`.

### Task 11: Sampling settings and connection cleanup

**Files:**
- Create: `crates/ui/src/app/sampling_settings.rs`, `crates/ui/src/app/sampling_settings_tests.rs`
- Modify: `crates/ui/src/app/persistence.rs` (or a sibling `settings_file.rs` if it would pass 800 lines), `crates/ui/src/app/connection_ops.rs` (delete a connection), `crates/ui/src/views/indexes.rs`, `crates/ui/locales/en.json`

**Interfaces:**
- Produces: `SamplingSettings { excluded_dbs: BTreeSet<String> }` persisted as `sampling.json` beside `connections.json` (same atomic 0600 write); `Input::Indexes(IndexesAction::ExcludeDatabase(String))` and `IncludeDatabase(String)`; on startup and after a change the app sends `Command::SetSamplingExcluded`; deleting a saved connection sends `Command::PurgeHistory { connection }`.

- [ ] **Step 1: Write failing tests:** `settings_round_trip_through_the_file`, `a_corrupt_settings_file_is_backed_up_and_defaults_apply` (same recovery rule as `connections.json`), `excluding_a_database_sends_set_sampling_excluded`, `deleting_a_connection_sends_purge_history_for_its_id`, `the_excluded_list_is_editable_from_the_tab_and_labelled`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-ui sampling --locked`. Expected: FAIL.
- [ ] **Step 3:** Implement using the existing persistence helpers (atomic write, backup on corruption) rather than a second mechanism.
- [ ] **Step 4:** Run `cargo test -p curlybrace-ui --locked`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(indexes): choose which databases are sampled, purge history with a connection (#68)`.

### Task 12: Docs and full verification

**Files:**
- Modify: `README.md` (feature list and roadmap, if the roadmap names index management), `docs/compare.md` only if it lists features

- [ ] **Step 1:** Update the README to describe the Indexes tab and the local history file (location, what is stored, how to exclude databases). Move index management out of the roadmap section if present.
- [ ] **Step 2:** Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. Expected: all pass.
- [ ] **Step 3:** Run `scripts/container-tests.sh` (needs Docker). Expected: all pass, no leftover `cb-it-*` containers.
- [ ] **Step 4:** Report the new i18n keys (list every key added to `en.json` in the PR description).
- [ ] **Step 5:** Commit `docs: describe the indexes tab and usage history (#68)`.
