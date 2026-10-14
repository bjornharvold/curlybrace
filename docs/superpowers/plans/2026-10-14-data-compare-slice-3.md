# Data Compare, Slice 3: Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a person take what a compare found and what they queued out of the app, only on their action and only to a file they choose: a **mongosh script** that applies the pending changes, a **JSON plan** of the same changes, and a **report** of the differences (CSV file or clipboard).

**Architecture:** Document data lives on the servers and in the runtime, not in the UI, so the runtime does the work. A new compare job kind, `Export`, takes the session's job slot, re-reads the documents the pending changes need (reusing the apply engine's *planning and re-verification without the write*), and streams the result to the chosen path in bounded memory, with progress and cancel. The report job streams the stored differences the same way. Renderers (mongosh values, JSON plan, CSV) are pure core functions with exhaustive tests. The UI adds an export state machine in the reducer, two small modals (plan export, report export) and a thin app-level seam for the native file dialog and the clipboard; the file path reaches the reducer as an action, and `CompareCommand::Export` is built in exactly one reducer function.

**Tech Stack:** Rust, `bson` 2.15, `serde_json` (preserve_order, in core), tokio `fs`, egui/eframe 0.32 (`ctx.copy_text`), `rfd` (native file dialog; new dependency, see ruling 1), the slice 2a-2c apply pipeline.

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` ("Export (slice 3, designed now)", "Security and privacy", "Slice 2a/2b/2c as built"). Issue #8. Slices 2a, 2b, 2c shipped in PRs #36, #39, #41.

## Decisions made while planning (rulings; each states the cost if wrong)

1. **New dependency: `rfd` (native save dialog)**, used only in `curlybrace-ui`'s app layer behind a small `FilePicker` trait so tests inject a fake and the reducer never sees it. There is no file picker in the repo today. Alternative if you veto it: a typed-path text box in the modal (no dependency, worse UX); the rest of the plan is unchanged because the path arrives as an action either way. Cost if wrong: swap the picker implementation.
2. **The runtime writes the file; the UI never holds document data.** `CompareCommand::Export { session, run, request, path }` makes core read documents and write to `path` itself. Only counts, the byte size and the path travel back (`ExportProgress`, `ExportDone`). The one exception is the clipboard report, which is limited to 10,000 rows and returned as text in `ExportText` for the UI to copy (no disk). Cost if wrong: none.
3. **Never a temp file, and never a half-written plan left behind.** The file is created at the chosen path (truncating; the OS dialog already asked about overwriting), with owner-only permissions (0600; the contents are document data), written through a buffered writer. If the job fails or is cancelled, core **removes the partial file** and says so in the event; a truncated mongosh script must not be runnable. A path that is relative, has no existing parent directory, or is a directory is refused up front before the slot is taken.
4. **Export reuses the apply planning, not a copy of it.** `plan_batch` and its inputs become `pub(crate)`; the export driver runs the *same* verification (primary re-read of both documents, both fingerprints equal the recorded ones, kind fits, fresh diff for field ops, scope-safe `plan_fields`) and gets the same `WriteOp`s without writing. Operations that would be skipped by an apply (changed since the compare, not applicable) are **listed as comments in the script and as `skipped` entries in the JSON** with their reason, never silently dropped and never exported as writes. Destination rules (view refused, time-series inserts only) are checked first, as for an apply. Cost if wrong: an exported script could contain an operation an apply would have refused.
5. **The mongosh script is rendered with an exhaustive BSON renderer, not `EJSON.parse`.** One `bulkWrite([...], { ordered: false })` statement per target collection, in pair order, on `db.getSiblingDB("<db>").getCollection("<name>")`; each operation is preceded by a comment with the fingerprints (hex) and the key; operations are `insertOne`, `replaceOne` (filter `{ _id: ... }`), `deleteOne` (filter `{ _id: ... }`) and `updateOne` (`$set`/`$unset` with dotted paths rendered from the segments, refusing the same unaddressable names the store refuses). Every BSON type renders with its shell constructor (`ObjectId("...")`, `ISODate("...")`, `NumberLong("...")`, `NumberInt(..)`, `NumberDecimal("...")`, `Timestamp(t, i)`, `BinData(subtype, "base64")`, `UUID("...")`, `/re/flags`, `MinKey()`, `MaxKey()`, `Code("...")`, DBRef as a document, `undefined`, `null`, doubles including `NaN`/`Infinity`/`-0`). Strings are written as JSON string literals (valid JS). A statement is one expression, so a truncated file can never apply half of a collection (and ruling 3 deletes truncated files anyway). Cost if wrong: a value the shell cannot parse; a tier-1 test runs the script through the real `mongosh` of a throwaway `mongo:7` container for every type.
6. **The JSON plan** is `{ "header": {...}, "pairs": [ { "source": {db, collection}, "target": {db, collection}, "key": [fields], "operations": [...] } ] }` streamed one operation at a time. Documents and values are canonical extended JSON; field paths are arrays of segments (a string for a field, a number for an array index), never a dotted string; each operation carries `op` (`insert|replace|delete|updateFields|skipped`), `direction`, the key, both fingerprints (hex or null), and for skipped ones the `reason`. `header` has `format` (`curlybrace-compare-plan`), `version` (1), the counts, the id policy and the free-text label.
7. **No secrets, hosts or connection names in any output.** Headers name databases and collections only, plus the label. The label is **free text from the person**: CR/LF and other control characters are stripped before it goes into a script comment or CSV comment (a label must never be able to inject script text); JSON escapes it. Names that go into comments are sanitized the same way; names inside code are JSON-string-escaped. Cost if wrong: injection into a script people run; pinned by tests with hostile labels and names.
8. **Report = one CSV of the differences** with header `pair,source,target,category,key,changed_paths`; one row per stored difference of the chosen pairs (streamed from the stored results in key order), `category` is `only_in_source|only_in_target|different`, `key` is the key tuple rendered as relaxed extended JSON, `changed_paths` is filled only when the option "include changed field paths" is on (default off: it re-reads both documents of every different row, serially, with progress and cancel; it writes paths only, never values). The **summary** (counts per category, identical, scanned per side, per pair and total) is written first as `#`-prefixed comment lines (documented; the per-pair duration is omitted because `Summary` has none). CSV is RFC 4180 quoting; cells starting with `=`, `+`, `-`, `@`, tab or CR are prefixed with a single quote so spreadsheets do not run them as formulas. Clipboard: same text, only up to 10,000 rows; above that the UI says to export to a file. Cost if wrong: none.
9. **Export gating.** Export needs a connected session with finished pairs. While an export runs: no Start, Check, Re-run, Remove, Discard, Find duplicates, Apply or Review, and no closing/replacing the session (same busy rule as an apply: one job per session; core refuses with the same "a comparison is already running" text). Plan export is disabled when the pending list is empty; report export when no selected pair has a finished result. Export is read-only on the databases and does not mark pairs applied.
10. **Warnings in both modals (text, never blocking):** the file will contain document data (plan) or keys, which can be personal data (report); the plan is a snapshot: documents may have changed since; operations skipped at export time are listed in the file. The "Security and privacy" lines of the spec are rewritten: *no document data is written to disk except by an explicit export to a file the person chooses.*
11. **Out of scope:** scheduled exports, exporting the Identical category, per-row selection for the report (the report covers whole pairs; pick pairs in the dialog), BSON/dump formats (see #38), a "restore from plan" import. Cost if wrong: later slice.

## Global Constraints

- **Only on the person's action.** `CompareCommand::Export` is built in exactly one reducer function (`start_export`), reachable only from the `ExportPathChosen` action in state `Exporting::Choosing`; the exhaustive no-write test gains the new actions and asserts none of them sends `Apply`, and a new exhaustive test asserts nothing but `start_export` sends `Export`.
- Keys, documents and field values are data: never logged, never in error text. Events carry counts, byte sizes and the chosen path (a path can contain a user name: show it in the UI, never log it).
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical, new group `compare.export.*`). No literal English in code; pseudo-locale frame tests for every new view; `i18n_usage_tests` rules apply (keys as string literals in production code).
- Immutability; files < 800 lines (aim 200-400); functions < 50 lines; nesting <= 4. New core code in `crates/core/src/compare/export/` (`mongosh.rs`, `json_plan.rs`, `csv.rs`, `sanitize.rs`, `plan.rs`, `report.rs`) and `runtime/compare/export.rs`; `app/compare.rs` (566 lines) must not grow past 620: new reducer code in `app/compare_export.rs`.
- Default `cargo test --workspace --locked` stays DB-, docker- and GPU-free. Tier-1 tests are `#[ignore]` + env-gated, need an explicit `CURLYBRACE_IT_MONGODB_URI` (empty/whitespace = unset), use only `cbit_<id>` databases and only a `cb-it-dev-*` container the test run starts itself (`cb-it-dev-3`, port 27122), never the user's `local7979`/`local265` or `127.0.0.1:27017`. Running `mongosh` happens only inside that own container (`docker exec -i cb-it-dev-3 mongosh ...`). GitHub CI never needs MongoDB (`scripts/check-no-db-in-ci.sh` stays green); the new `rfd` dependency must not break headless CI builds.
- Commits: conventional format ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Before every push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Sub-agents default to Sonnet; Tasks 1-4 (they write files with user data and generate scripts people run) and 5 (the state machine) and the final review use Opus reviewers. Implementers write files with the Write/Edit tools and keep temp files in the scratchpad directory, never `/tmp`.

