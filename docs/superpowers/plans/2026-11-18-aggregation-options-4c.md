# Aggregate options, code and explain (slice 4c) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** The draft's options reach the copied code and the explain grows its follow-ups. Copy as code renders the options in all nine languages (an "Include options" check, on by default), byte-identical code when no option is set. The Plan tab offers **Try an index** / **Clear hint** when the plan reads the whole collection or sorts in memory, and a **suggested index** from the leading `$match` and `$sort` with **Copy command** and **Create index...** (the existing index builder, pre-filled; disabled with the reason on a read-only connection).

**Architecture:** Core: `CodeRequest::Aggregate` carries `options: PipelineOptions`; `generate` turns them into a language-neutral `CodeAggOptions` (what the code sets, the read preference left out when the code writes) and each renderer spells it (an options object, keyword arguments or a builder chain; the read preference on a derived collection handle or as an operation option, whichever the driver takes). `explain::suggest_for_pipeline` reuses `suggest_from`. UI: the dialog gets one check box and a summary line; the Plan tab gets a follow-up block computed by a pure viewmodel; four new `ExplainAction`s are handled in a new reducer module that reuses the options apply/preview path, the explain `Run` path and `index_write_actions::open_with`.

**Tech Stack:** Rust 1.87 (no let-chains), `bson`, egui/eframe 0.32, egui_kittest. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-16-aggregation-slice-4-design.md` ("Which operation sends what", Copy as code column; "Explain hint selector and suggested index (slice 4c)"; i18n keys 4c; the 4c bullet). Built on 4a (`pipeline_options.rs`, `pipeline_options_apply.rs`) and 4b (`OptionsGate`, `OptionsEdit::{HintPicked, HintCleared}`, `OptionsAction::HintListWanted`, `HintListUi`, `CodeBlock::Options`, `run_option_labels`, `keys_text`). Model the codegen work on `codegen/*.rs` and `tests/codegen_golden.rs`, the reducer work on `app/aggregate_explain_actions.rs` and `app/index_edit_actions.rs::edit_copy` (an `open_with` caller), the views on `views/aggregate_plan.rs` and `views/code_export.rs`.

## Global Constraints

- Files I create: 200-400 lines (never over 400). Existing files over 400 get **no new tests** and only the mechanical edits a changed signature forces: `codegen/tests.rs` (412), `app/aggregate_explain_actions_tests.rs` (412), `viewmodel/aggregate_options.rs` (430), `viewmodel/aggregate_options_tests.rs` (478), `pipeline_options.rs` (447), `viewmodel/index_builder.rs` (598), `app/index_write_tests.rs` (496), `app/panels.rs` (625). `tests/codegen_golden.rs` (380) only gets the `options` field in its request literal: the new goldens live in a new test file. `app/mod.rs` (1006) gains only `mod` lines (+2 at most), `app/reducer.rs` (722) nothing. `wc -l` every touched file before each commit.
- Immutable style (`Self { .., ..self }`, moved drafts, never `&mut` setters on options); real objects over mocks (`PipelineOptions { .. }`, `AggregateState::default().add_stage(..)`, the in-memory store); TDD (failing test, run it, implement, run it); coverage of new code at least 80%.
- i18n: every visible string, tooltip and accessible name is a key written as a **literal** (`i18n_usage_tests.rs` finds literals and fails on an unused or missing key); `crates/ui/locales/en.json` nested and alphabetical; a key is added in the task whose source first names it. Generated code is not UI text (as in slice 3a): the fixed note in the code is English. Full text in "en.json" below; keys beyond the spec are marked there (report them).
- Option values are user data: never logged, never in an error message (errors carry variants), never in a `Debug` that is logged (`CodeDialog`'s hand-written `Debug` stays as is). Strings from options (locale, hint name, comment) reach code only through each language's `Syntax::string` / `key`, so they cannot break out of a literal.
- `cargo +1.87.0`; git as plain separate Bash calls; conventional commits ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never touch `CHANGELOG.md`.
- NEVER connect to any MongoDB already on the machine (the user has others, incl. 27017; the URI is **always** set). Real-server tests run only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted afterwards (Tasks 2 and 5).

## Review Focus

- **Nothing set, nothing changes:** with `PipelineOptions::default()` (or "Include options" off) every existing golden under `crates/core/tests/codegen/` is byte-identical (`git diff --stat crates/core/tests/codegen` shows only new `options_*.txt` files).
- **Code equals Run:** the code carries the options **as set** (spec), which is what the confirmed run sends (`applied(Run)`: collation, hint, allowDiskUse, maxTimeMS up to 60 s, comment). The read preference is the one exception the run has: when the code ends with `$out`/`$merge` (write stage kept), it is left out and a fixed note says why (the run always reads and writes on the primary). The preview `$limit` is never in the code (unchanged). Find code never gets options.
- **Injection:** the comment fixture holds quotes, backslashes, `$x`, `${y}`, `#{z}`, `%s`, `{0}` and U+202E; every language escapes it (the golden check for raw invisible characters covers the new files). New identifiers the snippets introduce are reserved names (`keywords.rs`); note the list is case-insensitive, so e.g. `hint` and `collation` become invalid variable names (acceptable; stated in the PR).
- **Try an index:** sets the draft's hint through the 4b path (`OptionsEdit::HintPicked` + apply, so Options shows it, "modified" appears, the preview re-runs, counts and plan go out of date) and re-explains at the selected verbosity; off with a reason while an explain runs or the form is blocked; an index name the server does not know answers `errors.hint_not_found` in the Plan tab.
- **Create index...:** only through `index_write_actions::open_with` (read-only: refused with the banner, and the button is off with `create_index_read_only` before that; production: the builder's own write guard on Create). Nothing is built without the dialog. The suggestion is computed from the **explained** pipeline and the options the plan was made with (`ExplainDone`), never from a newer draft.

## Per-language spelling (what Tasks 1-2 emit)

Option order everywhere: collation, hint, allowDiskUse, maxTimeMS, comment, read preference. `maxTimeMS` is an int (validation caps it at 60 000). Hint by name is a string, by keys a document in the language's own document syntax. "Handle" = the read preference on a collection handle derived inline in the call (works with setup lines on or off); "op" = an option of the aggregate call.

| Language | Shape | Collation | Hint | allowDiskUse / maxTimeMS / comment | Read preference | Verified |
|---|---|---|---|---|---|---|
| mongosh | `const options = { .. };` then `aggregate(pipeline, options)` | `collation: { locale: "en", strength: 2, .. }` (MongoDB field names) | `hint: "status_1"` / `hint: { status: 1 }` | `allowDiskUse: true`, `maxTimeMS: 5000`, `comment: ".."` | op: `readPreference: "secondaryPreferred"` | **No**: `readPreference` passed through `aggregate` options (mongosh hands options to the Node driver); checked against a throwaway server in Task 2 |
| Node.js | same object, `await collection.aggregate(pipeline, options).toArray()` | same | same | same | op: `readPreference: "secondaryPreferred"` (`ReadPreferenceLike`) | **No** (no driver installed): `AggregateOptions.{collation, hint, allowDiskUse, maxTimeMS, comment, readPreference}` |
| Python | keyword arguments of `aggregate(pipeline, ..)` | `collation={"locale": "en", "strength": 2}` | `hint="status_1"` / `hint=[("status", 1), ("total", -1)]` (list of pairs, as `sort_pairs`) | `allowDiskUse=True`, `maxTimeMS=5000`, `comment=".."` | handle: `collection.with_options(read_preference=ReadPreference.SECONDARY_PREFERRED)`, `from pymongo import MongoClient, ReadPreference` | **No**: `Collection.aggregate` kwargs, `with_options(read_preference=)`, `ReadPreference.{PRIMARY, PRIMARY_PREFERRED, SECONDARY, SECONDARY_PREFERRED, NEAREST}` |
| Java (sync 5.x) | chain on the `AggregateIterable` before `.into(..)` | `.collation(Collation.builder().locale("en").collationStrength(CollationStrength.SECONDARY).caseLevel(false).collationCaseFirst(CollationCaseFirst.UPPER).numericOrdering(true).collationAlternate(CollationAlternate.SHIFTED).build())` | `.hintString("status_1")` / `.hint(new Document(..))` | `.allowDiskUse(true)`, `.maxTime(5000, TimeUnit.MILLISECONDS)`, `.comment("..")` | handle: `collection.withReadPreference(ReadPreference.secondaryPreferred())` | **Yes**, `javap` on `mongodb-driver-sync/core/bson 5.8.1` in `~/.m2` (every name in this row) |
| C# | `var options = new AggregateOptions { .. };` then `Aggregate<BsonDocument>(pipeline, options)` | `Collation = new Collation("en", caseLevel: false, caseFirst: CollationCaseFirst.Upper, strength: CollationStrength.Secondary, numericOrdering: true, alternate: CollationAlternate.Shifted)` (constructor parameter order) | `Hint = "status_1"` / `Hint = new BsonDocument { .. }` | `AllowDiskUse = true`, `MaxTime = TimeSpan.FromMilliseconds(5000)`, `Comment = ".."` | handle: `collection.WithReadPreference(ReadPreference.SecondaryPreferred)` | **No** (no dotnet): `AggregateOptions.{Collation, Hint (BsonValue), AllowDiskUse, MaxTime, Comment}`, the `Collation` constructor's parameter names, `IMongoCollection.WithReadPreference`, `ReadPreference.SecondaryPreferred` |
| Go (v2) | `opts := options.Aggregate().\n\tSetCollation(..).\n\t..` then `Aggregate(context.TODO(), pipeline, opts)` | `SetCollation(&options.Collation{Locale: "en", Strength: 2, CaseFirst: "upper", NumericOrdering: true, Alternate: "shifted"})` (a bool set to `false` equals the server default and is written as such: `CaseLevel: false` is omitted) | `SetHint("status_1")` / `SetHint(bson.D{..})` | `SetAllowDiskUse(true)`, `SetCustom(bson.M{"maxTimeMS": 5000})` (v2 has no `SetMaxTime`), `SetComment("..")` | handle: `collection.Clone(options.Collection().SetReadPreference(readpref.SecondaryPreferred()))`, import `go.mongodb.org/mongo-driver/v2/mongo/readpref` | **No** (module not in the cache, download refused in the sandbox): `AggregateOptionsBuilder.{SetCollation, SetHint, SetAllowDiskUse, SetComment, SetCustom}`, absence of `SetMaxTime`, `options.Collation` field names, `Collection.Clone` returning one value in v2, `options.Collection().SetReadPreference`, `readpref.*()` |
| PHP | `$options = [ .. ];` then `$collection->aggregate($pipeline, $options)` | `'collation' => ['locale' => 'en', 'strength' => 2]` | `'hint' => 'status_1'` / `'hint' => [..]` | `'allowDiskUse' => true`, `'maxTimeMS' => 5000`, `'comment' => '..'` | op: `'readPreference' => new MongoDB\Driver\ReadPreference('secondaryPreferred')` | **No** (no ext-mongodb): the library's aggregate option keys, the string-mode `ReadPreference` constructor |
| Ruby | trailing hash arguments of `aggregate(pipeline, ..)` | `collation: { 'locale' => 'en', 'strength' => 2 }` | `hint: 'status_1'` / `hint: { 'status' => 1 }` | `allow_disk_use: true`, `max_time_ms: 5000`, `comment: '..'` | handle: `collection.with(read: { mode: :secondary_preferred })` | **No** (no gem): aggregate option symbols, `Collection#with(read:)`, mode symbols |
| Rust (`mongodb` 3) | setters on the `aggregate(pipeline)` action before `.await?` | `.collation(Collation::builder().locale("en").strength(CollationStrength::Secondary).case_level(false).case_first(CollationCaseFirst::Upper).numeric_ordering(true).alternate(CollationAlternate::Shifted).build())` | `.hint(Hint::Name("status_1".to_string()))` / `.hint(Hint::Keys(doc! { .. }))` | `.allow_disk_use(true)`, `.max_time(Duration::from_millis(5000))`, `.comment("..")` | op: `.selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::SecondaryPreferred { options: None }))` | **Yes**, against `mongodb-3.6.0` source in `~/.cargo/registry` (the `option_setters` macro takes `impl Into<Bson>` for `comment`; `Collation` builder is `setter(into)`) and compiled in Task 2 |

Mode spellings: camel `primary`/`primaryPreferred`/`secondary`/`secondaryPreferred`/`nearest` (mongosh, Node, PHP; Java and Go call the same names as functions), snake `:secondary_preferred` (Ruby), upper snake `SECONDARY_PREFERRED` (Python), Pascal `SecondaryPreferred` (C#, Rust). The fixed note (comment marker `//` or `#`, after any conversion note): `Note: this pipeline writes with $out or $merge, which runs on the primary, so the read preference is left out.`

### Task 1: the options in the code model; mongosh, Node.js, Python, PHP, Ruby

**Files:** create `crates/core/src/pipeline_options_doc.rs` (+ `pipeline_options_doc_tests.rs`), `crates/core/src/codegen/agg_options.rs`, `crates/core/src/codegen/tests_options.rs`, `crates/core/tests/codegen_options_golden.rs`, goldens `crates/core/tests/codegen/{mongosh,node,python,php,ruby}/options_{all,hint_keys,write_stage}{,_no_setup}.txt`; modify `crates/core/src/lib.rs`, `crates/core/src/pipeline.rs` (re-export), `codegen/model.rs`, `codegen/mod.rs` (+ `#[cfg(test)] mod tests_options;`), `codegen/keywords.rs`, `codegen/js_ops.rs`, `mongosh.rs`, `node.rs`, `python.rs`, `php.rs`, `ruby.rs`, `java.rs`, `csharp.rs`, `go.rs`, `rust.rs` (these four: the new parameter only, named `_agg` until Task 2), every `CodeRequest::Aggregate` literal (`codegen/tests_langs.rs`, `tests_lossy.rs`, `tests_review.rs`, `tests_scripting.rs`, `tests.rs`, `tests/codegen_golden.rs`, `crates/ui/src/viewmodel/code_export.rs`, `code_export_tests.rs`, `app/code_export_actions.rs`, `app/code_export_actions_tests.rs`, `views/code_export_tests.rs`, `crates/ui/tests/accessibility_code_export.rs`, `crates/ui/tests/support/code_export_shots.rs`: `options: PipelineOptions::default()`; in `code_export_actions.rs` with `// 4c Task 3: the draft's options`).

**Produces:**
```rust
// pipeline_options_doc.rs
impl CollationOption { pub fn to_document(&self) -> Document; } // MongoDB names, set fields only, strength as Int32
impl HintOption { pub fn to_bson(&self) -> Bson; }              // String(name) | Document(keys)

// codegen/model.rs
pub enum CodeRequest { Find { .. }, Aggregate { ns: Namespace, stages: Vec<Document>, options: PipelineOptions } }
pub struct CodeOptions { .., /// Render the aggregate's options; off gives the code of no options.
    pub include_options: bool }  // Default: true
pub enum CodegenError { .., /// The aggregate options do not validate (never built by the UI, which blocks first).
    InvalidOptions }             // error_variant "InvalidOptions", Display "the aggregate options are not valid"

// codegen/agg_options.rs
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CodeAggOptions {
    pub collation: Option<CollationOption>, pub hint: Option<HintOption>, pub allow_disk_use: Option<bool>,
    pub max_time_ms: Option<i32>, pub comment: Option<String>, pub read_preference: Option<ReadPreferenceMode>,
    /// A read preference was set and left out because the code ends with a write stage.
    pub read_preference_left_out: bool,
}
impl CodeAggOptions {
    pub(crate) fn of(options: &PipelineOptions, include: bool, writes: bool) -> Result<Self, CodegenError>;
        // include false or nothing set => default; options.clone().validated() else InvalidOptions
    pub(crate) fn is_empty(&self) -> bool;          // nothing to write and no note
    pub(crate) fn has_call_options(&self) -> bool;  // any of the first five
    pub(crate) fn note(&self, comment: &str) -> Option<String>; // the fixed note line, with its marker
}
pub(crate) fn mode_camel(m: ReadPreferenceMode) -> &'static str;
pub(crate) fn mode_snake(m: ReadPreferenceMode) -> &'static str;
pub(crate) fn mode_upper_snake(m: ReadPreferenceMode) -> &'static str;
pub(crate) fn mode_pascal(m: ReadPreferenceMode) -> &'static str;
/// Entries `(rendered key, rendered value)` of the options that are plain BSON (collation as a
/// document, hint, allowDiskUse, maxTimeMS, comment), keys spelled by `key`; for the object/dict/hash languages.
pub(crate) fn plain_entries(syn: &dyn Syntax, agg: &CodeAggOptions, key: fn(&str) -> String, depth: usize)
    -> Result<Vec<(String, String)>, CodegenError>;

// codegen/mod.rs
pub(crate) trait Renderer { fn aggregate(&self, ns: &Namespace, stages: &[Document], agg: &CodeAggOptions,
    options: &CodeOptions) -> Result<String, CodegenError>; /* find unchanged */ }
