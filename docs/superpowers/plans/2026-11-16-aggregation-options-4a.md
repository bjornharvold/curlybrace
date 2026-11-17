# Aggregate options, core (slice 4a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** The core of aggregate options: a validated `PipelineOptions` type, the one policy that decides what each operation sends, the driver mapping, the store and runtime plumbing for preview, count, explain and the confirmed run, typed errors for a missing hint, a refused collation and an unsatisfiable read preference, and container tests. No visible UI change: the UI sends `PipelineOptions::default()`, which sends exactly what is sent today, except that the confirmed run now pins the primary.

**Architecture:** Pure core types in `pipeline_options.rs` (+ policy in `pipeline_options_apply.rs`), driver mapping and error mapping in `mongo_pipeline_options.rs`, the three store methods take `&AppliedOptions` in place of `max_time`, the four aggregate commands carry `options: PipelineOptions` and their handlers call `options.applied(Operation::..)`. The preview handler moves out of `runtime/handlers.rs` into `runtime/aggregate_preview.rs` so that over-800 file shrinks.

**Tech Stack:** Rust 1.87 (no let-chains), `bson`, `mongodb` 3.x (`AggregateOptions`, `Collation`, `Hint`, `SelectionCriteria`, `ReadPreference`). No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-16-aggregation-slice-4-design.md` ("The options", "Which operation sends what", "Safety", "Error mapping", 4a). Model on `pipeline_count.rs` (a small pure module with its own tests file), `mongo_pipeline.rs` (store transport and its unit tests), `runtime/aggregate_count.rs` (a handler in its own file), `test_support/memory_aggregate.rs` (what the fake records).

## Global Constraints

- Files 200-400 lines (800 max). Do **not** grow `runtime/mod.rs` (1050), `test_support/memory_store.rs` (927), `mongo_store.rs` (846) or `runtime/handlers.rs` (814) beyond the signature edits; new logic goes into new modules. Tests in sibling `*_tests.rs` files as the crate does.
- Immutable style (builders and `..self`, no `&mut` setters on the options); real objects over mocks; TDD (write the failing test, run it, implement, run it); coverage of new code at least 80%.
- Option values are user data: never in a log line or an error message; errors carry variants only. The comment is sent to the server by design.
- i18n: 4a adds only `errors.hint_not_found`, `errors.collation_rejected`, `errors.read_preference_unavailable` to `crates/ui/locales/en.json` (nested under the existing `errors` object). Report them.
- `cargo +1.87.0`; run git as plain separate Bash calls; conventional commits (`feat(aggregate): ...`) ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never touch `CHANGELOG.md`.
- NEVER connect to any MongoDB already on the machine (the user has others, incl. 27017). Integration tests run only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted afterwards (Task 5).

## Review Focus

- `PipelineOptions::default().applied(op)` sends byte-for-byte what slices 1-3 send for preview, count and explain (existing tests stay green unchanged apart from the signature); only the run gains an explicit primary.
- Preview and count: `allowDiskUse` is `false` whatever the options say, forced in the store too. Run: primary whatever the options and the connection say, forced in the store too.
- maxTimeMS only lowers: `min(set, default)` per operation; `0` and values above 60 000 are refused by validation.
- Error mapping fires only when the matching option was sent (a `BadValue` without a hint stays `Database`), and a server-selection failure with a non-primary read preference is **not** a connection error (no ping, no "server lost").
- Serde: MongoDB spellings, unknown keys ignored, invalid known values refused, `is_empty` drives `skip_serializing_if` (4b relies on it).

### Task 1: the options type and validation

**Files:** create `crates/core/src/pipeline_options.rs`, `crates/core/src/pipeline_options_tests.rs`; modify `crates/core/src/lib.rs` (`pub mod pipeline_options;`), `crates/core/src/pipeline.rs` (`pub use crate::pipeline_options::{...}` for the public names).

**Produces:**
```rust
pub const MAX_TIME_MS_CAP: u64 = 60_000;
pub const MAX_LOCALE_CHARS: usize = 32;
pub const MAX_HINT_NAME_CHARS: usize = 256;
pub const MAX_COMMENT_CHARS: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawOptions", rename_all = "camelCase")]
pub struct PipelineOptions {
    pub collation: Option<CollationOption>,
    pub hint: Option<HintOption>,
    pub allow_disk_use: Option<bool>,
    #[serde(rename = "maxTimeMS")] pub max_time_ms: Option<u64>,
    pub comment: Option<String>,
    pub read_preference: Option<ReadPreferenceMode>,
}
pub struct CollationOption { pub locale: String, pub strength: Option<Strength>, pub case_level: Option<bool>,
    pub case_first: Option<CaseFirst>, pub numeric_ordering: Option<bool>, pub alternate: Option<Alternate> }