## Review Focus

1. **Script injection and wrong values.** Hostile labels, database/collection names, string values (quotes, backslashes, newlines, U+2028/2029, `</script>`-like text, NUL), keys, field names; every BSON type; `NaN`/`-0`/`Infinity`; huge numbers; deeply nested documents; a statement that is one expression. Pinned in Tasks 1, 2, 9 (the real `mongosh` check).
2. **A plan that differs from what an apply would do.** Skipped/stale operations exported as writes; view/time-series destinations; id policy; direction; fields ops; bulk items expanded identically. Pinned in Tasks 2 and 3.
3. **Files.** Partial file removal on cancel/failure/panic; permissions; refused paths (relative, directory, missing parent, symlink?); overwriting; no temp file. Pinned in Task 3.
4. **Leaks.** Connection names, hosts, URIs, secrets in headers; keys in logs; the clipboard cap; spreadsheet formula injection in the CSV. Pinned in Tasks 1, 4, 9.
5. **UI gating and honesty.** Busy rules, disabled reasons, the path-chosen action only in the right state, results that say what was written and what was skipped. Pinned in Tasks 5-7.

## File Structure

```
crates/core/src/compare/export/
  sanitize.rs     NEW  comment/label sanitising, JS string literal, CSV cell escaping          (Task 1)
  mongosh.rs      NEW  BSON -> shell value renderer, script writer                              (Task 1)
  csv.rs          NEW  CSV rows and summary comments                                           (Task 1)
  json_plan.rs    NEW  streaming JSON plan writer                                              (Task 1)
  plan.rs         NEW  export planning over the apply planner (verified ops + skips)           (Task 2)
  report.rs       NEW  stored results -> CSV rows (+ optional changed paths)                    (Task 4)
  mod.rs          NEW
crates/core/src/compare/apply_engine.rs   plan_batch/Planned/ApplyInput pub(crate)               (Task 2)
crates/core/src/compare/protocol.rs       CompareCommand::Export, ExportRequest, events           (Task 3)
crates/core/src/runtime/compare/export.rs NEW  job, slot, file lifecycle, events                  (Tasks 3-4)
crates/ui/src/viewmodel/compare/export.rs NEW  ExportUi state, options, gates, result             (Task 5)
crates/ui/src/app/compare_export.rs       NEW  actions/events, start_export                       (Task 5)
crates/ui/src/app/{mod,panels}.rs         FilePicker seam, UiEvent::PickExportFile, clipboard     (Task 6)
crates/ui/Cargo.toml                      + rfd                                                    (Task 6)
crates/ui/src/views/compare/export.rs     NEW  modals and result                                  (Task 7)
crates/ui/tests/{snapshots,accessibility_export,local_compare_export_flows}.rs, support/*          (Tasks 7-9)
crates/core/tests/local_compare_export.rs NEW tier-1 incl. real mongosh                           (Task 9)
docs / README / site                                                                              (Task 9)
```

