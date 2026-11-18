# Browse find query options (#75) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** Collation, hint, a lower `maxTimeMS`, read preference, `comment` and `allowDiskUse` on the Browse find: a **Query options** section behind **More options**, sent by Find and kept by paging and every re-fetch, applied to the automatic verdict and the guard check, carried by Copy as code in nine languages, typed errors inline, and a line saying which options shaped the shown page. No query history (#72 is not built; the spec rules what "saved" means now).

**Architecture:** `Query` gains `options: QueryOptions` (`= PipelineOptions`). The store applies one pure policy (`QueryOptions::find_applied(FindOperation)` -> `AppliedFindOptions`, wrapping 4a's `AppliedOptions`) inside `find`, `explain_find` and `explain_plan`, and classifies driver errors with 4a's `classify_with`. No command, event or store signature changes. UI: `FindBarState` holds an `OptionsForm` (4b) and its open choice; `submit_find` attaches the parsed options; the form is drawn by a view shared with the Aggregate tab (extracted first, no behaviour change); the hint picker reads the session's existing aggregate hint list. Codegen: the find renderers take 4c's `CodeAggOptions`.

**Tech Stack:** Rust 1.87 (no let-chains), `bson`, `mongodb` 3.x (`FindOptions`, `Collation`, `Hint`, `SelectionCriteria`), egui/eframe 0.32, egui_kittest, serde. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-18-find-query-options-design.md`. Binding earlier decisions: `docs/superpowers/specs/2026-11-16-aggregation-slice-4-design.md` and plans `2026-11-16-aggregation-options-4a.md`, `2026-11-17-aggregation-options-4b.md`, `2026-11-18-aggregation-options-4c.md` (the default sends today's bytes; maxTimeMS only lowers; option values never logged; typed errors only when the matching option was sent).

**Reuse points (do not duplicate):** core `pipeline_options.rs` (type, validation, serde, value-free `Debug`, `HintOption::parse`), `pipeline_options_apply.rs` (`AppliedOptions`, `MaxTimeEffect`, `clamped_time`), `mongo_pipeline_options.rs` (`driver_collation`, `driver_hint`, `read_criteria`, `explain_fields`, `classify_with`, `asks_for_non_primary`), `pipeline_options_doc.rs`, codegen `agg_options.rs` (`CodeAggOptions::of`, `plain_entries`, `mode_words`, `strength_words`, `case_first_words`, `alternate_words`, `HintKeys`), UI `viewmodel/aggregate_options.rs` (`OptionsForm`, `OptionsEdit`, `OptionField`, `Tri`, `COMMON_LOCALES`, `problem_message`, `keys_text`), `aggregate_hint_list.rs` + `app/aggregate_hint_actions.rs` (`wanted`, `retry`), `aggregate_options_view.rs` (`options_status`, `read_preference_key`), views `aggregate_options.rs`, `aggregate_options_fields.rs`, `aggregate_options_hint.rs`, `code_export_options.rs`, `findbar_more.rs` (`more_settled` pattern).

## Global Constraints

- Files I create: 200-400 lines (never over 400). Over-800 files must not grow: `crates/core/src/runtime/mod.rs` (1057), `test_support/memory_store.rs` (926), `mongo_store.rs` (841: Task 2 moves `find` out first), `runtime/handlers.rs` (809: untouched), `crates/ui/src/app/bulk_actions.rs` (849: Task 6 moves `bulk_entry` out first), `app/mod.rs` (1007: `mod` lines only, +3 at most). `app/reducer.rs` (722) gets only the one-line delegation of Task 6. Over-400 files get no new tests, only mechanical edits: `pipeline_options.rs` (447), `viewmodel/aggregate_options.rs` (430), `viewmodel/aggregate_options_tests.rs` (478), `mongo_pipeline_options_tests.rs` (477), `codegen/tests.rs` (416), `app/panels.rs` (625). `wc -l` every touched file before each commit.
- Immutable style: ops return new values (`Self { .., ..self }`); the find bar view moves the state (`std::mem::take`) into a viewmodel op and stores the result, never mutates fields of the form. Real objects over mocks (`Query { options, ..Query::default() }`, `PipelineOptions { .. }`, `InMemoryStore` recording). TDD per step: failing test, run it (red), implement, run it (green). Coverage of new code at least 80%.
- i18n: every visible string, tooltip and accessible name is a key written as a **literal** (`i18n_usage_tests.rs`); `crates/ui/locales/en.json` nested and alphabetical at every level (`en_json_is_nested_and_alphabetical_at_every_level`); a key is added in the task whose source first names it. Generated code is not UI text. Full text in "en.json" below.
- Option values and query text are user data: never in a log line, an error message or a logged `Debug`; log option **names** or `OptionsError::error_variant()` only.
- `cargo +1.87.0` always; git as plain separate Bash calls; conventional commits ending with
  `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never touch `CHANGELOG.md`.
- **NEVER connect to any MongoDB already on the machine** (there are others, incl. 27017; `local_e2e_flows.rs` defaults to it, so the URI is **always** set). Real-server tests run only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted afterwards. Recipe (standalone, then `--replSet cbitfq` for read preference):
```bash
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
DBPATH=$(mktemp -d /private/tmp/claude-501/cbit-fq.XXXXXX)
/opt/homebrew/bin/mongod --dbpath "$DBPATH" --bind_ip 127.0.0.1 --port "$PORT" --fork --logpath "$DBPATH/log" --pidfilepath "$DBPATH/pid"
CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" cargo +1.87.0 test -p <crate> --test <file> -- --ignored
kill "$(cat "$DBPATH/pid")"; rm -rf "$DBPATH"
# replica set: add --replSet cbitfq, then
/opt/homebrew/bin/mongosh --quiet "mongodb://127.0.0.1:$PORT/?directConnection=true" --eval 'rs.initiate({_id:"cbitfq",members:[{_id:0,host:"127.0.0.1:'"$PORT"'"}]})'
# wait for PRIMARY; URI "mongodb://127.0.0.1:$PORT/?replicaSet=cbitfq"; stop by pid; delete
```

## Review Focus

- **Nothing set, nothing changes:** `Query::default().options` is empty and find, explain and plan check send byte-for-byte today's commands (no `maxTimeMS` on the find; 2000 ms on both explains; the connection's read preference); every existing test passes with only the literal edits of Task 1; every existing `find*.txt` golden is byte-identical.
- **What is sent is what was shown:** Find refuses an invalid form; the store derives every option from `q.options` through `find_applied`; `issued_query == query` and `explain_actions::same_run` compare the options too (`same_run` lists fields by hand today and would otherwise accept a verdict computed under other options); paging, re-fetch after insert/edit/delete/bulk/aggregate run keep the shown page's options.
- **maxTimeMS only lowers:** find min(set, 30 s) and none when unset; explain and plan check min(set, 2 s); `FIND_MAX_TIME_MS` equals `COMMAND_TIMEOUT` by test.
- **Typed errors only when the option was sent**, shown in the find bar (no banner, no lost-server ping); other find failures keep the banner.
- **Collation safety:** bulk update/delete matching and the whole-result copy are off while the shown page was found with a collation; hand-picked copies, edit, clone, delete by `_id` are not.
- **Shared form refactor is behaviour-neutral:** every aggregate options test, accessibility test and snapshot passes unchanged after Task 7.
- **Logging:** no option value, comment or query text in any log line; `Query`'s `Debug` prints options through `PipelineOptions`' value-free `Debug`.

