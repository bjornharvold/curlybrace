# Export documents, slice 1 (PR 1 of #54) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Export a whole collection, or the documents matching the applied filter, to an NDJSON or JSON-array file the person chooses, in canonical (default) or relaxed extended JSON, streamed with progress and cancel.

**Architecture:** A new core job (`Command::Export` / `Event::Export`, own slot) reads documents by `_id` with `scan_sorted` in batches, renders each with the extended-JSON renderer, and writes through the shared `runtime/export_file` module from PR 0. The UI adds a modal that mirrors the compare export (configure, choose file, run, finish), opened from the collection context menu and the documents toolbar.

**Tech Stack:** Rust 1.87 (CI pin; verify with `cargo +1.87.0`; no let-chains), tokio, `bson` 2.15 / `serde_json` (extended JSON), egui + egui_kittest, `tempfile` (tests).

**Spec:** `docs/superpowers/specs/2026-11-25-export-documents-design.md` (PR 1 and "Decisions I made" 1 to 6). PR 0 is merged (#202); CSV and selected documents are PRs 2 and 3 with their own plans.

## Global Constraints

- Defaults: **NDJSON** and **canonical** extended JSON. Relaxed is labelled "readable, not type-exact".
- Documents come out in ascending `_id` order and whole: the query's sort, projection, skip and limit are NOT applied. The applied filter (`AppState.page_query.filter`), never the text being typed, is the source.
- `scan_sorted` matches under binary collation: the export is disabled with a reason when the applied query has `options.collation`.
- An unusable `_id` (`CompareKeyMissing` / `CompareKeyInvalid`) aborts the export and removes the file; never skip silently.
- Destination exists: overwritten (the save dialog asks). Empty source: a valid empty file (`[]\n` for a JSON array, zero bytes for NDJSON) and a result saying 0 documents.
- Not a point-in-time snapshot: the modal says so in one line; progress never exceeds 100%.
- Owner-only file, partial file removed on failure or cancel. Document values, filters and paths are never logged, put in an error, or printed by `Debug` (use redacting `Debug` impls).
- Every visible string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical, lowercase snake_case); report the new keys. Add the prefix `export.` to `PREFIXES` in `crates/ui/src/i18n_usage_tests.rs` (array length 12 to 13) so unused keys are caught.
- Rust 1.87, no let-chains; immutable state (return new values); files under 800 lines; TDD; commits `feat(export): ...` lowercase imperative, ending with the `Co-Authored-By` and `Claude-Session` trailers from the session's attribution instructions.
- Branch: `feat/export-documents-ndjson`. The profiler `query_shapes_*` tests are flaky (#203): rerun a failure there once before investigating.

## Review Focus

- A string with `\n`, `\r`, U+2028 or `"` in a document stays on ONE NDJSON line and parses back equal (Task 2).
- A JSON array is valid JSON after exactly 0, 1 and many documents (Task 2).
- Cancel mid-export removes the file and frees the slot so the next export starts (Task 4).
- A pre-existing file at the chosen path survives when creating the export fails before truncation (Task 1).
- The request, the UI state and the events never show the path or filter in `Debug` output (Tasks 3 and 5).
- A session closed or reconnected mid-export ends with `Failed` and no file (Task 4).

## File structure

| File | Responsibility |
| --- | --- |
| `crates/core/src/runtime/export_file/guard.rs` (modify) | Do not remove a pre-existing file when `create` fails before truncation |
| `crates/core/src/export/mod.rs` (new) | Module doc, re-exports |
| `crates/core/src/export/protocol.rs` (new) | `DocFormat`, `JsonMode`, `DocExportRequest`, `ExportCommand`, `ExportEvent`, `DocExportOutcome` |
| `crates/core/src/export/render.rs` (new) | `doc_line`, `DocWriter` (NDJSON and JSON array) |
| `crates/core/src/export/read.rs` (new) | `DocReader`: `_id`-ordered batches over `scan_sorted` |
| `crates/core/src/runtime/export/{mod.rs,run.rs,tests.rs}` (new) | Slot, dispatch, the job, runtime tests |
| `crates/core/src/runtime/{mod.rs,dispatch.rs,command.rs}` (modify) | `Command::Export`, `Event::Export`, routing, `is_write`/`context`/`request`/`session` arms |
| `crates/core/src/error.rs`, `crates/ui/locales/en.json` | `failures.export_documents`; no new `CoreError` variant |
| `crates/ui/src/viewmodel/doc_export.rs` (+ `_tests.rs`) | State machine and gate |
| `crates/ui/src/app/doc_export.rs` (+ tests), `reducer.rs`, `reducer_dispatch.rs`, `mod.rs`, `file_picker.rs` | Actions, event handling, file dialog |
| `crates/ui/src/views/doc_export.rs`, `views/sidebar.rs`, `views/documents.rs` | Modal and the two entry points |
| `crates/ui/tests/accessibility_doc_export.rs` | Role, label, keyboard checks |
| `crates/core/tests/local_export_documents.rs`, `docs/export.md`, `README.md` | Real-server round trip and docs |

---

### Task 1: Keep a pre-existing file when `FileGuard::create` fails before truncation

**Files:** Modify `runtime/export_file/guard.rs`, `runtime/export_file/guard_tests.rs`.

**Interfaces:** Consumes `FileGuard::create(path: &Path) -> Result<(File, Self)>` (unchanged signature). Produces the rule: on a failure inside `prepare`, the file is removed only if this call created it; a pre-existing file is left as it was and the original error is returned.

- [ ] **Step 1: Write the failing test** `a_failure_right_after_the_open_keeps_a_pre_existing_file` in `guard_tests.rs`: write `"KEEP"` to `dir/f`, call `test_hooks::fail_after_open(&path, false)`, assert `FileGuard::create(&path).unwrap_err() == CoreError::ExportWriteFailed`, `path.exists()`, and its content is still `KEEP`. The existing `a_failure_right_after_the_open_removes_the_file` (new file) must keep passing.
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-core --lib -- runtime::export_file::guard_tests` → the new test fails (file removed).
- [ ] **Step 3: Implement.** In `create`, record `let existed = fs::symlink_metadata(path).is_ok();` before `open_checked`; on a `prepare` failure, remove only when `!existed` (keep the `ExportFileNotRemoved` mapping for that case); when `existed`, drop the guard disarmed (`armed = false`) and return the original error. `restrict` runs before `set_len(0)` so the content is intact at that point.
- [ ] **Step 4: Run.** Same command → all `guard_tests` pass; `cargo test -p curlybrace-core --lib -- runtime::compare` still passes.
- [ ] **Step 5: Commit.** `fix(export): keep a file that was already there when creating the export fails`.

---

### Task 2: Render documents (`doc_line`, `DocWriter`)

**Files:** Create `crates/core/src/export/{mod.rs,render.rs,render_tests.rs}` and the protocol enums needed here (`DocFormat`, `JsonMode` in `export/protocol.rs`); modify `crates/core/src/lib.rs` (`pub mod export;`).

**Interfaces:**
- Produces: `pub enum DocFormat { Ndjson, JsonArray }`, `pub enum JsonMode { Canonical, Relaxed }` (both `Clone, Copy, Debug, PartialEq, Eq`; `Default` for `JsonMode` is `Canonical`, for `DocFormat` is `Ndjson`).
- `pub fn doc_line(doc: &Document, mode: JsonMode) -> String`: compact, single line, never contains a raw newline (`serde_json::to_string` of `into_canonical_extjson()` / `into_relaxed_extjson()`).
- `pub struct DocWriter<W: std::io::Write>` with `new(out: W, format: DocFormat, mode: JsonMode) -> Self`, `begin(&mut self) -> io::Result<()>`, `doc(&mut self, doc: &Document) -> io::Result<()>`, `finish(self) -> io::Result<W>`. NDJSON: `begin` writes nothing, `doc` writes `line + "\n"`. JSON array: `begin` writes `[`; `doc` writes `"\n"` before the first and `",\n"` before later ones, then the line; `finish` writes `"\n]\n"` after at least one document and `"]\n"` after none.

- [ ] **Step 1: Write the failing tests** in `render_tests.rs`:
  - `canonical_round_trip_preserves_every_bson_type`: `exotic_doc()` through `doc_line(Canonical)` then `editor_json_to_doc` equals the original.
  - `relaxed_is_not_type_exact_but_is_valid_json`: an `Int64` and an `Int32` of the same value render identically in relaxed and differently in canonical.
  - `a_string_with_line_breaks_stays_on_one_line`: value `"a\nb\r\u{2028}c\"d"` → `doc_line` has no `\n`/`\r`, and parses back equal.
  - `ndjson_writes_one_line_per_document_and_nothing_for_none` (0 docs → empty output).
  - `json_array_is_valid_json_after_zero_one_and_many_documents`: outputs `[]\n`, and for 1 and 3 documents `serde_json::from_slice::<Vec<Value>>` succeeds with the right length; each document parses back equal.
  - `default_mode_is_canonical`.
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-core --lib -- export::render` → compile error (module missing).
- [ ] **Step 3: Implement** the enums, `doc_line` and `DocWriter` as specified above. Write errors stay `io::Error` (the job maps them).
- [ ] **Step 4: Run.** Same command → pass.
- [ ] **Step 5: Commit.** `feat(export): render documents as ndjson lines and json arrays`.

---

### Task 3: Request/event protocol and the `_id` reader

**Files:** Modify `export/protocol.rs`, `export/mod.rs`; create `export/read.rs`, `export/read_tests.rs`.

**Interfaces:**
- Produces in `protocol.rs`:
  - `pub struct DocExportRequest { pub session: SessionId, pub ns: Namespace, pub filter: Document, pub format: DocFormat, pub mode: JsonMode, pub path: PathBuf }` with a manual `Debug` printing only `DocExportRequest { format, mode, .. }` (no path, filter or namespace).
  - `pub enum ExportCommand { Start { run: RunId, request: DocExportRequest }, Cancel { run: RunId } }`.
  - `pub enum ExportEvent { Progress { run: RunId, done: u64, total: Option<u64> }, Done { run: RunId, outcome: DocExportOutcome }, Failed { run: RunId, error: CoreError } }`.
  - `#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub struct DocExportOutcome { pub written: u64, pub bytes: u64, pub cancelled: bool, pub file_left: bool }`.
  - `SessionId` and `RunId` are `crate::compare::protocol::{SessionId, RunId}`.
- `read.rs`: `pub struct DocReader<S>` holding the store, namespace, filter, batch size and the resume key; `new(store: S, ns: Namespace, filter: Document, batch: usize) -> Self` and `pub async fn next_batch(&mut self) -> Result<Option<Vec<Document>>>` (`None` when finished). Store type: hold it the way `copy/engine.rs` holds `job.source`. Logic copied from `copy/engine.rs::prepare` and `drop_resumed`: `KeySpec::id()`, ask for `batch + 1` when resuming, drop the first document when its key equals the resume key, next resume key from `extract_key(last)`, finished when fewer than requested come back. Default batch constant `EXPORT_BATCH: usize = 500`.

- [ ] **Step 1: Write the failing tests** in `read_tests.rs` over `InMemoryStore`:
  - `reads_every_document_once_in_id_order_across_batches` (seeded with `mock_numbered_docs`, batch 3, ids deliberately inserted out of order) → concatenated batches equal the sorted input, no duplicates.
  - `the_filter_selects_the_documents`.
  - `an_empty_or_missing_collection_yields_no_batches`.
  - `an_unusable_id_is_a_typed_error_and_ends_the_read` (a document with an array `_id` → `CompareKeyInvalid`).
  - In `protocol_tests` (same file): `debug_of_a_request_never_shows_the_path_or_the_filter`.
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-core --lib -- export::` → compile error.
- [ ] **Step 3: Implement** the types and `DocReader` as specified.
- [ ] **Step 4: Run.** Same command → pass.
- [ ] **Step 5: Commit.** `feat(export): protocol and an id-ordered document reader`.

---

### Task 4: The runtime job

**Files:** Create `crates/core/src/runtime/export/{mod.rs,run.rs,tests.rs}`; modify `runtime/mod.rs` (`Command::Export(ExportCommand)`, `Event::Export(ExportEvent)`, `Shared.export: export::ExportSlot`), `runtime/dispatch.rs` (route `Command::Export` before session handling, as `Copy`), `runtime/command.rs` (`is_write` false; `context` `"export_documents"`; `request` none; `Event::session`/`request` arms), `crates/ui/locales/en.json` (`failures.export_documents`: "Could not export the documents").

**Interfaces:**
- Consumes: Task 1 to 3, `export_file::{validate_path, FileGuard, ExportSink, buffered, into_file}`, `Side::open`-style store lookup used by `runtime/copy` (`shared.session(id)`, `generation()`, `current_store()`, `is_current(generation)`), `CancelFlag`, `copy::CopySlot` as the model for `ExportSlot` (one active run; `take(run, cancel) -> Result<SlotGuard>`; released before the terminal event; `session_changing` cancels a run of that session).
- Produces: `pub(super) fn dispatch(shared: &Shared, events: Events, repaint: Repaint, command: ExportCommand)`.
- Behavior: validate synchronously (session open and connected, `validate_path`, slot free); otherwise `Failed` (no slot kept). In a task: `FileGuard::create` in `spawn_blocking`; read batches (`DocReader`), check cancel and `is_current` before each batch, render each document with `DocWriter` over `buffered(ExportSink)`, emit `Progress` after each batch (`done` capped at `total` when known); `total` is `estimated_count` for an empty filter, else `count_matching(ns, &filter, config.command_timeout)` ignoring errors, `None` if unavailable. On success: `into_file`, `sync_all` (in `spawn_blocking`), `guard.commit()`, `Done`. Cancel, error, panic or session change: guard removes the file; cancel ends with `Done { cancelled: true, file_left: false }`; errors with `Failed` and `ExportFileNotRemoved` when removal failed. A write `io::Error` maps to `CoreError::ExportWriteFailed`.

- [ ] **Step 1: Write the failing tests** in `runtime/export/tests.rs` (fixtures: `bulk_support::{FakeStore, connected}` or the compare `rig` pattern; `InMemoryStore` seeded; temp dir):
  - `exports_ndjson_to_the_chosen_file_with_progress_and_done`: N docs → file has N lines each parsing back equal (canonical), `Done.written == N`, file mode is `0o600`.
  - `exports_a_json_array_that_parses_as_json`.
  - `the_filter_limits_what_is_written`.
  - `an_empty_collection_writes_a_valid_empty_file_and_reports_zero`: NDJSON zero bytes, array `[]\n`.
  - `cancel_removes_the_file_and_frees_the_slot_for_the_next_export` (park the read with the `Gate` pattern from `compare/export_lifecycle_tests.rs`, cancel, assert no file, then a second export succeeds).
  - `a_write_failure_removes_the_file_and_fails_with_export_write_failed` (`test_hooks::fail_writes_after(&path, 0)`).
  - `an_unusable_id_aborts_removes_the_file_and_fails_with_a_typed_error`.
  - `a_closed_session_ends_with_failed_and_no_file` and `a_second_export_while_one_runs_is_refused`.
  - `an_existing_file_is_overwritten` and `an_invalid_path_is_refused_before_anything_is_created`.
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-core --lib -- runtime::export` → compile error.
- [ ] **Step 3: Implement** slot, dispatch, job and the `Command`/`Event` plumbing as specified; add the `failures.export_documents` key (the test `english_covers_every_core_error_and_failure_context` must pass).
- [ ] **Step 4: Run.** `cargo test -p curlybrace-core --lib -- runtime::export runtime::export_file export::` and `cargo test -p curlybrace-ui --lib -- i18n` pass.
- [ ] **Step 5: Commit.** `feat(export): a runtime job that streams documents to a file`.

---

### Task 5: UI state machine, gate and i18n keys

**Files:** Create `crates/ui/src/viewmodel/doc_export.rs`, `doc_export_tests.rs`; modify `viewmodel/mod.rs`, `locales/en.json`, `i18n_usage_tests.rs` (`PREFIXES` to 13 entries with `"export."`).

**Interfaces:**
- Produces (modelled on `viewmodel/compare/export.rs` and `export_gate.rs`):
  - `pub struct DocExportDraft { pub format: DocFormat, pub mode: JsonMode }` (`Default` = NDJSON + canonical).
  - `pub struct DocExportSource { pub session: SessionId, pub ns: Namespace, pub filter: Option<Document> }` (`None` = whole collection) with a redacting `Debug`.
  - `pub enum DocExportUi { Idle, Configuring { source, draft }, Choosing { source, draft }, Running { run: RunId, source, draft, path: RedactedPath, total: Option<u64>, done: u64, cancel_requested: bool }, Finished { outcome: Result<DocExportOutcome, CoreError>, path: Option<RedactedPath> } }` (`Default` = `Idle`) with pure transitions `open`, `edited`, `choosing`, `started`, `cancelling`, `progressed`, `finished`, `closed` (a stale `run` changes nothing; `closed` does nothing while busy), plus `is_busy()`.
  - `pub enum DocExportBlock { NotConnected, Collated, Busy }` with `i18n_key(self) -> &'static str` and `pub fn doc_export_block(connected: bool, query: &Query, busy: bool) -> Option<DocExportBlock>` (collation = `query.options.collation.is_some()`).
- Keys (all under `export.`): `title`, `source.collection` (`{name}`), `source.query` (`{name}`), `format.title`, `format.ndjson`, `format.json_array`, `mode.title`, `mode.canonical`, `mode.canonical_hint`, `mode.relaxed`, `mode.relaxed_hint` ("Readable, not type-exact"), `note.sensitive`, `note.consistency`, `choose_file`, `reason.not_connected`, `reason.collated`, `reason.busy`, `running.title`, `running.bar_name`, `running.progress` (`{done}`, `{total}`), `running.progress_unknown` (`{done}`), `running.cancel`, `running.cancelling`, `finished.done` (`{count}`), `finished.cancelled`, `finished.failed`, `finished.file_left`, `finished.close`, `entry.menu`, `entry.toolbar`. Use full string literals at the use sites (the scanner cannot see `format!` keys).

- [ ] **Step 1: Write the failing tests** in `doc_export_tests.rs`: default draft is NDJSON + canonical; `open` then `choosing` then `started` then `progressed` then `finished` happy path; a `progressed` or `finished` for another run changes nothing; `closed` is ignored while `Choosing` or `Running`; `done` never reported above a known `total`; `Debug` of every state hides path, filter and namespace; `doc_export_block` returns `Collated` when the query has a collation, `Busy` when busy, `NotConnected` first, `None` otherwise; every `DocExportBlock::i18n_key` exists in `en.json` (use `I18n::english()`).
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-ui --lib -- doc_export` → compile error.
- [ ] **Step 3: Implement** the state machine, gate and the English keys (alphabetical).
- [ ] **Step 4: Run.** `cargo test -p curlybrace-ui --lib -- doc_export i18n` → pass; unused or missing keys fail `i18n_usage_tests`, so keys must be used by Task 6 and 7 code: add them to the file as those tasks use them, and until then mark the unused ones with an `EXTRA_KEYS` entry that Task 7 removes.
- [ ] **Step 5: Commit.** `feat(export): the document export state machine and its strings`.

---

### Task 6: App wiring and the file dialog

**Files:** Create `crates/ui/src/app/doc_export.rs`, `doc_export_tests.rs`; modify `app/reducer.rs` (`Input::DocExport(DocExportAction)`), `app/reducer_dispatch.rs`, `app/mod.rs` (state field, `ensure_picker`/`poll_picker` branch for this modal), `app/file_picker.rs` (`suggested_doc_file(format: DocFormat) -> (&'static str, &'static str)`: `("export", "ndjson")` / `("export", "json")`), event routing for `Event::Export` taken ahead of the main match (see the stack-limit comment in `reducer.rs`).

**Interfaces:**
- Produces `pub enum DocExportAction { Open { ns: Namespace, whole_collection: bool }, Edit(DocExportDraft), ChooseFile, PathChosen(PathBuf), PathCancelled, Cancel, Close }`. `Open` reads `state.page_query.filter` when `whole_collection` is false and refuses when `doc_export_block` is `Some` (shows the reason, opens nothing). Starting sends `Command::Export(ExportCommand::Start { run, request })` with a fresh `RunId`; `Cancel` sends one `ExportCommand::Cancel`. `Event::Export` updates `DocExportUi` (stale runs ignored); `Failed` reopens the modal with the translated error banner under context `"export_documents"`.

- [ ] **Step 1: Write the failing tests** in `app/doc_export_tests.rs` mirroring `app/compare_tests/export*.rs` (use `FakePicker`-style `FilePicker` double): open from a collection with the applied filter; open refused with a collated query (`reason.collated`); choose file asks the picker for the `.ndjson` suffix (and `.json` for arrays); a chosen path sends exactly one `Start` whose request carries format, mode, filter and path; progress and done events update the state; cancel sends one `Cancel`; a cancelled dialog returns to `Configuring`; `Failed` shows the error; a second `Open` while running is ignored.
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-ui --lib -- app::doc_export` → compile error.
- [ ] **Step 3: Implement** by mirroring `app/compare_export.rs` and the `PendingPick` handling in `app/mod.rs` (add a separate pending-pick field for this modal; do not widen the compare one).
- [ ] **Step 4: Run.** `cargo test -p curlybrace-ui --lib` → pass.
- [ ] **Step 5: Commit.** `feat(export): wire the document export into the app`.

---

### Task 7: Views, entry points and accessibility

**Files:** Create `crates/ui/src/views/doc_export.rs`, `crates/ui/tests/accessibility_doc_export.rs`; modify `views/mod.rs`, `views/sidebar.rs` (`collection_menu`: an "Export documents..." entry after the Copy/Paste entries, disabled with `on_disabled_hover_text` when `doc_export_block` is `Some`, emits `Input::DocExport(Open { ns, whole_collection: true })`), `views/documents.rs` (`show_toolbar`: an "Export" button next to the selection buttons, emits `Open { ns, whole_collection: false }`, same disabled reason line).

**Interfaces:** Consumes `DocExportUi`, `DocExportAction`, `doc_export_block`, `I18n`. The modal shows: source line (`export.source.collection` or `.query`), format radios, mode radios each with its hint, the sensitive-data note, the consistency note, a "Choose file..." button; when running, a progress bar named `export.running.bar_name` plus Cancel; when finished, the result line and Close.

- [ ] **Step 1: Write the failing tests** in `tests/accessibility_doc_export.rs` using egui_kittest and the structure of `tests/accessibility_export.rs` / `tests/support/export_flow.rs`: controls found by role and translated label (`Role::RadioButton` for format and mode, `Role::Button` for Choose file and Cancel); the running state has a `Role::ProgressIndicator` named `running.bar_name`; the relaxed option shows its hint text; Enter/Space operate the buttons; a disabled entry shows its reason as text; opening, choosing a path through `FakePicker` and reaching the finished state works end to end with a scripted event stream.
- [ ] **Step 2: Run to see it fail.** `cargo test -p curlybrace-ui --test accessibility_doc_export` → fails (no view).
- [ ] **Step 3: Implement** the modal view and the two entry points; remove the temporary `EXTRA_KEYS` entries from Task 5 now that every key is used.
- [ ] **Step 4: Run.** `cargo test -p curlybrace-ui` (lib, accessibility, `layout_audit_800`, `wrapped_toolbars`) passes; if a snapshot test changes only because of the new button, update it per the repo's snapshot instructions (`crates/ui/tests/snapshots.rs`) and say so in the commit.
- [ ] **Step 5: Commit.** `feat(export): export documents from the sidebar and the documents toolbar`.

---

### Task 8: Real-server round trip, docs and CI parity

**Files:** Create `crates/core/tests/local_export_documents.rs`, `docs/export.md`; modify `README.md` (feature list, link), `CHANGELOG.md` is generated: do not edit.

- [ ] **Step 1: Write the failing test** `local_export_documents.rs` (pattern of `local_compare_export_files.rs`, `CURLYBRACE_IT` gating): insert the `exotic_doc()` family into a real collection, run the export job to NDJSON canonical, parse every line with `editor_json_to_doc`, and assert documents equal what `find` returns. Skipped (ignored) without a server, like its siblings.
- [ ] **Step 2: Run to see it fail or skip.** `CURLYBRACE_IT=1 cargo test -p curlybrace-core --test local_export_documents` (needs the local mongod used by sibling tests; if none is available, state that and rely on Task 4's in-memory equivalent).
- [ ] **Step 3: Write `docs/export.md`** (what is exported, formats and modes with the type-exactness caveat, ascending `_id` order, binary-collation limit, not a snapshot, overwrite, owner-only file, cancel removes the file) and one README line.
- [ ] **Step 4: Full CI parity.** `cargo +1.87.0 fmt --all --check`; `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`; `cargo +1.87.0 test --workspace --locked` (rerun a profiler `query_shapes_*` failure once, #203); `cargo build --release --locked -p curlybrace` has no warnings.
- [ ] **Step 5: Commit, push with `-u`, open the PR** against `main` ("Part of #54"), body from the template: what and why, how tested, checklist; end with the attribution line from the session's instructions.

---

## Self-review

- Spec coverage: PR 1 scope (collection or query, NDJSON/JSON array, canonical/relaxed, `scan_sorted` reader, progress estimate, cancel, modal, both entry points) is Tasks 2 to 7; decisions 1 (applied filter, collation disabled: Tasks 5, 6), 2 (abort on bad `_id`: Tasks 3, 4), 5 (overwrite, empty file: Tasks 2, 4), 6 (consistency note, progress clamp: Tasks 4, 5, 7); the review's `FileGuard` follow-up is Task 1; round trip is Tasks 2 and 8; i18n and accessibility are Tasks 5 and 7; docs are Task 8. Decision 3 (CSV numbers) and 4 (collection kinds beyond "the browse pane can open it") belong to PR 2 and to Task 4's builder check of views and time-series (a view must either export or be disabled with a reason: the builder verifies and adds a test).
- Type consistency: `DocFormat`, `JsonMode`, `DocExportRequest`, `ExportCommand`, `ExportEvent`, `DocExportOutcome` are defined once (Tasks 2 and 3) and used with those names in Tasks 4 to 7; `DocExportBlock`/`doc_export_block` once (Task 5).
- Proportion: signatures, behaviors and test names only; bodies are the implementer's.