---

### Task 1: Pure renderers: sanitising, shell values, script, CSV, JSON plan (Opus review)

**Files:**
- Create: `crates/core/src/compare/export/{mod,sanitize,mongosh,csv,json_plan}.rs` (+ `*_tests.rs` via `#[path]` for the big tables); modify `compare/mod.rs`

**Interfaces:**
- Produces (all pure, no I/O except through `std::io::Write`):
  ```rust
  // sanitize.rs
  pub fn comment_text(s: &str) -> String;        // strips CR/LF/control chars and U+2028/2029; collapses to one line; caps at 200 chars
  pub fn js_string(s: &str) -> String;           // a valid JavaScript string literal (JSON escaping + U+2028/2029 + NUL as \u0000)
  pub fn csv_cell(s: &str) -> String;            // RFC 4180 quoting + leading ' for = + - @ \t \r
  // mongosh.rs
  pub fn render_value(v: &Bson) -> Result<String, RenderError>;       // exhaustive over Bson
  pub fn render_document(d: &Document) -> Result<String, RenderError>;
  pub fn render_path(segments: &[PathSegment]) -> Result<String, RenderError>;   // dotted update path; same refusals as the store (empty/'.'/'$' names, _id, AnyElement)
  pub enum ShellOp { Insert{doc}, Replace{id, doc}, Delete{id}, UpdateFields{id, set, unset} }  // mirrors WriteOp
  pub struct ScriptWriter<W: Write> { .. }   // begin(header), collection(db, name), op(comment_fp, ShellOp), end_collection(), finish()
  // csv.rs
  pub struct CsvWriter<W: Write> { .. }      // summary comments, header row, row(...)
  // json_plan.rs
  pub struct PlanWriter<W: Write> { .. }     // header, pair(source, target, key), operation(PlanOp), finish() — streams valid JSON
  pub struct ExportHeader { pub label: String, pub counts: ExportCounts, pub id_policy: IdPolicy }
  ```
  `RenderError` is value-free (`Unrenderable`, `BadName`). Output of every writer is deterministic.