pub enum Strength { Primary = 1, Secondary, Tertiary, Quaternary, Identical }   // serde as the integer 1..=5
pub enum CaseFirst { Upper, Lower, Off }                                          // "upper" | "lower" | "off"
pub enum Alternate { NonIgnorable, Shifted }                                      // "non-ignorable" | "shifted"
pub enum HintOption { Name(String), Keys(Document) }                              // {"name": ".."} | {"keys": {..}}
pub enum ReadPreferenceMode { Primary, PrimaryPreferred, Secondary, SecondaryPreferred, Nearest } // camelCase strings
pub enum OptionsError { LocaleEmpty, LocaleInvalid, HintEmpty, HintNameTooLong, HintKeysInvalid, HintJson,
    MaxTimeRange, CommentEmpty, CommentTooLong, CommentControl }

impl OptionsError { pub fn error_variant(self) -> &'static str }  // snake_case, matches aggregate.options.problem.* (4b)
impl PipelineOptions {
    pub fn validated(self) -> Result<Self, OptionsError>;  // trims locale, comment and hint name; checks every rule
    pub fn is_empty(&self) -> bool;                        // nothing set (== default)
    pub fn set_count(&self) -> usize;                      // for the 4b indicator: each set top-level option counts 1
}
impl HintOption { pub fn parse(text: &str) -> Result<HintOption, OptionsError>; } // `{`-prefixed => keys via ejson::editor_json_to_doc, else a name
```
Rules: locale `simple` or `^[A-Za-z]{2,8}(_[A-Za-z0-9]{2,8})*(@collation=[a-z]+)?$` checked by hand (no regex crate), 1..=32 chars; hint keys non-empty, each value an int/long/double equal to 1 or -1 or a non-empty string, `$natural` allowed only as the sole key; maxTimeMS 1..=60 000; comment 1..=256 chars, no control characters. `RawOptions` mirrors the struct and `TryFrom<RawOptions>` calls `validated`, so a file with an invalid value fails to deserialize; unknown keys are ignored (no `deny_unknown_fields`).

- [ ] Failing tests in `pipeline_options_tests.rs`: `the_default_is_empty_and_counts_nothing`, `set_count_counts_each_top_level_option_once`, `a_valid_locale_passes_and_is_trimmed`, `simple_and_variant_locales_pass` (`simple`, `de@collation=phonebook`, `zh_Hant`), `bad_locales_are_refused` (empty, `en US`, 33 chars, `../x`), `hint_parse_reads_a_name_or_a_key_pattern`, `hint_keys_must_be_directions_or_index_types` (`{}`, `{"a": 2}`, `{"a": true}`, `{"$natural": 1, "a": 1}` refused; `{"a": 1, "b": -1}`, `{"t": "text"}`, `{"$natural": -1}` pass), `a_hint_that_is_not_json_is_hint_json`, `max_time_bounds` (0 and 60 001 refused, 1 and 60 000 pass), `comments_are_bounded_and_printable`, `serde_uses_mongodb_names` (exact JSON of a fully set value: `collation.locale`, `strength: 2`, `caseFirst: "upper"`, `alternate: "shifted"`, `hint.keys`, `allowDiskUse`, `maxTimeMS`, `comment`, `readPreference: "secondaryPreferred"`), `serde_round_trips_every_field`, `unknown_keys_are_ignored_on_load`, `an_invalid_known_value_fails_to_load` (strength 9, `readPreference: "tagged"`, `maxTimeMS: 0`), `error_variants_are_stable_and_content_free`.
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core pipeline_options`; commit `feat(aggregate): add validated pipeline options`.

### Task 2: the policy per operation

**Files:** create `crates/core/src/pipeline_options_apply.rs`, `crates/core/src/pipeline_options_apply_tests.rs`; modify `crates/core/src/lib.rs`, `crates/core/src/pipeline.rs` (re-exports).

**Produces:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation { Preview, Count, Explain, Run }

/// What one operation sends. Built only by `PipelineOptions::applied` or `AppliedOptions::bounded`.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedOptions {
    pub max_time: Duration,
    pub collation: Option<CollationOption>,
    pub hint: Option<HintOption>,
    pub allow_disk_use: Option<bool>,       // Preview/Count: always Some(false)
    pub comment: Option<String>,
    pub read_preference: Option<ReadPreferenceMode>, // Run: always None (the store pins the primary)
}
impl Operation { pub fn default_max_time_ms(self) -> u64 } // PREVIEW_MAX_TIME_MS, PREVIEW_MAX_TIME_MS, AGGREGATE_EXPLAIN_MAX_TIME_MS, AGGREGATE_RUN_MAX_TIME_MS
impl PipelineOptions { pub fn applied(&self, op: Operation) -> AppliedOptions; }
impl AppliedOptions {
    pub fn bounded(op: Operation, max_time: Duration) -> Self; // no user option; for tests and callers with their own bound
    pub fn max_time_lowered(&self, op: Operation) -> bool;     // the person's value is below the default (4b status wording)
}
```

- [ ] Failing tests: `the_default_applies_today_s_behaviour_to_every_operation` (preview/count: 10 s, `Some(false)` disk use, nothing else; explain: 10 s, `None` disk use; run: 60 s), `preview_and_count_never_allow_disk_use`, `explain_and_run_carry_disk_use_as_set`, `max_time_only_lowers` (5 000 lowers all; 30 000 lowers only the run; 60 000 changes nothing), `the_run_never_carries_a_read_preference`, `reads_carry_the_read_preference_as_set`, `collation_hint_and_comment_reach_every_operation`, `max_time_lowered_reports_only_a_real_lowering`.
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core pipeline_options_apply`; commit `feat(aggregate): decide which options each operation sends`.