### Task 1: options in the query and the find policy (core)

**Files:** create `crates/core/src/query_options.rs`, `crates/core/src/query_options_tests.rs`; modify `crates/core/src/lib.rs` (`pub mod query_options;`), `crates/core/src/store.rs` (`Query.options` + `Default`), `crates/core/src/query.rs` (`parse_query` literal: `options: QueryOptions::default()`), `crates/core/src/mongo_explain.rs` (`EXPLAIN_MAX_TIME_MS` now `FIND_EXPLAIN_MAX_TIME_MS as i64`, value unchanged), and the full `Query { .. }` literals: `crates/core/src/mongo_explain_tests.rs:11`, `shape/normalize_tests.rs:113`, `codegen/tests.rs:401`, `crates/core/tests/codegen_golden.rs:186`, `crates/ui/src/viewmodel/shell_queries_call.rs:215`, `doc_copy_tests.rs:30`, `app/bulk_test_support.rs:60`, `app/copy_documents_tests.rs:81` (add `..Query::default()` or `options: QueryOptions::default()`).

**Produces:**
```rust
// query_options.rs
/// The options of a Browse find: the same type, rules and serde as an aggregate's.
pub type QueryOptions = crate::pipeline_options::PipelineOptions;
/// The find's own bound: the client-side command timeout (a test pins it to COMMAND_TIMEOUT).
pub const FIND_MAX_TIME_MS: u64 = 30_000;
/// The server-side bound of the verdict's and the guard's explains (today's value).
pub const FIND_EXPLAIN_MAX_TIME_MS: u64 = 2_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindOperation { Find, Explain, PlanCheck }
impl FindOperation { pub fn default_max_time_ms(self) -> u64; }
/// What one find operation sends. Built only by `find_applied`.
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedFindOptions {
    pub(crate) options: crate::pipeline_options_apply::AppliedOptions,
    /// Whether a server-side time limit is sent (Find: only when the person set one).
    pub(crate) send_max_time: bool,
}
impl PipelineOptions {
    pub fn find_applied(&self, op: FindOperation) -> AppliedFindOptions;
    pub fn find_max_time_effect(&self, op: FindOperation) -> MaxTimeEffect;
}
impl AppliedFindOptions {
    pub fn is_default(&self) -> bool;           // nothing of the person's
    pub fn max_time_lowered(&self) -> bool;     // for the timeout wording
}
// store.rs
pub struct Query { .., /// As set; the store applies `find_applied`. Default: empty.
    pub options: crate::query_options::QueryOptions }
```
`find_applied`: collation, hint, comment, read preference, allowDiskUse as set (no `forbids_disk_use`); `max_time` = `min(set, default)` clamped to at least 1 ms, `send_max_time = op != Find || set.is_some()`.