- [ ] **Step 1: Failing tests** (pure):
  - `sanitize`: labels with CR/LF/tabs/U+2028/NULs/very long text become one safe line; `js_string` round-trips quotes, backslashes, newlines, U+2028/2029, NUL, emoji, `*/` and `</script>`; `csv_cell` quoting (commas, quotes, newlines) and formula prefixes for every trigger char, no double prefix, empty string.
  - `mongosh::render_value` for **every** `Bson` variant (Double incl. NaN/±Infinity/-0.0, String, Array, Document, Boolean, Null, RegularExpression incl. flags and `/` in the pattern, JavaScriptCode, JavaScriptCodeWithScope, Int32, Timestamp, Int64 beyond 2^53, ObjectId, DateTime incl. pre-1970 and out-of-range years, Symbol, Decimal128, Undefined, MaxKey, MinKey, DbPointer, Binary subtypes incl. UUID (subtype 4) and generic with base64); nested documents/arrays; field names needing quotes (`"a b"`, `"$x"`, numeric-looking) are always written as JSON string keys.
  - `render_path`: table incl. refusals and `Index` segments.
  - `ScriptWriter`: golden-file test of a full script (two collections, all four op kinds, a skipped-op comment, hostile label and hostile collection name `x"); db.dropDatabase(); //`): the hostile text appears only inside a string literal or a sanitized comment; exactly one `bulkWrite` statement per collection with `{ ordered: false }`; fingerprint comments present; no `const`/`let` leakage between statements.
  - `CsvWriter`: golden test (summary comment lines first, header row, rows with commas/newlines/formula-looking keys/unicode, empty `changed_paths`), the summary has no durations.
  - `PlanWriter`: output parses with `serde_json` for zero, one and many operations and for skipped ones; keys/documents are canonical extended JSON; paths are arrays of strings/numbers; header fields; hostile label is escaped; bytes identical across two runs.
- [ ] **Step 2: Run** — `cargo test -p curlybrace-core compare::export -- --locked` → FAIL.
- [ ] **Step 3: Implement.** Every function < 50 lines (split `render_value` per variant group).
- [ ] **Step 4: Run** → PASS (+ whole workspace).
- [ ] **Step 5: Commit** — `feat(compare): renderers for mongosh scripts, JSON plans and CSV reports`

---

### Task 2: Export planning over the apply planner (Opus review)

**Files:**
- Create: `crates/core/src/compare/export/plan.rs` (+ tests)
- Modify: `crates/core/src/compare/apply_engine.rs` (visibility only, plus a `PlannedOp` record so each planned write or skip is tied to its op, row fingerprints and reason)

**Interfaces:**
- Consumes: `ApplyInput`, `plan_batch`, `Planned`, `ApplyOp`, `OpOutcome`, `SkipReason`, `WriteOp`, `DiffEntry`.
- Produces:
  ```rust
  pub enum ExportItem {
      Write { op: ApplyOp, write: WriteOp, source_fp: Option<Fingerprint>, target_fp: Option<Fingerprint> },
      Skipped { op: ApplyOp, reason: SkipReason, source_fp: Option<Fingerprint>, target_fp: Option<Fingerprint> },
      Failed { op: ApplyOp, error: CoreError },          // a read error: value-free
  }
  /// Plans (verifies, never writes) the ops of ONE pair in batches; `sink` receives each batch's items in op order; cancel is checked before each batch.
  pub async fn plan_pair_export(input: ApplyInput<'_>, cancel: &CancelFlag, sink: &mut dyn FnMut(Vec<ExportItem>) -> Result<()>, progress: &mut dyn FnMut(u64)) -> Result<ExportPlanCounts>;
  ```
  `ExportPlanCounts { written, skipped_stale, skipped_not_planned, failed, not_run }` (sums to the ops requested, like `ApplyReport`). The destination side of each `WriteOp` follows the direction exactly as in `apply_batch`.

- [ ] **Step 1: Failing tests** (InMemoryStore + real `run_pair` ResultSet, like the engine tests): a plan equals the writes an apply would perform for the same ops on identical stores, for insert, replace, delete, fields ops and mixed batches (run both pipelines on cloned stores and compare the `WriteOp`s against what the spy store received); stale documents (either side) become `Skipped(Stale)`; non-fitting kinds `Skipped(NotPlanned)`; duplicate keys, unaddressable fields, scope-unsafe field paths behave exactly as in apply; **no write is ever issued** (a store whose `bulk_write` panics); cancel between batches reports `not_run`; counts add up to the ops requested; fingerprints on items equal the recorded entry's; batch size 500 boundaries (0, 1, 500, 501 ops); a read error becomes `Failed` without stopping the batch.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** (refactor `apply_batch` to share the planning step; behaviour of apply unchanged, all existing apply tests untouched and green); **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(compare): plan an export with the apply planner, without writing`