### Task 3: driver mapping, typed errors, store signatures

**Files:** create `crates/core/src/mongo_pipeline_options.rs`, `crates/core/src/mongo_pipeline_options_tests.rs`; modify `crates/core/src/mongo_pipeline.rs` (use the mapping; `preview_options`/`run_options` replaced; `aggregate_explain_command(ns, pipeline, verbosity, &AppliedOptions)`), `crates/core/src/store.rs` (trait signatures + docs), `crates/core/src/mongo_store.rs`, `crates/core/src/read_only_store.rs` (+ `read_only_store_tests.rs`), `crates/core/src/test_support/memory_store.rs` (signatures only), `crates/core/src/test_support/memory_aggregate.rs` (+ `memory_aggregate_tests.rs`), `crates/core/src/error.rs`, `crates/core/src/error_name.rs`, `crates/ui/locales/en.json`, and every caller of the three methods (`explain/agg_plan.rs`, `explain/agg_plan_tests.rs`, `explain/agg_hint_tests.rs`, `crates/ui/src/viewmodel/aggregate_explain.rs`, `crates/ui/src/views/aggregate_plan_tests.rs`, `crates/core/tests/local_aggregate*.rs`): `max_time` becomes `&AppliedOptions::bounded(Operation::.., max_time)`.

**Produces:**
```rust
// store.rs (DocumentStore)
async fn aggregate_preview(&self, ns: &Namespace, pipeline: Vec<Document>, options: &AppliedOptions) -> Result<Vec<Document>>;
async fn aggregate_explain(&self, ns: &Namespace, pipeline: Vec<Document>, verbosity: ExplainVerbosity, options: &AppliedOptions) -> Result<Explain>;
async fn aggregate_write(&self, ns: &Namespace, pipeline: Vec<Document>, options: &AppliedOptions) -> Result<()>;

// mongo_pipeline_options.rs
pub(crate) fn driver_collation(c: &CollationOption) -> mongodb::options::Collation;
pub(crate) fn driver_hint(h: &HintOption) -> mongodb::options::Hint;
pub(crate) fn read_criteria(o: &AppliedOptions) -> Option<SelectionCriteria>;   // None = the database's own
pub(crate) fn read_options(o: &AppliedOptions) -> AggregateOptions;               // allow_disk_use forced Some(false)
pub(crate) fn write_options(o: &AppliedOptions) -> AggregateOptions;              // selection_criteria forced Primary
pub(crate) fn explain_fields(o: &AppliedOptions) -> Document;                     // collation, hint, allowDiskUse, comment for the inner `aggregate`
pub(crate) fn classify_with(e: &mongodb::error::Error, o: &AppliedOptions) -> CoreError;

// error.rs
CoreError::HintNotFound            // "errors.hint_not_found",            error_name "HintNotFound"
CoreError::CollationRejected       // "errors.collation_rejected",        error_name "CollationRejected"
CoreError::ReadPreferenceUnavailable // "errors.read_preference_unavailable", error_name "ReadPreferenceUnavailable"

// memory_aggregate.rs: record what each call received
pub fn received_options(&self) -> Vec<(Operation, AppliedOptions)>; // preview, explain and write calls, in order (refused calls absent)
```
`classify_with`: a `Command` error with code 2 and the hint set and the server text containing `hint` (case-insensitive) is `HintNotFound`; code 2 with the collation set and text containing `collation` or `locale` is `CollationRejected`; `ErrorKind::ServerSelection` with a read preference other than `primary` set is `ReadPreferenceUnavailable` (checked **before** `classify_driver_error`, so it is never `Unreachable`); everything else `classify_driver_error`. The text is matched, never stored. The explain path passes `read_criteria(o).or_else(|| explain_criteria(db))` to `run_command`.