/// Whether the code for `request` ends with `$out` / `$merge` under `options` (the dialog's note uses it too).
pub fn code_writes(request: &CodeRequest, options: &CodeOptions) -> bool;
```
`generate`: `exported_stages` as today, `writes = stage_is_write(last exported)`, `agg = CodeAggOptions::of(&options, code.include_options, writes)?`. Renderers emit **exactly the old text** when `agg.is_empty()`. With options: mongosh/Node build `const options = <syn.document(entries)>;` (read preference as a camel string entry) between `pipeline` and `results`, and call `aggregate(pipeline, options)`; Python adds keyword arguments, one per line inside the `list(..)` call (the `find` layout), and the `with_options(..)` handle; PHP `$options` array (the `ReadPreference` object as the last entry) and `aggregate($pipeline, $options)`; Ruby trailing `key: value` lines and `with(read: ..)`. Keys added to `keywords.rs` RESERVED (snippet names): `ReadPreference`, `Collation`, `CollationStrength`, `CollationCaseFirst`, `CollationAlternate`, `TimeUnit`, `AggregateOptions`, `TimeSpan`, `Hint`, `SelectionCriteria`, `Duration`, `readpref`, `with_options`.

Golden cases in `codegen_options_golden.rs` (ns `shop.orders`, stages `[{ $match: { status: "A" } }, { $sort: { total: -1 } }]` unless said), each with setup on and off, same `UPDATE_GOLDEN=1` switch and `check` helper as `codegen_golden.rs` (copied, the file stays standalone):
- `options_all`: collation `en`, strength 2, caseLevel false, caseFirst upper, numericOrdering true, alternate shifted; hint name `status_1_total_-1`; allowDiskUse true; maxTimeMS 5000; comment `nightly "report" 'q' \ $x ${y} #{z} %s {0} ` + U+202E + `end`; read preference `secondaryPreferred`.
- `options_hint_keys`: collation `fr_CA` only; hint keys `{ status: 1, total: -1 }`; allowDiskUse false; read preference `nearest`.
- `options_write_stage`: stages `[$match, { $out: "archive" }]`; read preference `secondary`; comment `archive`.