---

### Task 3: Runtime export job for plans: command, events, file lifecycle (Opus review)

**Files:**
- Create: `crates/core/src/runtime/compare/export.rs` (+ `export_tests.rs`)
- Modify: `compare/protocol.rs`, `runtime/compare/{mod,session}.rs` (routing, `command_context` `"compare_export"`), `crates/ui/locales/en.json` (`failures.compare_export`), the UI exhaustive matches (explicit no-op arms with a `// slice 3` comment)

**Interfaces:**
- Produces:
  ```rust
  pub enum ExportFormat { Mongosh, JsonPlan }
  pub struct ExportRequest { pub format: ExportFormat, pub label: String, pub id_policy: IdPolicy,
                             pub ops: Vec<ApplyOp>, pub bulk: Vec<BulkApply> }       // same shapes as Apply
  // CompareCommand::Export { session, run, request: ExportRequest, path: PathBuf }
  // CompareEvent::ExportProgress { session, run, done, total }
  // CompareEvent::ExportDone { session, run, outcome: ExportOutcome }
  pub struct ExportOutcome { pub written: u64, pub skipped: u64, pub failed: u64, pub not_run: u64, pub bytes: u64,
                             pub cancelled: bool, pub partial_removed: bool }
  ```
  Behaviour: synchronous validation like `start_apply` (session open and connected, empty request refused, `expand_bulk`, `refuse_misshapen`, `refuse_duplicates`, results present, **path validation**: absolute, parent exists and is a directory, the path is not an existing directory, refused otherwise with `Unsupported("invalid export path")` before the slot is taken), takes the slot (shared with compare runs and applies), then in the task: `check_destinations` (view/time-series rules), create the file (0600, truncate), write header, per pair (first-appearance order) `plan_pair_export` into the format writer from Task 1 (script: one `bulkWrite` per target collection; JSON: one `pairs[]` entry per pair), flush, emit `ExportDone`. Cancel, failure, panic or a write error: remove the partial file, `partial_removed = true`, `cancelled` per the flag. A refusal after the slot was taken (`check_destinations`) emits `Failed{compare_export}` and releases the slot first (the 2a rule). No temp file is ever created.

- [ ] **Step 1: Failing tests** (InMemory session; temp directories only via `tempfile` inside the test, removed by the test): exports a script and a JSON plan whose content equals the planned ops (parse the JSON; for the script compare to the golden output of Task 1 for the same items); refusals leave no file (relative path, missing parent, a directory, empty request, unknown pair, view destination); cancel mid-export removes the partial file and reports `partial_removed`; a panicking store removes the file and still ends with `ExportDone`; closing the session mid-export emits nothing more and the file is removed; a second `Export`/`Start`/`Apply` while one runs is refused and a following one is accepted after it ends; file mode is 0600 (unix); an existing file at the path is truncated and replaced; the written file contains no connection names or URIs (the request does not carry any, assert on bytes); a hostile label cannot add a line to the script; event order `ExportProgress*` then exactly one `ExportDone`; a write error (read-only directory) ends with `Failed{compare_export}` or an outcome with the error, no partial file, and the slot is freed.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** (functions < 50 lines; a `FileGuard` that removes the file on drop unless `commit()` is called); **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(runtime): export pending changes as a mongosh script or JSON plan`

---

### Task 4: Runtime report job: CSV file and clipboard text (Opus review)

**Files:**
- Create: `crates/core/src/compare/export/report.rs`; extend `runtime/compare/export.rs` and `protocol.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct ReportRequest { pub pairs: Vec<PairId>, pub include_paths: bool, pub label: String }
  // CompareCommand::ExportReport { session, run, request: ReportRequest, target: ReportTarget }
  pub enum ReportTarget { File(PathBuf), Clipboard }
  // CompareEvent::ExportDone as above (rows in `written`), plus CompareEvent::ExportText { session, run, text: String } for Clipboard
  ```
  Behaviour: validation as in Task 3 (pairs must have stored results; the clipboard target is refused with `Unsupported("report too large for the clipboard")` before the slot is taken if the selected pairs hold more than 10,000 differences). Per pair in the request order, rows stream from the stored results (all three categories, key order) through `CsvWriter` (summary comment block first, built from the pair's counts and the stored `Summary` data the runtime keeps or recomputes from `ResultSet::count`; no durations). `include_paths`: for every *different* row re-read both documents (`fetch_unique` primary, both sides), normalize with the pair's rules, `diff_normalized`, write the rendered paths joined with `; ` (paths only, never values); a row whose documents changed or cannot be read gets an empty cell and is counted in `failed`/`skipped` in the outcome; cancel checked every batch of rows. File lifecycle exactly as in Task 3 (0600, removed on failure/cancel).

- [ ] **Step 1: Failing tests:** rows equal the stored differences for every category and for two pairs (order, escaping with commas/quotes/newlines/formula-looking keys); the summary comment block counts match `ResultSet::count`; `include_paths` fills paths for different rows only and never contains values (assert a sentinel value string is absent); stale/removed documents leave the cell empty and are counted; clipboard over 10,000 rows refused up front, under it returns `ExportText` identical to what the file would contain; unknown or discarded pair refused; no connection names or hosts in the output; cancel removes the partial file; accounting; a pair with 0 differences yields a header and a summary only.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement**; **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(runtime): export a report of the differences as CSV or clipboard text`

