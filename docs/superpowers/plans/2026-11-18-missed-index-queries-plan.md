# Queries that miss an index: the rest of #116, Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** Close #116. Releases A and B are on `main` (`ec967ce9`, PRs #127 and #128); this plan covers only the gaps listed in the spec:

- G1: **Create index...** from a Queries row and from the Browse verdict popup.
- G6: **Last run** on each row.
- G2: a **field order** verdict judged from the used index's key pattern.
- G3: Aggregate tab explains recorded as typed shapes.
- G4: a shape operator allowlist and a cleanup of old rows.
- G5: a profiler mark that is ahead of the server clock gets clamped back.

**Architecture:** No new command, event or table.

- Slice 1 is UI only. It reuses `IndexBuilderState::from_suggested` and `index_write_actions::open_with`.
- Slice 2 adds a pure `explain/field_order.rs`, used by `classify` (find explain) and `classify_entry` (profiler), plus one persisted `VerdictKind::FieldOrder`.
- Slice 3 makes three changes:
  - generalises `typed_record` to record any shape and calls it from `aggregate_explain::run`;
  - tightens `shape/normalize.rs` with an allowlist, plus a run-once cleanup in `db_schema::prepare`;
  - adds `ProfileRead.server_now_millis` from the existing `hello`.

**Tech stack:** Rust 1.87 (no let-chains), `bson`, `serde_json`, `sha2`, `rusqlite`, `mongodb` 3.x, egui/eframe 0.32, egui_kittest. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-18-missed-index-queries-design.md`.

**Binding earlier designs:**
- `2026-11-07-index-miss-detection-design.md`;
- plans `2026-11-07-index-miss-detection-release-{a,b}.md`;
- `2026-11-18-find-query-options-design.md` (no typed shape under a hint or a collation).

**Reuse (do not duplicate):**
- core:
  - `explain::{classify, suggest, suggest_from, is_equality, is_range, create_index_command, SuggestedIndex, Reason, VerdictKind::{ALL, code, from_code, severity}, worst, is_inefficient, walk}`;
  - `shape::{from_query, from_profile_command, suggest_for_shape, Shape::{from_json, json, hash}}` and `normalize::to_document`;
  - `history::{is_miss, ShapeObservation, ShapeKey, ShapeSource::Typed, TYPED_HOST, severity_sql}`;
  - `runtime::typed_record`;
  - `explain::parse_aggregate_explain`, `AggPlan`, `InputPlan`;
  - `mongo_profile::hello_host`.
- UI:
  - `IndexBuilderState::from_suggested`, `index_write_actions::open_with`;
  - `views::queries_rows::copyable`, `theme::{compact_text_button, gate, tonal_button}`;
  - `viewmodel::index_detail::Age` and the `indexes.ago_*` keys;
  - `viewmodel::queries_status::row_title`, `viewmodel::verdict::{kind_presentation, kind_tone}`;
  - `views::verdict_badge::{show_suggestion, reason_line}`.

## Global Constraints

- **Toolchain:** `cargo +1.87.0` for everything. No let-chains (`if let .. && let ..` does not compile on 1.87). CI pins 1.87.
- **File sizes:**
  - Files I create: 200-400 lines typical, never over 400.
  - Over-800 files must not grow: `crates/core/src/runtime/handlers.rs` (809: the one `aggregate_explain::run` call at line 442 changes in place, net 0), `crates/core/src/runtime/mod.rs` (1059), `crates/core/src/test_support/memory_store.rs` (910), `crates/core/src/mongo_store.rs` (827), `crates/ui/src/app/mod.rs` (1009: `mod` lines only).
  - Over-400 files get only mechanical edits; new code goes to a new file: `crates/core/src/runtime/profiler/pass.rs` (432), `crates/core/src/history/db.rs` (595).
  - `wc -l` every touched file before each commit.
- **Immutable style:** ops return new values (`Self { .., ..self }`), nothing mutates a passed-in value.
- **TDD:** failing test, see it red, implement, see it green.
- **Real objects over mocks:** captured explain and profile fixtures, `InMemoryStore`, `HistoryDb::open_in_memory()`, real `ShapeSummary` values.
- **Coverage:** at least 80% of new code (`cargo llvm-cov` is installed).
- **i18n:**
  - Every visible string, tooltip and accessible name is a key, written as a literal (`crates/ui/src/i18n_usage_tests.rs`).
  - `crates/ui/locales/en.json` is nested and alphabetical at every level (`en_json_is_nested_and_alphabetical_at_every_level`); unused keys fail.
  - A key is added in the task whose source first names it.
- **Privacy:** never a query value in `history.sqlite`, a log line, an error or a `Debug`. Log `variant_of(&error)` only. Field and index names are allowed (already stored and shown).
- **Real servers:** NEVER connect to any MongoDB already running on the machine. Real-server tests run only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted after:
```bash
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
DBPATH=$(mktemp -d /private/tmp/claude-501/cbit-miq.XXXXXX)
/opt/homebrew/bin/mongod --dbpath "$DBPATH" --bind_ip 127.0.0.1 --port "$PORT" --fork --logpath "$DBPATH/log" --pidfilepath "$DBPATH/pid"
CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" cargo +1.87.0 test -p curlybrace-core --test <file> --locked -- --ignored
kill "$(cat "$DBPATH/pid")"; rm -rf "$DBPATH"
```
  New real-server test files are added by hand to `scripts/integration-tests.sh`.
- **Snapshots:** regenerate only with `CURLYBRACE_SNAPSHOTS=1` (and `UPDATE_SNAPSHOTS=1` to overwrite). CI runs only the `snapshots` test file (`cargo test -p curlybrace-ui --test snapshots -- --include-ignored`), so new PNGs go in `crates/ui/tests/snapshots.rs`. No stray `*.new.png` or `*.diff.png`.
- **Docs and changelog:** never edit `CHANGELOG.md` or older specs and plans.
- **Git:** plain separate Bash calls. Conventional commits ending with
  `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push.