- [ ] Failing tests in `query_options_tests.rs`: `the_default_query_has_no_options`, `the_default_applies_today_s_find` (Find: `send_max_time == false`, no collation/hint/comment/disk use/read preference; Explain and PlanCheck: 2000 ms sent, nothing else), `find_carries_every_option_as_set` (allowDiskUse `Some(true)` stays `Some(true)`, unlike 4a's preview), `max_time_only_lowers_per_operation` (500: lowers all three; 5000: lowers Find only, explain stays 2000; 45 000: Find capped to 30 000 and `find_max_time_effect == Capped`), `a_set_time_on_find_is_sent`, `the_find_bound_is_the_command_timeout` (`FIND_MAX_TIME_MS == COMMAND_TIMEOUT.as_millis()`), `the_explain_bound_is_today_s` (`FIND_EXPLAIN_MAX_TIME_MS as i64 == mongo_explain::EXPLAIN_MAX_TIME_MS`), `query_debug_prints_no_option_value` (a `Query` with comment `secret-comment`, hint `secret_idx`, locale `fr_CA`: `format!("{q:?}")` contains none of them), `next_and_prev_page_keep_the_options`.
- [ ] Implement; `cargo +1.87.0 test -p curlybrace-core query_options` and `cargo +1.87.0 test --workspace --locked` (only literal edits needed); commit `feat(query): carry query options in the find query`.

### Task 2: the store sends them, typed errors (core)

**Files:** create `crates/core/src/mongo_find.rs` (+ `mongo_find_tests.rs`), `crates/core/src/test_support/memory_find.rs` (+ `memory_find_tests.rs`); modify `crates/core/src/mongo_store.rs` (`find` becomes a call into `mongo_find::find`; `mod` line; the file shrinks), `crates/core/src/mongo_explain.rs` (`explain_command(ns, q, verbosity, &AppliedFindOptions)`; criteria `read_criteria(&o.options).or_else(|| explain_criteria(db))`; errors through `classify_with`), `crates/core/src/mongo_explain_tests.rs` (call sites; new tests go to `mongo_explain_options_tests.rs`, registered in `mongo_explain.rs`), `crates/core/src/test_support/memory_store.rs` (`find` and `explain_find` delegate one line each into `memory_find`; net growth 0: move `select` there), `test_support/mod.rs`, `crates/core/src/runtime/typed_record.rs` (skip when hint or collation set).

**Produces:**
```rust
// mongo_find.rs
/// The driver options of a page find: skip, limit, sort, projection and the applied options.
pub(crate) fn find_options(q: &Query, o: &AppliedFindOptions) -> Result<mongodb::options::FindOptions>;
pub(crate) async fn find(collection: &Collection<Document>, q: &Query) -> Result<Page>; // classify_with on every driver error
// mongo_explain.rs
pub(crate) fn explain_command(ns: &Namespace, q: &Query, verbosity: Verbosity, o: &AppliedFindOptions) -> Result<Document>;
// the find doc gains explain_fields(&o.options) (collation, hint, allowDiskUse, comment); outer maxTimeMS from o
// memory_find.rs (in-memory store)
impl InMemoryStore {
    pub fn received_queries(&self) -> Vec<(FindOperation, Query)>; // find, explain_find, explain_plan in call order
}
// a hint naming no index of the namespace (memory_indexes) answers CoreError::HintNotFound; key hints are accepted
// typed_record.rs: record_typed returns early when query.options.hint or .collation is set
```

- [ ] Failing tests in `mongo_find_tests.rs`: `default_find_options_are_today_s` (no `max_time`, `allow_disk_use`, `collation`, `hint`, `comment`, `selection_criteria`; skip/limit/sort/projection as `find` sent them), `find_options_carry_every_set_option` (collation via `driver_collation`, hint name and keys, comment as `Bson::String`, `max_time` 500 ms, `allow_disk_use` `Some(true)`, read preference `SecondaryPreferred`), `a_time_above_the_bound_is_capped_at_thirty_seconds`, `a_negative_skip_still_fails_before_any_option` (existing `skip_as_i64` rule). In `mongo_explain_options_tests.rs`: `the_default_explain_command_is_unchanged` (exact `doc!` equal to today's for both verbosities), `the_explain_find_carries_collation_hint_comment_and_disk_use`, `the_explain_time_only_lowers`, `explain_criteria_prefer_the_option_over_the_connection`. In `memory_find_tests.rs`: `the_store_records_what_each_find_received`, `a_hint_naming_no_index_is_hint_not_found`, `a_listed_hint_and_a_key_hint_pass`, `explain_and_plan_record_their_operation`. `typed_record_tests.rs` (existing file, under 400): `a_hinted_or_collated_verdict_records_no_shape`, `a_plain_verdict_still_records_its_shape`. `read_only_store_tests.rs`: `find_options_pass_through_a_read_only_store`. A handler test in a new `runtime/find_options_tests.rs` (registered in `runtime/mod.rs` with one `#[cfg(test)] mod` line, the only edit there): `a_find_answered_read_preference_unavailable_is_failed_not_lost` (no ping, session stays connected).
- [ ] Implement; `wc -l crates/core/src/mongo_store.rs crates/core/src/test_support/memory_store.rs` (both shrink or stay); `cargo +1.87.0 test -p curlybrace-core`; commit `feat(query): send find options to the server with typed errors`.

### Task 3: container tests (core)

**Files:** create `crates/core/tests/local_find_options.rs` (`mod support;`, `#[ignore]` + `local_enabled`, throwaway `cbit_*` databases through `local_mongo()`, dropped at the end, as `local_aggregate_options.rs`).

Tests: `collation_strength_two_matches_and_sorts_ignoring_case` (`{name: "abc"}` finds `ABC` and `abc`; sort on `name` under `en` strength 2 versus binary order), `numeric_ordering_sorts_digit_strings_as_numbers`, `an_invalid_locale_is_collation_rejected`, `a_hint_on_a_missing_index_is_hint_not_found` (find, `explain_find` and `explain_plan` each), `a_hint_by_name_and_by_keys_is_the_plan_s_index` (explain `queryPlanner` names the hinted, non-obvious index), `a_lowered_max_time_expires` (200 docs, filter `{$where: "function() { var t = Date.now(); while (Date.now() - t < 5) {} return true; }"}`, `maxTimeMS` 50: `Database("timeout")`; if server-side JS is disabled on that build, fall back to 200 000 docs with an unindexed sort and `maxTimeMS` 1, and say which ran), `the_comment_reaches_the_profiler` (profiler level 2 on the throwaway db; the find's `system.profile` entry has `command.comment`), `allow_disk_use_reaches_the_server` (`command.allowDiskUse == true` in the profile entry), `the_default_find_sends_no_max_time` (profile entry has no `maxTimeMS`), `secondary_preferred_reads_on_a_one_member_set` and `secondary_on_a_one_member_set_is_read_preference_unavailable` (both `replica_set_or_skip`; the second with `serverSelectionTimeoutMS=2000` in the URI).

- [ ] Write the tests, run them on the standalone throwaway server, then on the one-member replica set (the two read preference tests not skipped there); stop and delete each server by its pid.
- [ ] Commit `test(query): container tests for find options`.

### Task 4: find code with options: mongosh, Node.js, Python, PHP, Ruby (codegen)

**Files:** create `crates/core/src/codegen/find_options.rs` (shared helpers for find chains; under 200 lines), `crates/core/src/codegen/tests_find_options.rs` (registered in `codegen/mod.rs`), `crates/core/tests/codegen_find_options_golden.rs`, goldens `crates/core/tests/codegen/{mongosh,node,python,php,ruby}/find_options_{all,hint_keys}{,_no_setup}.txt`; modify `codegen/mod.rs` (`Renderer::find` gains `agg: &CodeAggOptions`; `generate` computes `CodeAggOptions::of(&query.options, options.include_options, false)`), `codegen/js_ops.rs`, `mongosh.rs`, `node.rs`, `python.rs`, `php.rs`, `ruby.rs`, `java.rs`, `csharp.rs`, `go.rs`, `rust.rs` (the four: parameter only, `_agg`, until Task 5), `codegen/tests_options.rs` (`options_never_reach_find_code` replaced by `find_code_carries_the_options_unless_include_is_off`; same line count), `codegen/model.rs` (doc of `include_options`: "the aggregate's or the query's options").

**Produces:**
```rust
pub(crate) trait Renderer {
    fn find(&self, ns: &Namespace, query: &Query, agg: &CodeAggOptions, options: &CodeOptions) -> Result<String, CodegenError>;
    ..
}
```
Spellings (order: collation, hint, allowDiskUse, maxTimeMS, comment, read preference; strings only through `Syntax::string`):

| Language | Find with options | Verified |
|---|---|---|
| mongosh | cursor chain lines after `find(..)`: `.collation({ locale: "en", strength: 2 })`, `.hint("status_1")` / `.hint({ status: 1 })`, `.allowDiskUse(true)`, `.maxTimeMS(5000)`, `.comment("..")`, `.readPref("secondaryPreferred")` | **No**: `allowDiskUse(false)` and `readPref` checked by Task 5's mongosh run |
| Node.js | `find(filter, { readPreference: "secondaryPreferred" })` when set, chain `.collation(..)`, `.hint(..)`, `.allowDiskUse(true)`, `.maxTimeMS(5000)`, `.comment("..")` before `.toArray()` | **No** (no driver installed): `FindCursor` methods, `FindOptions.readPreference` |
| Python | extra keyword lines in `find(..)`: `collation={..}`, `hint="status_1"` / `hint=[("status", 1)]`, `allow_disk_use=True`, `max_time_ms=5000`, `comment=".."`; `collection.with_options(read_preference=ReadPreference.SECONDARY_PREFERRED).find(` | **No**: `Collection.find` kwargs |
| PHP | the options array gains `'collation' => [..]`, `'hint' => ..`, `'allowDiskUse' => true`, `'maxTimeMS' => 5000`, `'comment' => '..'`, `'readPreference' => new MongoDB\Driver\ReadPreference('secondaryPreferred')` | **No** (no ext-mongodb) |
| Ruby | trailing `collation:`, `hint:`, `allow_disk_use:`, `max_time_ms:`, `comment:` in `find(filter, ..)`; `collection.with(read: { mode: :secondary_preferred })` | **No** (no gem) |

Golden cases (ns `shop.orders`, filter `{ status: "A" }`, sort `{ total: -1 }`, limit 20), setup on and off, `UPDATE_GOLDEN=1` and `check` copied from `codegen_options_golden.rs` (standalone file): `find_options_all` (the 4c `options_all` options, same comment fixture with quotes, backslash, `$x`, `${y}`, `#{z}`, `%s`, `{0}`, U+202E), `find_options_hint_keys` (collation `fr_CA`, hint keys `{ status: 1, total: -1 }`, allowDiskUse false, read preference `nearest`).

- [ ] Failing tests in `tests_find_options.rs`: `no_options_render_today_s_find_code` (every language, every existing find case: include on with empty options equals include off equals today's golden text), `include_off_ignores_every_find_option`, `the_read_preference_is_never_left_out_of_find_code`, `invalid_find_options_are_refused_without_their_value` (`max_time_ms: Some(0)` -> `InvalidOptions`, Display holds no value), `a_find_option_string_cannot_break_out_of_its_literal` (comment `"); drop(); //` and `'; system('x'); //` per language), `code_writes_is_false_for_find_with_options`.
- [ ] Failing golden test `codegen_find_options_golden.rs::the_find_options_goldens_match` over `[Mongosh, Node, Python, Php, Ruby]` plus `no_find_options_golden_holds_a_raw_invisible_character`. Write with `UPDATE_GOLDEN=1 cargo +1.87.0 test -p curlybrace-core --test codegen_find_options_golden`, read each file against the table, rerun without it.
- [ ] `cargo +1.87.0 test -p curlybrace-core codegen`, `--test codegen_golden` and `--test codegen_options_golden` (no `UPDATE_GOLDEN`, unchanged); commit `feat(codegen): render find options in mongosh, Node.js, Python, PHP and Ruby code`.

### Task 5: find code with options: Java, C#, Go, Rust; check with real tools (codegen)

**Files:** create goldens `crates/core/tests/codegen/{java,csharp,go,rust}/find_options_*.txt`, `crates/core/tests/local_codegen_find_options.rs`; modify `codegen/java.rs`, `csharp.rs`, `go.rs`, `rust.rs` (imports as in 4c: Java `ReadPreference`, `Collation` + used enums, `TimeUnit`; Go `readpref`; Rust `use mongodb::options::{..}` sorted and `std::time::Duration`), `tests/codegen_find_options_golden.rs` (`Language::ALL`).

| Language | Find with options | Verified |
|---|---|---|
| Java (sync 5.x) | `FindIterable` chain `.collation(Collation.builder()..build())`, `.hintString("x")` / `.hint(new Document(..))`, `.allowDiskUse(true)`, `.maxTime(5000, TimeUnit.MILLISECONDS)`, `.comment("..")`; handle `collection.withReadPreference(ReadPreference.secondaryPreferred())` | `javap` on the 5.8.1 jars in `~/.m2` |
| C# | `var options = new FindOptions { Collation = .., Hint = .., AllowDiskUse = true, MaxTime = TimeSpan.FromMilliseconds(5000), Comment = ".." };` then `Find(filter, options)`; handle `WithReadPreference(ReadPreference.SecondaryPreferred)` | **No** (no dotnet): say so in the PR |
| Go (v2) | `options.Find()` chain `SetCollation(&options.Collation{..})`, `SetHint(..)`, `SetAllowDiskUse(true)`, `SetComment("..")`; time limit: if `FindOptionsBuilder` has no `SetMaxTime` in v2, a `context.WithTimeout(context.TODO(), 5000*time.Millisecond)` with `defer cancel()` and a fixed note that the driver derives `maxTimeMS` from it; handle `Clone(options.Collection().SetReadPreference(readpref.SecondaryPreferred()))` | check the module source if it is in the cache; otherwise **No**, listed |
| Rust (`mongodb` 3) | setters on the `find(filter)` action: `.collation(..)`, `.hint(Hint::Name(..))` / `.hint(Hint::Keys(doc! {..}))`, `.allow_disk_use(true)`, `.max_time(Duration::from_millis(5000))`, `.comment("..")`, `.selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::SecondaryPreferred { options: None }))` | compiled in a scratch crate `--offline` |

- [ ] Failing golden test over all nine languages plus `every_language_renders_find_options_all`. Write, read, rerun.
- [ ] Real tools on every `find_options_*` golden in a scratch dir under the session scratchpad (nothing committed): `node --check`, `python3 -m py_compile`, `php -l`, `ruby -c`, `gofmt -l -e`, `javac` with the three 5.8.1 jars, Rust scratch crate (`mongodb = "=3.6.0"`, `tokio`, `futures`, `--offline`). Fix the renderer, not the golden.
- [ ] `local_codegen_find_options.rs` (`#[ignore]`, `local_enabled`, `/opt/homebrew/bin/mongosh` present, throwaway `cbit_*` db): `the_mongosh_find_code_with_every_option_runs` (index `status_1_total_-1`, three orders, the generated file run with `mongosh <uri> --quiet --file`: exit 0, `A` orders by total), `the_mongosh_read_pref_is_honoured` (replica-set run only: `secondary` with `serverSelectionTimeoutMS=2000` exits non-zero, `secondaryPreferred` exits 0). If `readPref` or `allowDiskUse(false)` is not honoured, switch the spelling, regenerate, report.
- [ ] `cargo +1.87.0 test -p curlybrace-core`; commit `feat(codegen): render find options in Java, C#, Go and Rust code`.

### Task 6: the find bar holds the options; the reducer sends, routes and gates (UI viewmodel + reducer)

**Files:** create `crates/ui/src/viewmodel/findbar_options.rs` (+ `findbar_options_tests.rs`), `crates/ui/src/viewmodel/options_action.rs` (only if `FindAction` needs the hint actions shared; otherwise skip), `crates/ui/src/app/find_options_actions.rs` (+ `find_options_actions_tests.rs`), `crates/ui/src/app/bulk_entry_state.rs` (move `impl AppState { fn bulk_entry }` out of `bulk_actions.rs`, then add the rule); modify `viewmodel/findbar.rs` (fields; `submit_find` attaches the options and refuses a blocked form; `resolve_find_intent` Clear resets them), `viewmodel/findbar_more.rs` (`more_open` and the count include the set options: two expressions), `viewmodel/mod.rs`, `viewmodel/bulk_entry.rs` (`BulkEntry::Collated`, reason `query.options.bulk_collation`), `viewmodel/copy_clipboard.rs` (`PasteBlock::CollatedResult`), `viewmodel/doc_copy.rs` (`clipboard_for` refuses `DocCopy::Result` when `query.options.collation` is set), `viewmodel/copy_menu.rs` (key `query.options.copy_collation`), `viewmodel/code_export_memo.rs` (`FindBarState::export_error` unchanged; new `find_code_block(&self) -> Option<&'static str>` in `findbar_options.rs`), `views/code_export_entry.rs` (`find_blocked` also returns `query.options.blocked`), `app/explain_actions.rs` (`same_run` compares `options`), `app/actions.rs` (`FindAction::HintListWanted` / `HintListRetry` arms delegating to `aggregate_hint_actions::{wanted, retry}`), `app/code_export_actions.rs` (`CodeSource::Find` request carries `state.find.options.parsed()`; `None` when blocked), `app/reducer.rs` (the `"find"` arm of `data_failed` becomes `"find" => super::find_options_actions::failed(state, error)`), `app/mod.rs` (`mod find_options_actions; mod bulk_entry_state;`), `crates/ui/locales/en.json` (`query.options.{blocked, bulk_collation, copy_collation, timeout_set}`).

**Produces:**
```rust
// viewmodel/findbar.rs: FindBarState gains
pub options: OptionsForm,            // 4b's form, as typed
pub options_open: Option<bool>,      // like `more`: None follows the content, a choice sticks until Clear
pub error_from_options: bool,        // the last error came from a find that carried a lowered time limit
// FindAction gains
HintListWanted, HintListRetry,
// viewmodel/findbar_options.rs
pub fn options_set_count(form: &OptionsForm) -> usize; // parsed: set_count; blocked: count of fields differing from the default form
pub fn options_open(state: &FindBarState) -> bool;
impl FindBarState {
    pub fn options_toggled(&self) -> Self;
    pub fn options_settled(&self) -> Self;                 // as more_settled
    pub fn options_edited(self, edit: OptionsEdit) -> Self; // the form only; clears error_from_options
    pub fn options_reset(self) -> Self;
    pub fn options_blocked(&self) -> bool;                  // form.is_blocked()
    pub fn find_code_block(&self) -> Option<&'static str>;  // Some("query.options.blocked") when blocked
}
/// The weak lines that say which options shaped the shown page (keys + params, literals).
pub fn page_options_status(options: &QueryOptions, i18n: &I18n) -> Vec<StatusLine>;
// = aggregate_options_view::options_status (read_from, hint) + query.options.status.{collation {locale},
//   max_time {ms} only when find_max_time_effect(Find) == Lowered, disk_use when Some(true),
//   secondary_note when the mode is set and not Primary}
// app/find_options_actions.rs
/// A failed find: an error an option caused (HintNotFound, CollationRejected,
/// ReadPreferenceUnavailable, or a timeout while issued_query sent a lowered time limit) goes to
/// the find bar with More and the section open; anything else keeps today's banner path.
pub(super) fn failed(state: AppState, error: CoreError) -> AppState;
// app/bulk_entry_state.rs: bulk_entry() returns Collated when page_query.options.collation is set
//   (after ReadOnly and Projected, before Busy)
```
`submit_find`: text error first (as today); then a blocked form returns `Err(state with more: Some(true), options_open: Some(true))` and no action; else `Ok(Query { options: form.parsed().cloned(), ..parsed })`.

- [ ] Failing tests in `findbar_options_tests.rs`: `the_default_bar_finds_with_no_options`, `find_attaches_the_parsed_options`, `a_blocked_form_finds_nothing_and_opens_more_and_the_section`, `a_text_error_is_reported_before_an_options_problem`, `clear_resets_the_options_and_their_choice`, `more_counts_and_opens_for_set_options`, `the_section_settles_open_like_more`, `toggling_overrides_the_automatic_state`, `paging_keeps_the_options_of_the_shown_page` (Next after editing the form: the command's query has the page's options), `the_status_names_what_shaped_the_page` (each line, and none for empty options), `a_raised_time_limit_shows_no_time_line`, `the_code_block_names_the_options_problem`, `edits_never_log` (no `log` call path: assert via the `Debug` of the state holding no value).
- [ ] Failing tests in `find_options_actions_tests.rs` (reducer level, a connected session with a collection, `AppState` built as `app/explain_tests.rs` does, `InMemoryStore`-free: commands inspected): `find_sends_the_options_in_the_query`, `the_verdict_explain_and_the_guard_check_carry_them` (`ExplainFind.query.options`, `GuardCheck.targets[0].query.options`), `an_answer_for_other_options_is_stale` (same text, other options: `Results` ignored), `a_verdict_for_other_options_is_not_shown` (`same_run`), `refetch_after_a_write_keeps_the_options` (insert finished -> `Find` with the page's options), `a_hint_not_found_failure_is_inline_without_a_banner` (also `CollationRejected`, `ReadPreferenceUnavailable`; `banner == None`, `last_error` set, More and section open, `loading_page == false`), `a_timeout_under_a_lowered_limit_is_inline_and_worded_as_yours` (`error_from_options`), `a_plain_timeout_keeps_the_banner`, `other_find_failures_keep_the_banner`, `hint_list_wanted_from_the_find_bar_sends_one_list_indexes` (request `>= HINT_REQUEST_BASE`; with the Indexes tab holding the ns: no command), `options_work_on_a_read_only_session`, `bulk_is_off_while_the_page_was_found_with_a_collation` (`bulk_entry() == Collated`, `Open` sends nothing; a hint alone does not block), `copying_the_whole_result_is_refused_under_a_collation_but_picked_documents_copy`, `copy_as_code_for_find_carries_the_options` (dialog request), `copy_as_code_is_off_while_the_form_is_blocked`, `opening_another_collection_resets_the_options`, `a_parked_session_keeps_its_options`.
- [ ] `wc -l crates/ui/src/app/bulk_actions.rs crates/ui/src/app/reducer.rs crates/ui/src/app/mod.rs` (bulk_actions shrinks; reducer +0 net; mod +2); `cargo +1.87.0 test -p curlybrace-ui` and `cargo +1.87.0 test -p curlybrace-ui i18n`; commit `feat(query): hold find options in the query bar and send them`.

### Task 7: extract the shared options form (views; no behaviour change)

**Files:** create `crates/ui/src/views/options_form.rs` (the collation group, the hint group, the tri/enum combos and the other fields drawn from `&OptionsForm` + `&HintListUi` + a `FormContext`; returns `Gathered`), `crates/ui/src/views/options_form_tests.rs`; modify `views/aggregate_options.rs` (`show_collation`, `show_others`, `show_max_time` call the shared form; the file shrinks), `views/aggregate_options_hint.rs` (takes `&OptionsForm, &HintListUi` instead of `&AggregateState`; may fold into `options_form.rs`), `views/aggregate_options_fields.rs` (`Typed` gains `pub enter: bool`; `typed_field` and `pick_combo` take an `id_scope: &str` salt so the find bar and the Aggregate tab never share a widget id), `views/mod.rs`.

**Produces:**
```rust
// views/options_form.rs
pub(super) struct FormContext<'a> {
    pub id_scope: &'a str,                       // "aggregate" | "find"
    pub layout: FormLayout,                      // Columns (aggregate, > TWO_COLUMNS_FROM) | Wrapped (find)
    pub disk_use_note: &'static str,             // "aggregate.options.allow_disk_use.preview_note" | "query.options.disk_use_note"
    pub comment_placeholder: &'static str,
    pub max_time_placeholder: (&'static str, Vec<(&'static str, String)>),
    pub max_time_clamped: Option<(&'static str, Vec<(&'static str, String)>)>,
    pub flags_when_off: FlagsWhenOff,            // Disabled (aggregate) | Hidden (find)
}
pub(super) fn show_options_form(ui: &mut egui::Ui, form: &OptionsForm, hints: &HintListUi, cx: &FormContext, i18n: &I18n) -> Gathered;
```
`Gathered` moves here (`edits`, `commit`, `reset`, new `enter`). The Aggregate tab builds its `FormContext` with today's keys and `FormLayout::Columns`; nothing it shows or emits changes.

- [ ] Failing tests in `options_form_tests.rs` (kittest): `the_wrapped_layout_shows_every_control_in_reading_order`, `hidden_flags_appear_when_collation_is_ticked`, `two_scopes_never_share_a_widget_id` (both forms in one frame: typing in one changes only its own `Gathered`), `enter_is_reported_apart_from_focus_loss`.
- [ ] Run the existing suites unchanged: `cargo +1.87.0 test -p curlybrace-ui aggregate_options`, `--test accessibility_aggregate_options`, `--test aggregate_options_layout`, and the snapshots `CURLYBRACE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test aggregate_snapshots -- --include-ignored` (no `UPDATE_SNAPSHOTS`: every aggregate PNG identical). Commit `refactor(ui): share the options form between the aggregate and find views`.

### Task 8: the Query options section, status line and code dialog (views)

**Files:** create `crates/ui/src/views/findbar_options.rs` (toggle + bounded, scrolling section; calls `show_options_form` with the find context), `crates/ui/src/views/findbar_options_tests.rs` (kittest); modify `views/findbar.rs` (`show_findbar` keeps its signature and delegates to new `show_findbar_with(ui, state, paging, hints: &HintListUi, i18n)`; the error line words `query.options.timeout_set` when `error_from_options`; Find gated with `query.options.blocked`), `views/findbar_more.rs` (the toggle at the end of the More row; `show_more_fields` returns `MoreOutcome { enter, toggled }`), `views/mod.rs`, `app/browse_panel.rs` (`show_findbar_with(.., &state.aggregate.options_ui.hint, ..)`; status after the verdict in `show_switches_and_verdict`), `views/code_export_options.rs` + `viewmodel/code_export_options.rs` (`code_options_view` handles `CodeRequest::Find`; `CodeOptionsView.include_key`: `codegen.options.include_find_options` for find), `crates/ui/locales/en.json` (remaining `query.options.*`, `codegen.options.include_find_options`).

**Produces:**
```rust
pub const OPTIONS_MAX_HEIGHT_SHARE: f32 = 0.35;
pub const OPTIONS_MIN_HEIGHT: f32 = 140.0;
pub(super) fn show_options_toggle(ui: &mut egui::Ui, state: &FindBarState, i18n: &I18n) -> bool; // toggled
pub(super) fn show_options_section(ui: &mut egui::Ui, state: &FindBarState, hints: &HintListUi, i18n: &I18n) -> SectionOutcome;
pub(super) struct SectionOutcome { pub edits: Vec<OptionsEdit>, pub reset: bool, pub enter: bool, pub wants_hints: bool }
pub fn show_page_options_status(ui: &mut egui::Ui, options: &QueryOptions, i18n: &I18n);
```
Order inside `show_findbar_with`: settle `more` and `options_open`; Filter row; More row (Sort, Projection, Skip, Limit, Query options toggle); section; error line or `query.options.blocked`; then edits applied (`state = take(state).options_edited(e)` in order, then reset), then the intent (Enter from any field, the options' fields included). Emits `FindAction::HintListWanted` while the section is open and the list `wants_list()`. Toggle: "Query options" (tonal) or "Query options (N)" (filled), `WidgetInfo::selected(Button, true, open, label)`, hover `toggle_hint`.

- [ ] Failing tests in `findbar_options_tests.rs`: `the_toggle_reads_query_options_then_the_count`, `the_toggle_reports_its_expanded_state`, `a_closed_section_draws_nothing`, `the_section_sits_under_the_more_row`, `typing_a_locale_edits_the_form_and_sends_nothing`, `enter_in_an_options_field_finds`, `a_problem_shows_under_its_field_and_find_is_off_with_the_reason`, `the_hint_combo_lists_the_indexes_and_picking_sets_the_hint`, `the_open_section_asks_for_the_hint_list_once`, `a_time_limit_above_thirty_seconds_says_finds_stop_there`, `the_comment_note_says_it_reaches_the_server`, `reset_empties_the_form`, `the_section_scrolls_within_its_share_of_the_window`, `a_lowered_timeout_is_worded_as_yours`. In `views/code_export_options_tests.rs` (existing, under 400) or a new sibling: `find_code_with_options_shows_the_include_check_with_its_own_label`, `find_code_without_options_shows_no_check`.
- [ ] `wc -l crates/ui/src/views/findbar*.rs crates/ui/src/app/browse_panel.rs`; `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(query): show the query options section and what shaped the page`.

### Task 9: accessibility, layout, snapshots, end to end (UI)

**Files:** create `crates/ui/tests/accessibility_find_options.rs`, `crates/ui/tests/find_options_layout.rs`, `crates/ui/tests/support/find_options_shots.rs` (registered in `tests/support/mod.rs`), `crates/ui/tests/local_find_options_flows.rs`; modify `crates/ui/tests/snapshots.rs` (three shots), `tests/snapshots/browser_find_options*.png` (new).

- [ ] `accessibility_find_options.rs` (English locale, driven like `accessibility_findbar.rs`): `every_query_options_control_has_a_role_and_a_name`, `tab_walks_the_bar_then_the_section_in_reading_order` (Filter, Find, Clear, Copy as code, Previous, Next, More options, Sort, Projection, Skip, Limit, Query options, Collation, Locale, Common locales, Strength, Existing indexes, Hint, Clear, Time limit, Read preference, Allow disk use, Comment, Reset options), `the_toggles_are_toggled_and_named_with_their_counts`, `a_problem_is_text_under_its_field_and_find_reads_the_reason`, `every_option_is_usable_on_a_read_only_connection`, `the_page_status_is_text`.
- [ ] `find_options_layout.rs` (accesskit bounds of the real app, as `browse_layout.rs`): `with_the_section_closed_the_first_row_stays_within_the_budget` (`FIRST_ROW_BUDGET` 300 at 1000x700, unchanged), `at_800x500_the_open_section_keeps_a_document_row_on_screen_and_nothing_overlaps` (collation ticked, every flag shown), `at_1000x700_the_open_section_wraps_without_overlap`, `the_section_scrolls_and_never_exceeds_its_share`.
- [ ] Snapshots (`find_options_shots.rs`: a page found with collation `en` strength 2, hint `status_1`, time limit 5000, read preference `secondaryPreferred`, section open, one problem on the comment): `browser_find_options` (1000x700), `browser_find_options_dark`, `browser_find_options_small` (800x500). `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture`, look at each PNG, rerun without `UPDATE_SNAPSHOTS` (every existing browser PNG unchanged; do not commit stray `*.new.png`).
- [ ] `local_find_options_flows.rs` (`#[ignore]` + `local_enabled`, own `cbit_*` database dropped at the end, the `Fixture` of `local_e2e_flows.rs`, throwaway server only): `a_collation_set_in_the_section_changes_the_page` (strength 2: filter `{name: "abc"}` shows `ABC` and `abc`; the status line names the collation), `a_hint_on_a_missing_index_says_so_in_the_bar` (`errors.hint_not_found` text, no banner, last page kept), `the_hint_combo_lists_the_collection_s_indexes`, `a_lowered_time_limit_times_out_in_your_words` (the `$where` fixture of Task 3), `paging_keeps_the_options`, `copy_as_code_carries_the_options` (Node text holds `maxTimeMS(5000)`), `update_matching_is_off_under_a_collation`.
- [ ] Commit `test(query): accessibility, layout, snapshots and flows of find options` (tests + PNGs).

### Task 10: README, site, full check

**Files:** modify `README.md` (Browse: line ~49 query bar bullet; Copy as code ~303-310; roadmap ~386, 390, 408), `site/index.html` (the query bar and roadmap lines, as #176 did).

- [ ] README: the query bar bullet names **Query options** behind More options (collation, hint with the collection's indexes, time limit that only lowers: finds stop at 30 s and nothing is sent by default, read preference, comment that reaches the server's logs and profiler, allow disk use sent as set on MongoDB 4.4+); the options apply on Find and stay with paging and refresh; the line under the switches says what shaped the page, with "Recent writes may not show yet" on a secondary; typed errors in the bar; bulk update/delete and copying the whole result are off while a collation shaped the page; the verdict and the scan warning judge the query with its options; Copy as code carries them (include check). Say plainly that query history (#72) does not exist yet and options are not stored. Roadmap: #75 done (aggregate and find); remove "Options on the Browse find query" from open items. Site: the same in its own words.
- [ ] Full check: `cargo +1.87.0 fmt --all --check`, `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`, `cargo +1.87.0 test --workspace --locked`; coverage of the new modules at least 80% (`cargo llvm-cov` if installed, else list tests per public function in the PR body); `wc -l` every touched file (report the over-800 files before and after; none created over 400); `git diff --stat main -- crates/core/tests/codegen` shows only new `find_options_*` files; a grep for `info!`, `warn!`, `error!` and `debug!` over the new modules shows no option value or query text.
- [ ] Commit `docs(query): describe find query options in the README and site`. Report the new i18n keys, the regenerated PNGs (none expected besides the new three), and every driver API name still unverified.

## en.json

Under `query`, between `guard` and `verdict`:
```json
"options": {
  "blocked": "Fix the query options to run or copy this query.",
  "bulk_collation": "This page was found with a collation, and a bulk write matches without one. Clear the collation and find again to update or delete matching documents.",
  "comment_placeholder": "Text to find this query in the server's logs",
  "copy_collation": "This page was found with a collation, which a copy of the whole result would not use. Clear the collation and find again, or copy selected documents.",
  "disk_use_note": "Lets a large sort spill to disk on the server (MongoDB 4.4 or later).",
  "max_time": { "clamped": "Finds still stop at {limit} s.", "placeholder": "Default: none; finds stop at {limit} s" },
  "status": {
    "collation": "Collation: {locale}",
    "disk_use": "Disk use allowed",
    "max_time": "Time limit: {ms} ms",
    "secondary_note": "Recent writes may not show yet."
  },
  "timeout_set": "The query took longer than the time limit you set in Query options. The last results stay on screen.",
  "toggle": "Query options",
  "toggle_count": "Query options ({n})",
  "toggle_hint": "Collation, index hint, time limit, read preference, comment and disk use for this query"
}
```
Under `codegen.options` (before `include_options`): `"include_find_options": "Include the query's options"`. Written one key per line in the file. Reused, not added: `aggregate.options.{collation.*, hint.*, tri.*, read_preference.*, allow_disk_use.label, max_time.label, comment.label, comment_sent_note, problem.*, reset, status.read_from, status.hint}`, `errors.{hint_not_found, collation_rejected, read_preference_unavailable}`.

## Verification (in order, all with `cargo +1.87.0`)

1. Per task: the task's failing tests seen red, then green; `wc -l` of touched files.
2. `cargo +1.87.0 fmt --all --check`.
3. `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`.
4. `cargo +1.87.0 test --workspace --locked` (unit, kittest, i18n usage and en.json order tests).
5. Goldens: `--test codegen_golden`, `--test codegen_options_golden`, `--test codegen_find_options_golden` without `UPDATE_GOLDEN`; existing goldens unchanged.
6. Real tools on the new goldens (Task 5 list); report what could not be checked.
7. Throwaway `mongod` (standalone, then one-member replica set), never an existing server: `curlybrace-core --test local_find_options --test local_explain --test local_guard --test local_mongo_crud --test local_codegen_find_options -- --ignored`; `curlybrace-ui --test local_find_options_flows --test local_e2e_flows -- --ignored`; stop by pid, delete the dbpath.
8. Snapshots: `CURLYBRACE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test snapshots --test aggregate_snapshots --test code_export_snapshots -- --include-ignored` (only the three new PNGs; no stray `*.new.png`).
9. Run the app (`run` skill) against the throwaway server at 1000x700 and 800x500: More and Query options open and close, the first document row stays on screen with the section open, Tab order as in Task 9, the status line after a find with options, the inline hint error.
10. Coverage of the new modules at least 80%.

## Calls made here (owner may overrule; defaults are implemented)

1. `QueryOptions` is `PipelineOptions` (one type, one validation, one serde), carried inside `Query`.
2. Find sends no `maxTimeMS` unless set; set values only lower, capped at 30 s (`COMMAND_TIMEOUT`).
3. allowDiskUse on the find is sent as set (a Browse find is the real query, not a preview).
4. Options apply on Find, not on edit; paging keeps the shown page's options.
5. The hint picker reuses the session's aggregate hint list (one list per collection).
6. Option-caused find failures are inline in the bar, not banners.
7. Bulk matching and the whole-result copy are off under a collation.
8. Shared labels keep their `aggregate.options.*` keys.
9. Clear and a collection change reset the options.
10. A verdict under a hint or collation records no typed shape.
11. Query history (#72): not built; options are kept in the session and the code; #72 persists them with `PipelineOptions`' serde and restores with `OptionsForm::from_options`.