- [ ] Failing tests in `pipeline_options_doc_tests.rs`: `a_collation_document_has_only_the_set_fields_in_mongodb_spelling` (exact `doc!`), `strength_is_an_int32`, `a_hint_is_a_string_or_its_keys`.
- [ ] Failing tests in `codegen/tests_options.rs`: `no_options_render_the_code_of_today` (every language, every existing aggregate case: `include_options` on with `PipelineOptions::default()` equals `include_options` off), `include_options_off_ignores_every_option` (options_all request, off, equals the default-options code, all nine languages), `options_never_reach_find_code`, `the_read_preference_is_left_out_with_a_note_when_the_code_writes` (mongosh, Python: note present, no `secondary` text), `dropping_the_write_stage_keeps_the_read_preference` (`write_stage: false`), `code_writes_follows_the_write_stage_option`, `invalid_options_are_refused_without_their_value` (`max_time_ms: Some(0)`: `Err(InvalidOptions)`, Display holds no value), `an_option_string_cannot_break_out_of_its_literal` (comment `"); drop(); //` and `'; system('x'); //` per language: the generated text parses back to one string literal; assert the payload appears only escaped via each `quote` function), `new_snippet_names_are_reserved` (`ReadPreference`, `Duration`, `readpref` refused as `collection_var`).
- [ ] Failing golden test `codegen_options_golden.rs::the_options_goldens_match` over `[Mongosh, Node, Python, Php, Ruby]`, plus `no_options_golden_holds_a_raw_invisible_character` (same predicate as `codegen_golden.rs`). Write the goldens with `UPDATE_GOLDEN=1 cargo +1.87.0 test -p curlybrace-core --test codegen_options_golden`, read every file by eye against the table, then run without it.
- [ ] `cargo +1.87.0 test -p curlybrace-core codegen` and `--test codegen_golden` (unchanged goldens, no `UPDATE_GOLDEN`), `cargo +1.87.0 test -p curlybrace-ui` (compiles, nothing changes); commit `feat(codegen): render aggregate options in mongosh, Node.js, Python, PHP and Ruby code`.