- **Branches:** one per slice, PR to `main`. Slices 1 and 2 say "Part of #116". Slice 3 says "Closes #116".

## Review Focus

- **Nothing is built without the builder:** Create index... only opens `IndexBuilderState::from_suggested`. Read-only is off with a reason and also refused in `open_with`. Production keeps the builder's write guard.
- **FieldOrder never shows a false alarm on a good plan:**
  - an index used in equality-sort-range order is `IndexUsed`/`Covered`;
  - an unparseable `planSummary` gives no field-order judgement;
  - non-±1 keys and plans with several index scans are not judged;
  - the guard (#148) is unchanged.
- **Persisted codes:** `field_order` is new and every old code is unchanged. An older build reading it gets neutral `NoFilter` (test pins `from_code("field_order")` round-trips and `severity_sql` ranks it 2).
- **Privacy:**
  - G4 collapses any document with a non-allowlisted `$` key;
  - the cleanup deletes old rows that no longer validate, once per file, without a `user_version` bump;
  - no new log line carries a value.
- **No new server load:** an Aggregate explain is recorded only when the person pressed Explain; previews are never explained.

---

## Slice 1: act on a miss (branch `feat/116-create-from-miss`)

### T1: rows carry the suggestion as data and the last run (viewmodel)

**Files:**
- modify `crates/ui/src/viewmodel/queries.rs` (`QueryRow` fields, `row_of`, `suggestion_of` returns both forms);
- create `crates/ui/src/viewmodel/queries_row_tests.rs` (registered with `#[cfg(test)] #[path]`; `queries_tests.rs` is left as is).

**Produces:**
```rust
pub struct QueryRow {
    pub hash: String,                       // identifies the row for actions
    pub shape_json: String,
    pub source: ShapeSource,
    pub executions: u64, pub misses: u64, pub max_millis: u64,
    pub worst: VerdictKind, pub latest: VerdictKind,
    pub suggestion: Option<String>,         // the createIndex command, as today
    pub suggested: Option<SuggestedIndex>,  // the same index as data, for the builder
    /// Start (Unix secs) of the newest bucket of the range that ran the shape.
    pub last_run_secs: Option<i64>,
}
pub fn last_run_secs(summary: &ShapeSummary) -> Option<i64>; // max start_secs with executions > 0
```

- [ ] Failing tests:
  - `a_miss_row_carries_the_suggested_index_and_its_command` (the command equals `suggested.command(ns)`);
  - `a_good_row_has_no_suggested_index`;
  - `last_run_is_the_newest_bucket_that_ran`;
  - `last_run_ignores_empty_buckets`;
  - `no_bucket_ran_means_no_last_run`;
  - `row_order_is_unchanged` (the existing severity/misses/slowest/text order).
- [ ] Implement, `cargo +1.87.0 test -p curlybrace-ui queries_row`.
- [ ] Commit `feat(queries): carry the suggested index and the last run on each row`.

### T2: Create index... from a Queries row (actions)

**Files:**
- modify `crates/ui/src/viewmodel/queries.rs` (`QueriesAction::CreateIndex { hash: String }`);
- modify `crates/ui/src/app/queries_actions.rs` (the one arm delegates to a new fn);
- create `crates/ui/src/app/queries_create.rs` (+ `queries_create_tests.rs`);
- modify `crates/ui/src/app/mod.rs` (`mod` line only).

**Produces:**
```rust
/// Opens the index builder pre-filled with the suggestion of the shape `hash` of the open
/// Queries view; nothing without a current miss. Collation: none (an app shape has none).
pub(super) fn create_index(state: AppState, hash: &str) -> Out; // via open_with + from_suggested
```

- [ ] Failing tests (real `ShapeSummary` from `queries_test_support.rs`):
  - `create_index_opens_the_builder_with_the_shape_s_keys` (keys, default name, no collation, `EstimatedCount` command as `open_with` sends);
  - `create_index_on_a_read_only_connection_shows_the_read_only_banner_and_sends_nothing`;
  - `create_index_for_a_shape_that_is_no_longer_a_miss_does_nothing`;
  - `create_index_for_an_unknown_hash_does_nothing`;
  - `create_index_while_an_index_write_is_out_does_nothing`;
  - `create_index_while_disconnected_does_nothing`.
- [ ] Implement, test, `wc -l`.
- [ ] Commit `feat(queries): open the index builder from a query that misses an index`.

### T3: the row's button and last run (view)

**Files:**
- modify `crates/ui/src/views/queries_rows.rs` (`show_rows` gets `enabled: bool, read_only: bool` and returns `Option<QueriesAction>`; the Create button sits after the suggestion's Copy; "Last run" sits on the counts row);
- modify `crates/ui/src/views/queries.rs` (pass and return the action; `enabled` = connected, no write or prompt out, not read-only);
- modify `crates/ui/locales/en.json`.

**i18n (under `indexes.queries`, alphabetical):**
```json
"create_index": "Create index...",
"create_index_n": "Create the suggested index for query {position} of {total}",
"create_index_read_only": "This connection is read-only. Copy the command to create the index elsewhere.",
"last_run": "Last run {when}",
```
`{when}` is `indexes.ago_{now,minutes,hours,days}` through `index_detail::Age`.

- [ ] Failing kittest tests in `crates/ui/tests/accessibility_queries.rs`:
  - `a_miss_row_has_a_named_create_index_button` (accessible name `create_index_n` with position);
  - `create_index_is_off_with_the_read_only_reason` (disabled, description `create_index_read_only`);
  - `a_good_row_has_no_create_index_button`;
  - `each_row_says_when_it_last_ran`.
- [ ] Failing layout test in `crates/ui/tests/indexes_layout.rs`: `queries_rows_with_create_fit_at_800x500` (no widget past the right edge with the 28 pt controls; the first row is on screen).
- [ ] Implement, test, then regenerate `queries_view_{light,dark}` with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots -- --include-ignored queries_view`.
- [ ] Commit `feat(queries): Create index and last run on each query row`.

### T4: Create index... from the Browse verdict popup

**Files:**
- modify `crates/ui/src/views/verdict_badge.rs`:
  - `VerdictAction::CreateIndex`;
  - `show_suggestion(ui, command, create: Option<bool>, i18n)`, where `None` means no button (guard notice, shell guard) and `Some(enabled)` means shown, off when false;
- modify `crates/ui/src/views/guard_notice.rs` and `crates/ui/src/views/shell_guard.rs` (pass `None`, map only `CopySuggestion`; a mechanical edit);
- modify `crates/ui/src/app/browse_panel.rs` (map `CreateIndex` to `FindAction::CreateSuggestedIndex`);
- modify `crates/ui/src/viewmodel/findbar.rs` (the variant);
- create `crates/ui/src/app/verdict_create.rs` (+ `verdict_create_tests.rs`);
- modify `crates/ui/src/app/explain_actions.rs` (one-line delegation);
- modify `crates/ui/locales/en.json`.

**Produces:**
```rust
/// The builder pre-filled with the shown verdict's suggestion and the collation of the query it
/// judged (`VerdictState::Ready { query, verdict }`); nothing for another state or no suggestion.
pub(super) fn create_index(state: AppState) -> Out;
```

**i18n (under `query.verdict`):**
```json
"create_index": "Create index...",
"create_index_read_only": "This connection is read-only. Copy the command to create the index elsewhere.",
```

- [ ] Failing tests:
  - `create_from_verdict_carries_keys_and_the_query_s_collation`;
  - `create_from_verdict_without_suggestion_does_nothing`;
  - `create_from_a_pending_or_unavailable_verdict_does_nothing`;
  - `create_from_verdict_on_read_only_sends_nothing`;
  - kittest `accessibility_verdict.rs::why_popup_offers_create_index_beside_copy`, `guard_notice_has_no_create_index` (both guard notices unchanged).
- [ ] Implement, test.
- [ ] Commit `feat(explain): open the index builder from the query bar verdict`.

### Slice 1 verification

Run the Verification section steps 1-5 and 8-9. PR title `feat(indexes): create the suggested index from a missed query (#116, part 3)`.

---

## Slice 2: field order (branch `feat/116-field-order`)

### T5: the pure field-order check (core)

**Files:**
- create `crates/core/src/explain/field_order.rs`;
- create `crates/core/src/explain/field_order_tests.rs`;
- modify `crates/core/src/explain/mod.rs` (`mod` and `pub(crate) use`).

**Produces:**
```rust
/// What is wrong with the order of the used index's keys for this query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldOrderFinding {
    /// R1: `range` comes before the equality field `field` among the used keys.
    RangeBeforeEquality { range: String, field: String },
    /// R2: `range` comes before the sort field `field`.
    RangeBeforeSort { range: String, field: String },
}
/// `keys`: the index key pattern in order. `None` when every used key is in E, S, R order, when
/// a key value is not 1/-1, or when there is nothing to judge. Only the keys from the start up
/// to the first one the query does not use are looked at. Top-level filter fields only;
/// `is_equality`/`is_range` classify them.
pub(crate) fn check(keys: &Document, filter: &Document, sort: Option<&Document>) -> Option<FieldOrderFinding>;
/// The field names of a profiler plan summary with exactly one `IXSCAN { .. }`; `None` for
/// zero or several scans or a name containing `,` `:` `{` `}`.
pub(crate) fn summary_keys(plan_summary: &str) -> Option<Document>;
```

- [ ] Failing tests:
  - `range_before_equality_is_found` (`{createdAt:1, status:1}` with `{status: "a", createdAt: {$gt: ..}}`);
  - `esr_order_is_fine`;
  - `range_before_sort_is_found`;
  - `in_counts_as_equality`;
  - `ne_and_regex_count_as_range`;
  - `keys_after_the_first_unused_key_are_not_judged`;
  - `hashed_text_and_2dsphere_keys_are_not_judged`;
  - `a_single_key_index_is_never_wrong`;
  - `an_empty_filter_with_a_sort_only_index_is_fine`;
  - `summary_keys_reads_one_ixscan`;
  - `summary_keys_refuses_two_scans_and_odd_names`;
  - `summary_keys_keeps_direction`.
- [ ] Implement (≤ 200 lines), test.
- [ ] Commit `feat(explain): judge the field order of the index a query uses`.

### T6: the `FieldOrder` verdict in explain and profiler (core)

**Files:**
- modify `crates/core/src/explain/verdict.rs` (`VerdictKind::FieldOrder`, `ALL: [_; 8]`, `index` 7, `severity` 2, `code` `"field_order"`);
- modify `crates/core/src/explain/classify.rs`:
  - `walk` also returns the first index scan's `keyPattern` and the number of index scans; the signature is internal, so its two callers are updated;
  - precedence per the spec;
  - reasons `field_order` / `field_order_sort` with params `index`, `range`, `field`;
  - `wants_suggestion` includes `FieldOrder`;
- modify `crates/core/src/history/shapes.rs` (`is_miss` includes `FieldOrder`);
- modify `crates/core/src/profile/classify.rs` (`classify_entry(entry, shape: &Shape)`; the `has_filter` arg is derived from `shape.has_filter()`);
- modify `crates/core/src/runtime/profiler_fold.rs` (pass the shape it already has);
- create `crates/core/src/explain/classify_field_order_tests.rs` and `crates/core/src/profile/classify_field_order_tests.rs`;
- add fixture `crates/core/src/explain/fixtures/real_field_order.json`, captured from a throwaway mongod and scrubbed like the existing `real_*.json` (no host, port, git version or db name).

- [ ] Failing tests:
  - `range_before_equality_index_is_field_order_on_small_data` (fixture: 10 docs, ratio not tripped);
  - `field_order_and_inefficient_show_both_reasons`;
  - `sort_after_range_stays_sort_not_covered_with_field_order_reason`;
  - `collscan_still_wins_over_field_order`;
  - `esr_index_is_index_used` (`real_ixscan.json` unchanged);
  - `two_index_scans_are_not_judged` (`or_multi_ixscan.json`);
  - `plan_only_explain_is_judged_by_keys`;
  - `field_order_gets_the_esr_suggestion`;
  - `field_order_code_round_trips` (`from_code(FieldOrder.code())`);
  - `severity_sql_ranks_field_order_as_two` (`shape_schema`);
  - `field_order_is_a_miss`;
  - profiler: `profile_entry_with_range_first_index_is_field_order`;
  - profiler: `unparseable_summary_is_not_field_order`;
  - profiler: `count_and_aggregate_entries_follow_the_same_rule`.
- [ ] Implement, `cargo +1.87.0 test -p curlybrace-core`.
- [ ] **Commit boundary:** the new variant breaks the UI's exhaustive matches (`kind_presentation`, `kind_tone`, `row_title`), so T6 is not committed alone. T6 and T7 land as one commit, `feat(explain): a field order verdict for an index used in the wrong order`, after `cargo +1.87.0 test --workspace --locked` is green.

### T7: field order in the UI

**Files:**
- modify `crates/ui/src/viewmodel/verdict.rs` (`kind_presentation`: Warn, ⚠, `query.verdict.title_field_order`);
- modify `crates/ui/src/views/verdict_badge.rs` (`reason_text_key` maps `field_order`, `field_order_sort`);
- modify `crates/ui/locales/en.json`.

**i18n (under `query.verdict`, alphabetical):**
```json
"reason_field_order": "The index {index} puts {range}, a range, before {field}, an exact match. Exact matches should come first, then sort fields, then ranges.",
"reason_field_order_sort": "The index {index} puts {range}, a range, before the sort field {field}, so the server sorts in memory.",
"title_field_order": "Index field order",
```

- [ ] Failing tests:
  - viewmodel `field_order_badge_is_a_warning_with_reasons`;
  - kittest `accessibility_verdict.rs::field_order_badge_reads_icon_title_and_reason`;
  - `accessibility_queries.rs::field_order_row_title_and_suggestion`;
  - snapshot `queries_view_field_order_light` in `crates/ui/tests/snapshots.rs` (a builder in `crates/ui/tests/support/indexes_shots.rs`).
- [ ] Implement, regenerate the snapshot with `CURLYBRACE_SNAPSHOTS=1`.
- [ ] Commit together with T6 (see T6's commit boundary).

### T8: real server

**Files:**
- modify `crates/core/tests/local_explain.rs`: `field_order_on_a_range_first_index`, against a throwaway mongod with 10 docs and index `{createdAt: 1, status: 1}`, query `{status, createdAt: {$gt}}`;
- modify `crates/core/tests/local_profiler.rs`: `profiler_entry_judged_field_order`.

- [ ] Run against a throwaway mongod (recipe above) three times.
- [ ] Commit `test(explain): field order against a real server`.

### Slice 2 verification

Run all of the Verification section. PR `feat(indexes): catch an index used in the wrong field order (#116, part 4)`.

---

## Slice 3: what gets recorded (branch `feat/116-recording`, closes #116)

### T9: record Aggregate tab explains as typed shapes (core)

**Files:**
- modify `crates/core/src/runtime/typed_record.rs`: `record_typed` keeps its signature and becomes a thin wrapper over the new `record_typed_shape`, so the `handlers.rs` call site is unchanged;
- create `crates/core/src/runtime/aggregate_record.rs` (+ `aggregate_record_tests.rs`);
- modify `crates/core/src/runtime/aggregate_explain.rs` (`run` gains `history: Option<&Arc<HistoryDb>>`; on `Ok` it parses and records);
- modify `crates/core/src/runtime/handlers.rs` line 442 in place (pass `history.as_ref()`; net 0 lines).

**Produces:**
```rust
// typed_record.rs
pub(super) fn record_typed_shape(history: &Arc<HistoryDb>, connection: SessionId, ns: &Namespace,
    shape: Shape, kind: VerdictKind, now: &dyn Fn() -> i64);
// aggregate_record.rs
/// The verdict of a pipeline's input plan, judged like a profiled aggregate: CollectionScan,
/// SortNotCovered (a SORT in the input plan), FieldOrder (T5 on the input index's keyPattern),
/// EmptyCollection, NoFilter, else IndexUsed; never InefficientIndex (like the profiler). `None`
/// for a sharded plan, no input plan or `NotUnderstood`.
pub(super) fn pipeline_verdict(plan: &AggPlanOutcome, shape: &Shape) -> Option<VerdictKind>;
/// Records the explained pipeline's shape (`shape::from_profile_command("command", {aggregate, pipeline})`
/// equivalent: leading `$match` plus `$sort`) unless the options carry a hint or a collation.
pub(super) fn record_pipeline(history: &Arc<HistoryDb>, session: SessionId, ns: &Namespace,
    stages: &[Document], options: &PipelineOptions, explain: &Document, now: &dyn Fn() -> i64);
```
`InputPlan` gains `key_pattern: Option<Document>`, read in `explain/agg_input.rs` the same way T6 reads it for find.

- [ ] Failing tests:
  - `an_explained_match_pipeline_is_recorded_as_typed`;
  - `the_pipeline_shape_equals_the_profiler_shape_of_the_same_pipeline` (same hash as `from_profile_command`);
  - `a_pipeline_not_starting_with_match_is_not_recorded` (only a leading `$match` is recorded; a `$sort` alone gives no shape);
  - `a_hint_or_collation_records_nothing`;
  - `a_failed_explain_records_nothing`;
  - `a_sharded_plan_records_nothing`;
  - `collscan_input_is_a_collection_scan`;
  - `sort_in_input_plan_is_sort_not_covered`;
  - `recording_never_delays_the_event` (a busy `HistoryDb` lock still answers `AggregateExplained`);
  - `nothing_about_the_pipeline_is_logged` (captured logger: no literal from the stages).
- [ ] Implement, test.
- [ ] Commit `feat(queries): record pipelines explained in the Aggregate tab`.

### T10: operator allowlist and cleanup of old rows (core)

**Files:**
- modify `crates/core/src/shape/normalize.rs` (`is_operator_doc` checks `QUERY_OPERATORS`);
- create `crates/core/src/shape/operators.rs` (the list, `pub(crate) const QUERY_OPERATORS: [&str; N]`);
- create `crates/core/src/shape/allowlist_tests.rs`;
- create `crates/core/src/history/shape_cleanup.rs` (+ `shape_cleanup_tests.rs`);
- modify `crates/core/src/history/db_schema.rs` (`prepare` calls `shape_cleanup::run_once(conn)` after `ensure_profile_mark`; `SCHEMA_VERSION` stays 3).

**Produces:**
```rust
/// Deletes `shape` rows (buckets cascade) whose `shape_json` `Shape::from_json` rejects, once
/// per file: a `shape_cleanup (rule INTEGER PRIMARY KEY)` marker row for rule 1 is written in
/// the same IMMEDIATE transaction. Logs the count, never the JSON.
pub(super) fn run_once(conn: &mut Connection) -> Result<(), SchemaError>;
```

- [ ] Failing tests:
  - `unknown_dollar_keys_collapse_to_a_placeholder` (`{a: {$secret: 1}}` gives `{"a":"?"}`);
  - `known_operators_are_kept`;
  - `a_mixed_known_and_unknown_operator_document_collapses`;
  - `elem_match_and_not_still_recurse`;
  - `from_json_rejects_the_old_form_with_an_unknown_operator`;
  - `existing_normalize_tests_still_pass` (the whole `normalize_tests.rs` unchanged);
  - `cleanup_deletes_invalid_shapes_and_their_buckets`;
  - `cleanup_keeps_valid_shapes`;
  - `cleanup_runs_once_per_file` (a second open does not scan: the marker is present);
  - `cleanup_leaves_user_version_at_3`;
  - `cleanup_on_a_v1_file_migrates_then_runs`.
- [ ] Implement, test.
- [ ] Commit `fix(shape): keep only known query operators and clean up older shapes`.

### T11: a profiler mark ahead of the server clock (core)

**Files:**
- modify `crates/core/src/store.rs` (`ProfileRead.server_now_millis: Option<i64>`, documented);
- modify `crates/core/src/mongo_profile.rs` (read `localTime` from the `hello` reply it already has; `hello_now(reply) -> Option<i64>`);
- modify `crates/core/src/test_support/memory_store.rs` (the new field goes on an existing line of the literal, so the file does not grow past 910 lines);
- the other `ProfileRead { .. }` literals (8 in all; add the field);
- create `crates/core/src/runtime/profiler/mark_clock.rs` (+ `mark_clock_tests.rs`);
- modify `crates/core/src/runtime/profiler/pass.rs` (one call before the read is used; mechanical).

**Produces:**
```rust
pub(in super::super) const MARK_AHEAD_TOLERANCE_MS: i64 = 60_000;
/// The mark to read from: `mark` itself, or the server's now (unanchored) when `mark` is more
/// than the tolerance ahead of it. `None` server time keeps the mark.
pub(in super::super) fn clamp_ahead(mark_millis: i64, server_now_millis: Option<i64>) -> (i64, bool /* clamped */);
```

- [ ] Failing tests:
  - `a_mark_within_tolerance_is_kept`;
  - `a_mark_far_ahead_is_moved_to_server_now`;
  - `no_server_time_keeps_the_mark`;
  - `a_clamped_mark_reports_no_missed_entries`;
  - `the_clamp_is_persisted_with_the_next_page`;
  - `hello_now_reads_local_time`;
  - `read_only_store_forwards_server_now`.
- [ ] Implement, test.
- [ ] Commit `fix(profile): move a watermark that is ahead of the server clock back`.

### T12: real server and close-out

**Files:**
- modify `crates/core/tests/local_profiler_poller.rs`: `aggregate_explain_is_recorded_as_typed` (throwaway mongod: Explain a `$match`+`$sort` pipeline through the runtime, read `QueryShapes`, one typed row with the profiler's hash);
- modify `crates/core/tests/local_profiler.rs`: `hello_reports_server_now`.

- [ ] Run against a throwaway mongod three times.
- [ ] Commit `test(profile): aggregate shapes and server time on a real server`.

### Slice 3 verification

Run all of the Verification section. PR `feat(indexes): record pipelines, tighten shapes and guard the profiler mark (closes #116)`. In the body:
- note that #118's docker_e2e "Connect" item is already fixed on `main`;
- list the owner questions from the spec.

---

## i18n summary (all new keys)

| Slice | Keys |
|---|---|
| 1 | `indexes.queries.{create_index, create_index_n, create_index_read_only, last_run}`, `query.verdict.{create_index, create_index_read_only}` |
| 2 | `query.verdict.{reason_field_order, reason_field_order_sort, title_field_order}` |
| 3 | none |

Reused: `indexes.ago_*`, `query.verdict.{suggestion, copy}`, `indexes.queries.{copy_suggestion, copy_shape}`.

## Verification (per slice, in order, all with `cargo +1.87.0`)

1. Per task: the failing tests seen red, then green; `wc -l` of every touched file (none created over 400; the over-800 files did not grow).
2. `cargo +1.87.0 fmt --all --check`.
3. `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`.
4. `cargo +1.87.0 test --workspace --locked` (unit, kittest, `i18n_usage_tests`, en.json order and unused-key tests).
5. Coverage of the new modules at least 80%: `cargo +1.87.0 llvm-cov --workspace --locked --summary-only`; list any new branch left untested in the PR body.
6. Real server (slices 2 and 3), throwaway `mongod` only, never an existing one: `curlybrace-core --test local_explain --test local_profiler --test local_profiler_poller --test local_aggregate_explain -- --ignored`; stop by pid, delete the dbpath. New test files go into `scripts/integration-tests.sh`.
7. Snapshots: `CURLYBRACE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots -- --include-ignored` passes; only the intended PNGs changed (`git status crates/ui/tests/snapshots`); no `*.new.png`.
8. Privacy grep over the new modules: no `info!`/`warn!`/`error!`/`debug!` formats a filter, pipeline, shape or option value.
9. Run the app (`run` skill) against the throwaway server at 1000x700 and 800x500:
   - Queries rows show Last run and Create index..., which opens the builder pre-filled;
   - read-only: off with the reason;
   - Browse verdict popup: Create index...;
   - (slice 2) a range-first index shows "Index field order".
   Report anything not checked on screen.

## Calls made here (the spec's list; the defaults are implemented)

1. Three slices; slice 3 closes #116.
2. `FieldOrder` is a persisted verdict, judged structurally and conservatively.
3. Create index... reuses `from_suggested` and `open_with`; not offered in the guard notices.
4. Aggregate explains are recorded; previews are not explained.
5. Operator allowlist plus a run-once cleanup without a schema version bump.
6. A mark ahead of the server clock is clamped using the existing `hello`.
7. Ranking unchanged; "Last run" from the newest bucket.