---

### Task 5: UI export state machine and reducer (Opus review)

**Files:**
- Create: `crates/ui/src/viewmodel/compare/export.rs` (+ tests), `crates/ui/src/app/compare_export.rs` (+ `compare_tests/export.rs`); modify `viewmodel/compare/mod.rs`, `app/compare.rs` (route only), `compare_tests/no_write.rs`

**Interfaces:**
- Produces:
  ```rust
  pub enum ExportKind { Plan, Report }
  pub struct PlanOptions { pub format: ExportFormat, pub label: String, pub id_policy: IdPolicy }
  pub struct ReportOptions { pub pairs: BTreeSet<EntryId>, pub include_paths: bool, pub label: String }
  pub enum ExportUi {
      Idle,
      Configuring { kind: ExportKind, plan: PlanOptions, report: ReportOptions },        // the modal
      Choosing   { .. same data .. },                    // the file dialog is open (waiting for ExportPathChosen or ExportPathCancelled)
      Running    { run: RunId, kind, total: u64, done: u64, cancel_requested: bool },
      Finished   { outcome: ExportOutcome, path_label: String, kind: ExportKind },
      Copied     { rows: u64 },                           // clipboard report text handed to the UI
  }
  // CompareAction: OpenExport(ExportKind), SetExportFormat, SetExportLabel(String), SetExportIdPolicy, ToggleExportPair(EntryId), SetExportIncludePaths(bool), ChooseExportFile, ExportPathChosen(PathBuf), ExportPathCancelled, CopyReport, CancelExport, CloseExport
  ```
  Rules: `OpenExport(Plan)` only when `pending` is non-empty and gates allow (session ready, no compare run/check, no apply review/run, no export busy); `OpenExport(Report)` only when at least one selected-able pair is completed; `ChooseExportFile` moves `Configuring -> Choosing` and the app layer (Task 6) opens the dialog; **`ExportPathChosen(path)` is honoured only in `Choosing` and is the only transition that builds `CompareCommand::Export`/`ExportReport`** (`start_export`), with the reviewed options, a fresh `run` from `next_run`, the pending list's `to_command_parts` (plan) or the chosen entries' current `PairId`s (report, completed pairs only); `CopyReport` builds `ExportReport{ target: Clipboard }` directly from `Configuring` (no file) and its `ExportText` answer is turned into a UI effect (Task 6); busy rules like apply (Start, Check, Rerun, Remove, Discard, FindDuplicates, Apply/Review, CloseSession, Open refused while `Choosing` or `Running`); `CancelExport` sends one `Cancel{run}`; events fold only for the matching run and current session; `Failed{compare_export}` raises the failure banner and returns to `Configuring` (options kept); session drop -> `Idle`; the label is stored sanitised for display only (core sanitises again).

- [ ] **Step 1: Failing tests** (reducer): the exhaustive **no-export-command test** (every `CompareAction` variant in every `ExportUi` state sends no `Export`/`ExportReport` command except `ExportPathChosen` from `Choosing`; a second `ExportPathChosen` sends nothing; `ExportPathChosen` in other states sends nothing) built like `no_write.rs` (compile-time exhaustive chain, no wildcards); the existing no-write test extended with the new actions (none sends `Apply`); `OpenExport` gates (empty pending, run active, check in flight, apply busy, export busy, session not ready); `ChooseExportFile` only from `Configuring`; command content equals the options (plan: parts equal `to_command_parts`, format, label, id policy; report: only completed pairs, include_paths); `ExportPathCancelled` returns to `Configuring`; events: progress/done fold for the matching run, stale run/session ignored, `Failed` handling, `ExportDone` with `partial_removed` shows in `Finished`; busy gates refuse the listed actions in `Choosing` and `Running` (positive controls after `CloseExport`); closing the session refused while busy; the clipboard flow produces exactly one UI effect.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement**; **Step 4: Run** → PASS (+ workspace).
- [ ] **Step 5: Commit** — `feat(ui): export state machine and reducer`

---

### Task 6: App seam: file dialog and clipboard

**Files:**
- Modify: `crates/ui/Cargo.toml` (`rfd`), `crates/ui/src/app/{mod,panels}.rs`, `Cargo.lock`
- Create: `crates/ui/src/app/file_picker.rs` (+ tests)