### Task 2: Java, C#, Go, Rust; check the goldens with real tools

**Files:** create `crates/core/tests/local_codegen_options.rs`, goldens `crates/core/tests/codegen/{java,csharp,go,rust}/options_*.txt`; modify `codegen/java.rs` (frame takes the extra imports: `com.mongodb.ReadPreference`, `com.mongodb.client.model.Collation` and the used enum classes, `java.util.concurrent.TimeUnit`), `codegen/csharp.rs`, `codegen/go.rs` (`readpref` import when used; `check_names` clash list gains `readpref`), `codegen/rust.rs` (`use mongodb::options::{..};` with the used names sorted, `use std::time::Duration;`), `tests/codegen_options_golden.rs` (`Language::ALL`). Java, Go and Rust render the call as a chain, one setter per line under the call (Java/Rust four-space continuation, Go trailing `.` and a tab, `gofmt`'s style); C# `var options = new AggregateOptions\n{\n    ..,\n};`.

- [ ] Failing golden test over all nine languages plus `every_language_renders_the_options_all_case` (no error, file exists). Write, read, rerun as in Task 1.
- [ ] Check every `options_*` golden with real tools (scratch dir under the session scratchpad, nothing committed): `node --check` (Node, wrapped as in slice 3a), `python3 -m py_compile`, `php -l`, `ruby -c`, `gofmt -l -e`, `javac` with `-cp` the three 5.8.1 jars in `~/.m2/repository/org/mongodb/` (wrap `_no_setup` bodies in the setup frame), Rust: a scratch crate with `mongodb = "=3.6.0"`, `tokio`, `futures` built `--offline` with each golden as `src/main.rs`. C# is not compiled (no dotnet): say so in the PR. Fix the renderer, not the golden.
- [ ] `local_codegen_options.rs` (`#[ignore]`, gated like `local_aggregate.rs` with `local_enabled`, plus `/opt/homebrew/bin/mongosh` present; throwaway `cbit_*` database, dropped at the end): `the_mongosh_code_with_every_option_runs` (create index `status_1_total_-1`, insert three orders, write `generate(Mongosh, options_all request for the cbit ns, default CodeOptions)` to a temp file, run `mongosh <uri> --quiet --file <file>`: exit 0, output holds the `A` orders sorted by total), `the_mongosh_read_preference_option_is_honoured` (replica-set run only, `replica_set_or_skip`: the same code with read preference `secondary` and `serverSelectionTimeoutMS=2000` in the URI exits non-zero naming server selection; with `secondaryPreferred` it exits 0). If the second test shows the option is ignored, change mongosh's read preference to `db.getMongo().setReadPref("<mode>")` before the call, regenerate its goldens, and report it.

Throwaway servers (standalone, then a one-member replica set), from the worktree:
```bash
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
DBPATH=$(mktemp -d /private/tmp/claude-501/cbit-4c.XXXXXX)
/opt/homebrew/bin/mongod --dbpath "$DBPATH" --bind_ip 127.0.0.1 --port "$PORT" --fork --logpath "$DBPATH/log" --pidfilepath "$DBPATH/pid"
CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" cargo +1.87.0 test -p curlybrace-core --test local_codegen_options -- --ignored
kill "$(cat "$DBPATH/pid")"; rm -rf "$DBPATH"
# again with: mongod ... --replSet cbit4c, then
/opt/homebrew/bin/mongosh --quiet "mongodb://127.0.0.1:$PORT/?directConnection=true" --eval 'rs.initiate({_id:"cbit4c",members:[{_id:0,host:"127.0.0.1:'"$PORT"'"}]})'
# wait for PRIMARY, run with CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT/?replicaSet=cbit4c", stop by pid, delete
```
- [ ] `cargo +1.87.0 test -p curlybrace-core`; commit `feat(codegen): render aggregate options in Java, C#, Go and Rust code`.

### Task 3: the Copy as code dialog shows the options

**Files:** create `crates/ui/src/viewmodel/code_export_options.rs` (+ `code_export_options_tests.rs`), `crates/ui/src/views/code_export_options.rs`, `crates/ui/src/views/code_export_options_tests.rs` (kittest, registered in `views/code_export.rs`), `crates/ui/src/app/code_export_options_tests.rs` (registered in `code_export_actions.rs`); modify `viewmodel/mod.rs`, `views/mod.rs`, `viewmodel/code_export.rs` (`include_options_set`; the `edited` fallback keeps `include_options` as it keeps `setup` and `write_stage`), `app/code_export_actions.rs` (`CodeAction::SetIncludeOptions(bool)`; the request carries `state.aggregate.options.clone()`; the Task 1 note removed), `views/code_export.rs` (the check box after the write-stage box, the summary under the names; `changes_options` lists the new action), `views/code_export_text.rs` (`InvalidOptions`), `views/code_export_entry_tests.rs` (one test), `crates/ui/tests/accessibility_code_export.rs`, `tests/support/code_export_shots.rs`, `tests/code_export_snapshots.rs`, `crates/ui/locales/en.json` (`codegen.options.*`, `codegen.error.invalid_options`).

**Produces:**
```rust
// viewmodel/code_export_options.rs
/// What the dialog says about the options; `None` for find code or a draft with no option set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeOptionsView {
    pub include: bool,                 // the check box
    pub labels: Vec<&'static str>,     // label keys of what the code sets (empty when not included)
    pub read_preference_left_out: bool,
}
pub fn code_options_view(dialog: &CodeDialog) -> Option<CodeOptionsView>;
// labels: aggregate.options.collation.enabled, .hint.label, .allow_disk_use.label, .max_time.label,
// .comment.label, .read_preference.label (left out when code_writes), in that order
impl CodeExportUi { pub fn include_options_set(self, include: bool) -> Self; }
// views/code_export_options.rs
pub(super) fn show_include_options(ui: &mut egui::Ui, view: &CodeOptionsView, i18n: &I18n) -> (Option<CodeAction>, egui::Id);
pub(super) fn show_options_summary(ui: &mut egui::Ui, view: &CodeOptionsView, i18n: &I18n);
    // codegen.options.listed {options} = translated labels joined by ", "; read_preference_left_out note
```
The check box shows only for an aggregate whose request has options; its id joins the widgets that own Enter.

- [ ] Failing tests in `code_export_options_tests.rs` (viewmodel): `no_options_no_view`, `find_code_has_no_options_view`, `the_labels_follow_what_is_set_in_order`, `include_off_lists_nothing`, `a_kept_write_stage_leaves_the_read_preference_out_of_the_labels_and_says_so`, `include_options_is_remembered_for_the_next_dialog` (including after an invalid name edit).
- [ ] Failing tests in `app/code_export_options_tests.rs`: `opening_from_the_aggregate_tab_carries_the_draft_options`, `the_code_contains_the_options` (Node text holds `maxTimeMS: 5000`), `unchecking_include_options_regenerates_without_them`, `copy_puts_the_previewed_text_with_options_on_the_clipboard_output`, `a_blocked_form_opens_no_dialog` (the 4b gate: `Open(Aggregate)` changes nothing).
- [ ] Failing tests in `views/code_export_options_tests.rs`: `the_include_options_box_shows_only_when_options_are_set`, `toggling_it_emits_set_include_options`, `enter_on_the_box_does_not_copy`, `the_summary_names_the_options_in_words`, `the_left_out_read_preference_is_explained`. In `code_export_entry_tests.rs`: `the_aggregate_button_says_the_options_block_it` (tooltip and accessible description `aggregate.options.blocked`).
- [ ] `accessibility_code_export.rs`: `the_include_options_box_is_named_and_in_tab_order` (language, setup, write stage, include options, the names). Snapshot `code_export_dialog_options` (Node.js, options of `options_all` minus the bidi character) written with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test code_export_snapshots -- --include-ignored`, looked at, rerun without `UPDATE_SNAPSHOTS` (existing PNGs unchanged).
- [ ] `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(codegen): show and include the draft's options in Copy as code`.

### Task 4: suggested index, Try an index, Create index (core, viewmodel, reducer)

**Files:** create `crates/core/src/explain/agg_suggest.rs` (+ `agg_suggest_tests.rs`), `crates/ui/src/viewmodel/aggregate_plan_next.rs` (+ `aggregate_plan_next_tests.rs`), `crates/ui/src/viewmodel/index_builder_suggested.rs` (+ `index_builder_suggested_tests.rs`), `crates/ui/src/app/aggregate_plan_actions.rs` (+ `aggregate_plan_actions_tests.rs`); modify `explain/mod.rs` (mods + `pub use agg_suggest::{create_index_command, suggest_for_pipeline}`), `viewmodel/mod.rs`, `app/mod.rs` (`mod aggregate_plan_actions;`), `app/aggregate_explain_actions.rs` (four variants, each one arm delegating), `app/index_write_actions.rs` (`write_finished`: when `ns` is the selected collection, the active draft's hint list is reset with `hint_edited(HintListUi::reset)`, so the new index is listed).

**Produces:**
```rust
// explain/agg_suggest.rs
/// The equality-sort-range index for the leading `$match` and a `$sort` directly after it (a leading
/// `$sort` alone counts as an empty match). `None` for any other first stage, a `$match` with `$text`,
/// or no field to index.
pub fn suggest_for_pipeline(stages: &[Document]) -> Option<SuggestedIndex>;
/// `index.command(ns)` exactly without a collation; with one, `createIndex({ .. }, { collation: { .. } })`,
/// every string through `js_string`.
pub fn create_index_command(index: &SuggestedIndex, ns: &Namespace, collation: Option<&CollationOption>) -> String;

// viewmodel/aggregate_plan_next.rs
#[derive(Clone, Debug, PartialEq)]
pub struct PlanNext {
    pub try_index: bool,                 // the plan has CollectionScan or InMemorySort, or a hint is set
    pub current_hint: Option<String>,    // name, or keys_text of a key pattern
    pub blocked: Option<&'static str>,   // "aggregate.explain.disabled.running" | "aggregate.options.blocked"
    pub suggested: Option<SuggestedIndex>,
    pub command: Option<String>,         // create_index_command with the plan's own collation
}
pub fn plan_next(draft: &AggregateState, ns: Option<&Namespace>) -> PlanNext; // all off unless the Plan tab holds a Done plan
pub fn plan_wants_an_index(plan: &AggPlan) -> bool;
// suggestion from done.for_draft and done.options.collation, shown only when plan_wants_an_index

// viewmodel/index_builder_suggested.rs
impl IndexBuilderState {
    /// The builder pre-filled with `index`'s keys (1 Asc, -1 Desc), the default name following them,
    /// and the collation as its JSON text when there is one.
    pub fn from_suggested(ns: Namespace, index: &SuggestedIndex, collation: Option<&CollationOption>) -> Self;
}

// app/aggregate_explain_actions.rs: ExplainAction gains
TryIndex(String), ClearHint, CopyCreateIndex /* the view copied it; nothing changes */, CreateIndex,
// app/aggregate_plan_actions.rs
pub(super) fn try_index(state: AppState, name: String) -> Out; // options Edit(HintPicked) (applies, previews) then explain Run
pub(super) fn clear_hint(state: AppState) -> Out;              // options Edit(HintCleared) then explain Run
pub(super) fn create_index(state: AppState) -> Out;            // open_with(from_suggested(..)) from the Done plan, else nothing
```
`try_index` and `clear_hint` do nothing while `PlanNext::blocked` is set (a loading explain, a blocked form) and chain the commands of the two existing handlers in order (preview first, then explain). The explain uses the verbosity selected now (the toggle on screen).

- [ ] Failing tests in `agg_suggest_tests.rs`: `a_leading_match_gives_equality_then_range` (`{a: 1, b: {$gt: 2}}` -> `a, b`), `a_sort_after_the_match_goes_between` (`$match {a: 1, c: {$lt: 5}}`, `$sort {b: -1}` -> `a 1, b -1, c 1`), `a_leading_sort_alone_suggests_its_keys`, `a_sort_not_directly_after_the_match_is_ignored`, `a_first_stage_that_is_neither_suggests_nothing` (`$group`, `$lookup`, `$geoNear`), `text_search_suggests_nothing`, `an_empty_match_suggests_nothing`, `the_command_without_collation_is_suggested_index_command`, `the_command_carries_the_collation_escaped` (locale with a quote cannot break out).
- [ ] Failing tests in `aggregate_plan_next_tests.rs`: `nothing_before_a_plan`, `a_collection_scan_offers_try_and_the_suggestion`, `an_in_memory_sort_offers_them_too`, `an_index_scan_offers_nothing_unless_a_hint_is_set`, `the_suggestion_comes_from_the_explained_pipeline_not_the_draft` (draft edited after the plan), `the_command_uses_the_plan_s_collation`, `a_running_explain_or_a_blocked_form_turns_try_off_with_its_reason`, `the_current_hint_is_shown_by_name_or_keys`.
- [ ] Failing tests in `index_builder_suggested_tests.rs`: `keys_and_directions_fill_the_rows`, `the_name_follows_the_keys`, `a_collation_fills_its_text_and_parses_in_spec`, `the_builder_spec_is_the_suggested_index` (`spec()` Ok with the same keys).
- [ ] Failing tests in `aggregate_plan_actions_tests.rs` (reducer, connected session, a Done plan with a collection scan, `AppState` built as `aggregate_explain_actions_tests.rs`): `try_index_sets_the_hint_and_sends_preview_then_explain` (commands in order; both carry `HintOption::Name`; form text is the name; `saved` memo forgotten), `try_index_keeps_the_verbosity_selected`, `the_same_index_again_only_re_explains`, `clear_hint_sends_both_without_a_hint`, `try_index_does_nothing_while_an_explain_runs`, `try_index_does_nothing_while_the_form_is_blocked`, `create_index_opens_the_builder_pre_filled` (`state.indexes.builder` keys, `EstimatedCount` sent), `create_index_is_refused_on_a_read_only_connection` (banner `ReadOnly`, no builder, no command), `create_index_on_production_opens_and_create_is_guarded` (Create parks the write warning), `create_index_without_a_suggestion_does_nothing`, `a_created_index_resets_the_hint_list_of_its_collection` (`write_finished` for the selected ns: `wants_list()` again; another ns: untouched).
- [ ] `cargo +1.87.0 test -p curlybrace-core explain` and `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(aggregate): try an index and suggest one from the plan`.

### Task 5: the Plan tab's follow-ups (views), accessibility, snapshots, end to end, README, full check

**Files:** create `crates/ui/src/views/aggregate_plan_next.rs` (+ `aggregate_plan_next_tests.rs`, kittest), `crates/ui/tests/accessibility_aggregate_plan_next.rs`, `crates/ui/tests/local_aggregate_plan_flows.rs`; modify `views/mod.rs`, `views/aggregate_explain.rs` (`show_results(.., context: &RunContext, ..)` and `show_plan_tab` draws the follow-ups under the plan inside its scroll area), `views/aggregate.rs` (passes `run` on), `views/aggregate_run.rs` (`RunContext` gains `pub read_only: bool, pub ns: Option<Namespace>`), `app/browse_panel.rs` (fills them from `state.active_is_read_only()` and `state.selected_ns()`), the `RunContext` literals in `tests/accessibility_aggregate_run.rs` and `tests/accessibility_aggregate_options.rs`, `views/aggregate_explain_tests.rs` (the call), `tests/support/explain_shots.rs` (+ `aggregate_plan_next_tab`: `$match {status: "A"}` + `$sort {total: -1}`, an in-memory-sort plan, hint `status_1` set), `tests/aggregate_snapshots.rs`, `crates/ui/locales/en.json` (`aggregate.options.explain.*`), `README.md`.

**Produces:**
```rust
// views/aggregate_plan_next.rs
pub(super) fn show_plan_next(ui: &mut egui::Ui, draft: &AggregateState, context: &RunContext, i18n: &I18n) -> Vec<AggregateAction>;
```
Layout and controls, in Tab order: a row **Try an index** (combo named `try_index`, selected text `current_hint` or `no_hint`, items the `HintListUi` names, `hint.loading` / `hint.list_failed` in their place; emits `Options(HintListWanted)` while the list `wants_list()`; picking emits `Explain(TryIndex(name))`; hover `try_index_hint`) and **Clear hint** (only with a hint set); both off through `gate` with `blocked` as tooltip and accessible description. Then, with a suggestion: `suggested` heading, the command in a read-only selectable monospace label, **Copy command** (copies `command` with `ctx.copy_text`, emits `CopyCreateIndex`) and **Create index...** (off with `create_index_read_only` as tooltip and description when `context.read_only`). Wraps at 800 pt (`horizontal_wrapped`).

- [ ] Failing tests in `views/aggregate_plan_next_tests.rs`: `a_collection_scan_plan_shows_try_an_index_and_the_suggestion`, `an_index_plan_without_a_hint_shows_nothing`, `picking_an_index_emits_try_index`, `clear_hint_shows_only_with_a_hint_and_emits_clear_hint`, `the_picker_asks_for_the_index_list_once`, `loading_and_failed_lists_say_so`, `try_is_off_with_the_reason_while_an_explain_runs`, `copy_puts_the_command_on_the_clipboard`, `create_index_is_off_with_the_reason_on_a_read_only_connection`, `create_index_emits_create_index`.
- [ ] `accessibility_aggregate_plan_next.rs` (English locale, driven like `accessibility_aggregate_explain.rs`): `every_follow_up_control_has_a_role_and_a_name`, `tab_walks_try_clear_copy_create_in_order`, `the_disabled_create_index_reads_its_reason`, `at_800x500_the_follow_ups_fit_without_overlap` (accesskit bounds inside the window, none overlapping).
- [ ] Snapshots: `aggregate_plan_next` (1000x700), `aggregate_plan_next_dark`, `aggregate_plan_next_small` (`Shot::sized` 800x500). The existing `aggregate_plan` / `aggregate_plan_dark` legitimately change (their fixture plan is a collection scan, so the follow-ups now show): regenerate those two and report it. Write with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test aggregate_snapshots -- --include-ignored --nocapture`, look at each PNG, rerun without `UPDATE_SNAPSHOTS` (every other aggregate PNG unchanged).
- [ ] `local_aggregate_plan_flows.rs` (`#[ignore]` + `local_enabled`, own `cbit_*` database dropped at the end, the `Fixture` of `local_e2e_flows.rs`, standalone throwaway server as in Task 2): `try_an_index_re_explains_through_that_index` (indexes `status_1` and `total_-1`; explain `$match {status: "A"}` + `$sort {total: -1}` on an unindexed field first; pick `total_-1`: the new plan's input names it, Options shows the hint), `clear_hint_explains_without_it`, `create_index_from_the_suggestion_builds_it_and_lists_it` (open, Create, the index exists on the server, the picker lists it after reopening).
- [ ] README, **Aggregate**: replace "Copy as code does not include the options yet." with what the code carries (the options as set, an "Include options" check, the read preference left out when the code keeps `$out`/`$merge`, the read preference on the collection handle or as an operation option per driver); in the **Explain** bullet: Try an index / Clear hint (sets the hint in Options and explains again), the suggested index from the first `$match` and `$sort` (with the draft's collation) with Copy command and Create index... through the index builder, off on read-only connections. Line 333 "Not yet" loses the options; roadmap lines 373/377: the aggregation builder is built, nothing left from #75's aggregate side (find-query options stay open).
- [ ] Full check: `cargo +1.87.0 fmt --all`, `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`, `cargo +1.87.0 test --workspace --locked`; coverage of the new modules at least 80% (`cargo llvm-cov` if installed, else list the tests per public function in the PR body); `wc -l` of every touched file (report `app/mod.rs` before and after; none created over 400); `git diff --stat main -- crates/core/tests/codegen` shows only new files.
- [ ] Commits: `feat(aggregate): offer the plan's follow-ups in the Plan tab` (views, en.json, unit tests), `test(aggregate): accessibility, snapshots and flows of the plan follow-ups` (tests + PNGs), `docs(aggregate): describe options in code and the plan follow-ups`. Report the new i18n keys (the ones beyond the spec named), the regenerated PNGs, and every driver API name still unverified.

## en.json

Under `codegen` (between `note` and `preview`), and one key in `codegen.error` (between `invalid_name` and `key_order`):
```json
"options": {
  "include_options": "Include the pipeline's options",
  "listed": "The code sets: {options}",
  "read_preference_left_out": "This code writes with $out or $merge, which runs on the primary, so it leaves out the read preference."
}
// codegen.error.invalid_options: "The options have a problem. Fix them in Options before you copy code."
```
Under `aggregate.options` (between `comment_sent_note` and `explain_timeout_set`):
```json
"explain": {
  "clear_hint": "Clear hint",
  "copy_create_index": "Copy command",
  "create_index": "Create index...",
  "create_index_read_only": "This connection is read-only. Copy the command to create the index elsewhere.",
  "no_hint": "No hint",
  "suggested": "Suggested index for the first $match and $sort",
  "try_index": "Try an index",
  "try_index_hint": "Sets this index as the hint in Options and explains again."
}
```
Beyond the spec: `codegen.options.listed`, `codegen.options.read_preference_left_out`, `codegen.error.invalid_options`, `aggregate.options.explain.no_hint`, `aggregate.options.explain.try_index_hint`. Written one key per line in the file.

## Calls made here (owner may overrule; defaults are implemented)

1. **Read preference in code that keeps a write stage:** the spec's Copy-as-code column says "as set"; the run pins the primary. Default: left out with the fixed note (code never differs from what Run sends). Overrule: emit it as set.
2. **Where the read preference goes:** the spec says "on the collection handle". Python, Java, C#, Go and Ruby derive the handle inline in the call; mongosh, Node.js, PHP and Rust take it as an operation option (their idiom, and it survives "setup" off, where no handle line exists).
3. **Suggested index with a collation:** `SuggestedIndex::command` has no collation; an index without the query's collation cannot serve its string comparisons. Default: `create_index_command` adds the plan's collation, and the builder is pre-filled with it.
4. **A leading `$sort` alone** (no `$match`) also gets a suggestion; the spec names only `$match` + `$sort`.
5. **Try an index stays visible while a hint is set**, even when the new plan no longer scans, so Clear hint is reachable.