- [ ] Failing tests in `mongo_pipeline_options_tests.rs`: `read_options_never_allow_disk_use` (even with `Some(true)` applied), `read_options_carry_time_collation_hint_comment_and_read_preference`, `write_options_always_select_the_primary` (with `Secondary` applied and with none), `write_options_carry_disk_use_as_applied`, `driver_collation_maps_every_field`, `driver_hint_maps_name_and_keys`, `explain_fields_are_empty_for_the_default` (keeps `the_explain_command_wraps_an_aggregate_with_verbosity_and_time_limit` passing unchanged), `explain_command_carries_the_options_inside_aggregate`, `classify_with_maps_a_missing_hint_only_when_a_hint_was_sent`, `classify_with_maps_a_refused_collation_only_when_one_was_sent`, `classify_with_maps_server_selection_only_for_a_non_primary_read_preference`, `classified_errors_hold_no_server_text`. Command errors built like `driver_error.rs`'s `command_error(code)` test helper (extend it with a message if needed, in the tests file). In `memory_aggregate_tests.rs`: `the_store_records_the_options_each_call_received`, `a_refused_pipeline_records_no_options`. In `read_only_store_tests.rs`: the existing pass-through and refusal tests updated, plus `options_pass_through_unchanged`. `error.rs` tests: the three keys exist in `en.json` (the existing all-keys test covers it; run it).
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core` and `cargo +1.87.0 test -p curlybrace-ui` (callers compile); commit `feat(aggregate): send aggregate options to the server with typed errors`.

### Task 4: commands and handlers

**Files:** create `crates/core/src/runtime/aggregate_preview.rs` (the `Command::AggregatePreview` arm moved out of `handlers.rs`, like `aggregate_count.rs`); modify `crates/core/src/runtime/mod.rs` (`options: PipelineOptions` on `AggregatePreview`, `AggregateExplain`, `AggregateCount`, `AggregateRun`; `mod aggregate_preview;`), `runtime/handlers.rs` (the preview arm becomes a call; arms pass `options`), `runtime/aggregate_explain.rs`, `runtime/aggregate_count.rs`, `runtime/aggregate_run.rs` (each takes `&PipelineOptions` and calls `applied(Operation::..)` for its bound, replacing the local `max_time`), their `*_tests.rs`, `runtime/aggregate_tests.rs`, `runtime/read_only_tests.rs`, and the four UI constructors (`crates/ui/src/app/aggregate_actions.rs:253`, `aggregate_explain_actions.rs:61`, `aggregate_counts_actions.rs:70`, `aggregate_run_actions.rs:128`) passing `options: PipelineOptions::default()` with a `// 4b: the draft's options` note.

**Produces:**
```rust
// runtime/aggregate_preview.rs
pub(super) async fn run(session: SessionId, store: &dyn DocumentStore, request: u64, ns: Namespace,
    pipeline: Vec<Document>, options: &PipelineOptions) -> Result<Event>;
// aggregate_explain::run, aggregate_count::run, aggregate_run::run gain `options: &PipelineOptions` (last parameter)
```
`Command::is_write`, `context`, request numbering, client bounds (`command_time_limit`) and `timeout_answer`s are unchanged.