**Behaviour:** `pub trait FilePicker: Send + Sync { fn save_file(&self, suggested_name: &str, extension: &str) -> Option<PathBuf>; }` with an `rfd` implementation (blocking call run on a worker thread via `std::thread::spawn`, result sent through an `mpsc` channel that `pump()` drains, followed by `repaint`); the app holds an `Arc<dyn FilePicker>` (injectable for tests). New `UiEvent::PickExportFile` (emitted by the view on the `ChooseExportFile` action together with the action) makes `CurlybraceApp` start the picker; a chosen path dispatches `Input::Compare(CompareAction::ExportPathChosen(path))`, a cancelled dialog `ExportPathCancelled`. Suggested names come from i18n-free constants (`curlybrace-plan.js`, `curlybrace-plan.json`, `curlybrace-report.csv`). Clipboard: a UI effect from the reducer (`ExportText` -> `ctx.copy_text(text)`), after which the reducer state is `Copied`. Only one dialog can be open (a second request while one is open is ignored).

- [ ] **Step 1: Failing tests:** with a fake picker: `PickExportFile` returns a path -> `ExportPathChosen` is dispatched exactly once; cancelled -> `ExportPathCancelled`; a second `PickExportFile` while a dialog is pending is ignored; the clipboard effect copies exactly the text (use egui's output `copied_text` in a headless context); the picker is never called by the reducer (grep-level assertion in a unit test is not needed: the reducer has no access to it).
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement**; verify the headless CI build (`cargo build --workspace --locked` on Linux CI features: `rfd` must not require GTK on the CI runner: choose the `xdg-portal` or `gtk3`-free feature set and say so in the report; if the dependency cannot build headless, gate it behind `cfg(target_os = "macos")` for now and make the fallback implementation a typed-path request that returns `None`, documented). **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(ui): native file dialog and clipboard seam for exports`

---

### Task 7: Views: export modals, progress, result

**Files:**
- Create: `crates/ui/src/views/compare/export.rs`; modify `views/compare/{pending,overview,mod}.rs`, `locales/en.json`
- Test: new `crates/ui/tests/accessibility_export.rs`; frames in `app/compare_tests/frames_export.rs`

**Behaviour:** the pending panel gets **Export changes…** (`OpenExport(Plan)`, disabled with a reason: empty pending list, run/check active, apply busy, export busy); the overview totals area gets **Export report…** (`OpenExport(Report)`, disabled with reasons). The modal (`egui::Modal`, id `compare_export`): *Plan*: format radios (mongosh script / JSON plan), label text box (single line, shown as typed; length-capped), id-policy radio when the pending list has inserts, the counts of the pending list, warnings (contains document data; snapshot may be out of date; skipped operations are listed), **Choose file and export** (`ChooseExportFile`), **Cancel** (`CloseExport`). *Report*: pair checkboxes (completed pairs, all ticked by default), "include changed field paths" checkbox with a note about the cost, label, warnings (keys can be personal data), **Choose file and export**, **Copy to clipboard** (disabled with a reason above 10,000 rows, using the stored counts), **Cancel**. While `Choosing`: the modal shows "waiting for the file dialog" and **Cancel**. While `Running`: progress bar `done/total`, Cancel (`CancelExport`, once). `Finished`: what was written (counts of written/skipped/failed/not run, bytes, the path as data), a note when cancelled or when the partial file was removed, Dismiss. `Copied`: "N rows copied", Dismiss. No keyboard shortcut starts an export (Enter in the label field does nothing).

- [ ] **Step 1: Failing tests:** kittest: each gate shows the button disabled with its reason; modal options emit the right actions (format, label typed, id policy only with inserts, pair toggles, include-paths); Choose file emits `ChooseExportFile` once and Enter in the label field emits nothing; Copy disabled over the cap; Cancel/Escape emit `CloseExport`; running shows progress and Cancel; finished panel counts add up and shows the partial-file note; accessible names for every control; pseudo-locale frames cover all states (`assert_translated`); `i18n_usage_tests` green.
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** (+ `compare.export.*` keys); **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(ui): export dialogs for plans and reports`

---

### Task 8: Snapshots and visual review

**Files:** `crates/ui/tests/support/compare_export_states.rs`, `crates/ui/tests/snapshots.rs` (entries stay in the one `snapshots` target), new PNGs

- [ ] **Step 1:** builders through the real reducer: `export_plan_modal`, `export_report_modal` (pairs, include-paths, clipboard cap disabled reason), `export_choosing`, `export_running`, `export_finished_with_removal`, `pending_panel_with_export_button`.
- [ ] **Step 2:** add the GPU-gated `shot!` entries, generate baselines locally, **inspect every new PNG** (truncation, clipped text, long paths wrapping in the result, disabled reasons readable), fix layout problems found, regenerate; list defects found and fixed. Delete a PNG before regenerating when only a few pixels changed. Run the full local snapshot tier and the default suite.
- [ ] **Step 3: Commit** — `test(ui): snapshots for the export dialogs`

---

### Task 9: Tier-1 export flows (real server and real `mongosh`), docs, landing page

**Files:**
- Create: `crates/core/tests/local_compare_export.rs`, `crates/ui/tests/local_compare_export_flows.rs`; modify the spec, `docs/superpowers/notes/testing.md`, `README.md`, `site/index.html`

- [ ] **Step 1: Tier-1 core tests** (`#[ignore]`, `CURLYBRACE_IT=1` and an explicit non-empty `CURLYBRACE_IT_MONGODB_URI`, own container `cb-it-dev-3` port 27122, `cbit_` databases only, temp dirs via `tempfile`): `exported_script_applies_the_same_changes_as_an_apply` (compare, export the script, then run it with the container's `mongosh` through `docker exec -i cb-it-dev-3 mongosh --quiet <uri> < script`, and assert the target equals the result of an in-process apply on a copy); `script_round_trips_every_bson_type` (a collection with one document of every BSON type, copied to an empty target via the script; compare again: zero differences; types preserved incl. Int32/Int64/Double/Decimal128/Date/ObjectId/BinData/UUID/regex/Timestamp/min/max keys, NaN); `script_with_hostile_names_and_values_runs_without_side_effects` (collection and label with quotes/semicolons/comments; the script runs and only the intended collection changes); `field_ops_in_a_script_update_only_the_chosen_fields`; `plan_json_parses_and_matches_the_planned_ops`; `report_csv_matches_the_stored_differences` (parse with a CSV reader written in the test; formula-looking keys are quoted/prefixed); `report_with_paths_lists_paths_and_no_values`; `cancel_removes_the_partial_file` (large insert set); `stale_documents_are_listed_as_skipped_and_not_exported_as_writes`. UI tier-1 (`local_compare_export_flows.rs`, gestures with a fake `FilePicker` returning a temp path): export a plan from the pending list and a report from the overview, assert the files exist, the finished panel text, and that nothing was written to the databases.
- [ ] **Step 2: Docs.** Spec: "Slice 3 as built" under Export (the rulings; the command shapes; the three outputs; the file lifecycle; sanitising; the clipboard cap; limits: no per-row report selection, no Identical rows, no import/restore, durations omitted because `Summary` has none, `include_paths` is slow on big result sets, clipboard limited to 10,000 rows, the script is a snapshot of an earlier comparison); **rewrite the "No document data is written to disk" statements** (decision 5 of the summary list and the first bullet of "Security and privacy") to: *no document data is written to disk except by an explicit export to a file the person chooses*; update the slice list and the 'Not documented there' table. `docs/superpowers/notes/testing.md`: the new tier-1 files, the `docker exec` mongosh usage, container `cb-it-dev-3`. README and `site/index.html`: compare can export the pending changes as a mongosh script or JSON plan and the differences as a CSV report (clipboard or file), with the warnings; the roadmap line "Still to build" is removed from the compare entry; keep "New in the next release" wording until a release is cut; no Studio 3T assets or claims beyond what is built.
- [ ] **Step 3: Verify** — fmt, clippy `-D warnings`, `cargo test --workspace --locked`, `scripts/check-no-db-in-ci.sh`, the tier-1 files against the own container (run twice), `docker ps` shows only `local7979` and `local265` afterwards.
- [ ] **Step 4: Commit** — `test(compare): tier-1 export flows with real mongosh; docs for slice 3`

---

## Self-review

- **Spec coverage (Export section):** mongosh script with one `bulkWrite` per target collection and fingerprints as comments (Tasks 1-3), JSON `{header, pairs:[{source,target,operations}]}` with canonical extended JSON (Tasks 1-3), report with pair/key/category/changed paths as CSV or clipboard plus summary (Tasks 1, 4), files only to a chosen path and never a temp file (Task 3 + rfd seam in Task 6), no credentials/hosts (ruling 7, Tasks 1, 3, 4, 9), data warning in the UI (Task 7), all on the person's action (Task 5, the no-export-command test). **Deviations recorded:** duration omitted from the summary (no duration exists); report is per pair, not per row; clipboard capped.
- **Placeholders:** none; where a test says "like no_write.rs" the pattern exists.
- **Type consistency:** `ExportRequest`/`ReportRequest`/`ExportOutcome` (Tasks 3-4) are built only by `start_export` (Task 5) and shown by Task 7; `ExportItem`/`plan_pair_export` (Task 2) feed the writers of Task 1 in Task 3; `FilePicker` (Task 6) produces only `ExportPathChosen`/`ExportPathCancelled` (Task 5).