- [ ] Failing tests: `aggregate_tests.rs` `a_preview_sends_the_options_it_was_given` (in-memory `received_options` shows `Operation::Preview` with the collation, hint and lowered time) and `the_default_options_preview_as_before`; `aggregate_count_tests.rs` `a_count_sends_the_options_without_disk_use`; `aggregate_explain_tests.rs` `an_explain_sends_disk_use_and_read_preference_as_set`; `aggregate_run_tests.rs` `a_run_never_sends_the_read_preference` and `a_run_carries_collation_hint_and_a_lower_time`; `read_only_tests.rs` `a_read_only_session_previews_with_options` and `a_read_only_session_still_refuses_a_run_with_options`; a handler test that `ReadPreferenceUnavailable` comes back as the preview's outcome, not as `Event::Failed`/a ping.
- [ ] Implement; `wc -l crates/core/src/runtime/handlers.rs crates/core/src/runtime/mod.rs` (handlers shrinks, mod grows by the four fields only); `cargo +1.87.0 test --workspace --locked`; commit `feat(aggregate): carry pipeline options on the aggregate commands`.

### Task 5: container tests, then the full check

**Files:** create `crates/core/tests/local_aggregate_options.rs` (`mod support;`, `#[ignore]` + `local_enabled`, throwaway `cbit_*` databases through `local_mongo()`, as `local_aggregate.rs`).

Tests: `collation_strength_two_matches_and_sorts_ignoring_case` (`$match {name: "abc"}` finds `ABC` and `abc`; `$sort` order under `en` strength 2), `numeric_ordering_sorts_digit_strings_as_numbers`, `an_invalid_locale_is_collation_rejected`, `a_hint_by_name_and_by_keys_is_the_plan_s_index` (explain `queryPlanner` with a hint on the non-obvious index shows that `indexName`; `parse_aggregate_explain` names it), `a_hint_on_a_missing_index_is_hint_not_found` (preview, count pipeline and explain each answer `CoreError::HintNotFound`), `a_lowered_max_time_expires` (the self cross join from `max_time_expiry_is_the_timeout_error` with `maxTimeMS: 50` through `PipelineOptions::applied` answers `Database("timeout")`), `the_comment_reaches_the_profiler` (profiler level 2 on the throwaway db, the `system.profile` entry's `command.comment` equals the comment), `disk_use_is_false_on_preview_even_when_set` (memory-heavy `$group` with `allowDiskUse: true` in options: the preview's `system.profile` entry has `allowDiskUse: false`), `a_run_with_collation_and_hint_writes_the_expected_target`, `secondary_preferred_reads_on_a_one_member_set` and `secondary_on_a_one_member_set_is_read_preference_unavailable` (both gated with `replica_set_or_skip`; the second builds its store from the URI plus `serverSelectionTimeoutMS=2000`).

Throwaway server (two runs: standalone, then one-member replica set), from the worktree:
```bash
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
DBPATH=$(mktemp -d /private/tmp/claude-501/cbit-4a.XXXXXX)
/opt/homebrew/bin/mongod --dbpath "$DBPATH" --bind_ip 127.0.0.1 --port "$PORT" --fork --logpath "$DBPATH/log" --pidfilepath "$DBPATH/pid"
CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" cargo +1.87.0 test -p curlybrace-core --test local_aggregate_options --test local_aggregate --test local_aggregate_explain --test local_aggregate_count --test local_aggregate_run -- --ignored
kill "$(cat "$DBPATH/pid")"; rm -rf "$DBPATH"
# again with: mongod ... --replSet cbit4a, then
/opt/homebrew/bin/mongosh --quiet "mongodb://127.0.0.1:$PORT/?directConnection=true" --eval 'rs.initiate({_id:"cbit4a",members:[{_id:0,host:"127.0.0.1:'"$PORT"'"}]})'
# wait for PRIMARY, run the same tests with CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT/?replicaSet=cbit4a", stop by pid, delete
```

- [ ] Write the tests, run them against both throwaway servers (all pass, the replica-set ones not skipped on the second run), stop and delete each server by its pid.
- [ ] Full check: `cargo +1.87.0 fmt --all`, `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`, `cargo +1.87.0 test --workspace --locked`; coverage of the new modules at least 80% (`cargo llvm-cov` if installed, else list the tests per public function in the PR body).
- [ ] Commit `test(aggregate): container tests for aggregate options`. Report the three new `errors.*` keys and the file sizes of every touched over-800 file (before and after).
