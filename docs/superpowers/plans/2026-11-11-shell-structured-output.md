# Shell tab table / tree output (slice D) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a Shell tab script's last value is documents, let the person switch the output pane between the usual Text view and a Table view or a Tree view of those documents.

**Architecture:** After the user's script block has been fully evaluated without error, the runner sends one more line (the *epilogue*) that makes `mongosh` print the last value (`_`) as one canonical-EJSON line behind a per-run secret marker. The classifier lifts that line out of the text stream, the scrubber masks it, core parses it into a `ShellResult`, the runtime sends it as `Event::ShellResult`, and the UI shows it with the existing document table/tree code. The text output is untouched. The mechanism and every measurement are in the spike note.

**Tech Stack:** Rust, `bson` (extended JSON parsing), `serde_json`, egui / `egui_extras`, the existing shell runner (`crates/core/src/shell`), runtime (`runtime/shell_handlers.rs`) and shell view-model/views.

**Spec:** `docs/superpowers/specs/2026-11-10-shell-tab-design.md` (slice D: "table/tree output"; it said `mongosh --json`, which cannot be combined with stdin — the mechanism below replaces it). **Mechanism and evidence:** `docs/superpowers/notes/shell-structured-output-spike.md` (mongosh 2.14.1) — read it before Task 2; it has the exact emitter and epilogue lines, payload shapes and caps.

## Global Constraints

- Text output is unchanged and remains the default view. The side-channel line never appears in the text pane, in logs, in `Debug` output, or in history.
- Caps (from the spike): at most 200 elements and 4 MiB of EJSON per result (`too_big` above that); the app also refuses to buffer a payload line over 8 MiB. The payload does NOT count against the text output cap (`RunLimits::max_output_bytes`), but is counted against its own cap.
- The marker token is 16 hex characters from the OS RNG (`uuid::Uuid::new_v4().simple()`, first 16 chars), generated per run, sent ONLY in the epilogue and never in the preamble, argv, env or logs.
- The epilogue is sent only after `Piece::Evaluated` of a complete block with no `Uncaught`, no `Incomplete`, no early end, and not when the block's output contains `Expression assignment to _ now disabled.`; in every other case behave exactly as today (close stdin).
- The scrubber must mask the password in the payload BEFORE it is parsed or shown, and it must also mask the JSON-escaped and the mongosh-inspect-escaped forms of the password (today only the raw and percent-encoded forms are masked: a password containing `\`, `"` or a control character leaks in both the text pane and the payload).
- Read-only/production guards are unchanged: the epilogue is not a script and issues no server commands.
- Every user-visible string is an i18n key in `crates/ui/locales/en.json` (nested, under `shell.result.*`); report new keys. Immutability; files 200-400 lines, 800 max; TDD; real objects over mocks. CI is Rust 1.87: NO let-chains. Never connect to any MongoDB already on the machine; use a throwaway `mongod` on a spare port (27995-27999) with a temp dbpath and a temporary `HOME` for `mongosh`; stop and delete afterwards. Do not edit `CHANGELOG.md`. Commits: conventional, with the two attribution trailers.

## Review Focus

- A password containing `\`, `"`, a newline or a non-ASCII character must not appear in the text pane, the Table or the Tree (all three tested against a real result).
- A script that prints a fake marker line (even with a guessed token), assigns `_`, assigns `__cbEmit`, redefines `print`/`EJSON`, or overrides `prompt` must not produce a wrong-looking-right table: spoofing is impossible without the token, and the documented degraded cases give "no table" with an explanation.
- A result of 200 documents with 40 columns and a result with one 1 MiB string must not freeze the UI (virtualised rows, truncated cells).
- Documents whose fields differ between rows; empty result; `undefined`; a script that errors after printing; a cancelled run: each must show a clear state, never a stale result from the previous run.
- A new run clears the previous result immediately (Table/Tree never show the old run's documents under the new run's text).
- Run with the Table mode selected and a non-document result: the person sees why there is no table and can reach Text in one click.

## File Structure

- Core: modify `shell/invocation.rs` (emitter in the preamble, epilogue builder, token), `shell/classify.rs` (payload lane), `shell/pipe.rs` + `shell/run.rs` + `shell/control.rs` (epilogue writing, payload callback), `shell/scrub.rs` (escaped forms), `shell/fake.rs`; create `shell/result.rs` (+ `result_tests.rs`); modify `runtime/shell_handlers.rs`, `runtime/mod.rs` (`Event::ShellResult`), `runtime/command.rs` (`Event::session()`); tests in `runtime/shell_result_tests.rs` (new); gated `crates/core/tests/local_shell.rs`.
- UI: create `viewmodel/shell_result.rs` (+ tests); modify `viewmodel/shell.rs`, `app/shell_actions.rs`, `app/reducer.rs`; create `views/shell_result.rs` (mode switch + Table + Tree + notices); modify `views/shell.rs`, `views/shell_layout.rs` if needed, `locales/en.json`, `tests/accessibility_shell.rs`.
- Docs: `README.md`, `SECURITY.md`, `docs/superpowers/notes/testing.md` if it lists gated suites.

---

### Task 1: Scrub the escaped forms of the password

**Files:** modify `crates/core/src/shell/scrub.rs`, `crates/core/src/shell/scrub_tests.rs`, `crates/core/src/runtime/shell_handlers.rs` (`secrets_of`, ~line 252).

**Interfaces:**
- Produces: `pub fn secret_forms(password: &str) -> Vec<String>` in `shell/scrub.rs`: the raw password, its percent-encoded form, its JSON-escaped form (as `serde_json` would write the string contents, without the quotes) and its mongosh-inspect-escaped form (backslash doubled; mongosh quotes strings with `'`, `"` or a backtick as needed — include the forms that differ from the raw one), deduplicated, skipping any form identical to the raw. `secrets_of` uses it.
- Consumes: `Scrubber::new(secrets: Vec<String>)` as today.

- [ ] **Step 1: Failing tests** (`scrub_tests.rs`): `a_password_with_a_backslash_is_masked_in_its_doubled_form`, `a_password_with_a_quote_is_masked_in_its_json_escaped_form`, `a_password_with_a_newline_or_unicode_is_masked_in_json_form` (`\n`, `é`), `forms_equal_to_the_raw_password_are_not_duplicated`, `the_escaped_form_is_masked_across_a_chunk_boundary`, `a_plain_password_still_yields_the_raw_and_percent_encoded_forms`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core shell::scrub` → FAIL.
- [ ] **Step 3:** Implement `secret_forms`; wire `secrets_of`. Check the scrubber's hold-back length covers the longest form.
- [ ] **Step 4:** Run → PASS; `cargo +1.87.0 clippy -p curlybrace-core --all-targets -- -D warnings`.
- [ ] **Step 5: Commit** `fix(shell): mask the escaped forms of the password too`.

### Task 2: The side-channel in the runner

**Files:** modify `shell/invocation.rs`, `classify.rs`, `pipe.rs`, `run.rs`, `control.rs`, `fake.rs` and their tests.

**Interfaces:**
- Consumes: Task 1 scrubber; the spike note's `__cbEmit` definition and epilogue (copy them exactly).
- Produces:
  - `invocation.rs`: the emitter statement added to the preamble line (no token, no secret); `pub fn epilogue(token: &str) -> String` returning `__cbEmit(_, "\u0001CBJ-<token> ", 200, 4194304)`; `Invocation` carries a `result_token: String` (fresh per `build_invocation`, 16 hex chars); the `Debug` of `Invocation` stays redacting and does not print the token.
  - `run.rs`: `spawn_shell(..., on_output, on_result: impl FnMut(String) + Send + 'static, on_done)` — `on_result` receives the raw (scrubbed) payload JSON text once at most; `RunLimits::max_result_bytes = 8 MiB`.
  - `classify.rs`: after `Piece::Evaluated`, a payload window: a line starting at line start with `\u0001CBJ-<token> ` is a `Piece::Result(String)`, the single empty line after it is dropped, all other lines in that window are ordinary text; a payload line over `max_result_bytes` is dropped with a `Piece::ResultTooLarge`.
  - `pipe.rs`: the writer sends the epilogue after the last script line only when the gating rule in Global Constraints holds (needs a signal from the classifier: Evaluated seen, no `Uncaught`, not `Incomplete`, the `_ now disabled` text not seen), then closes stdin as today. The text lane never shows the payload line; the payload goes through the `Scrubber` first.
  - `fake.rs`: the fake `mongosh` can be told to answer the epilogue with a canned payload line (using the token it reads from the epilogue), to answer with nothing, with a spoofed marker, or with a huge line.

- [ ] **Step 1: Failing tests:** classifier — `the_payload_line_is_lifted_out_of_the_text`, `a_marker_with_the_wrong_token_is_ordinary_text`, `a_marker_before_evaluated_is_ordinary_text`, `the_blank_line_after_the_payload_is_dropped`, `late_async_output_in_the_window_stays_text`, `an_oversize_payload_is_dropped_and_reported`; run (fake) — `the_epilogue_is_sent_after_a_clean_block`, `no_epilogue_after_an_uncaught_error`, `no_epilogue_after_an_incomplete_block`, `no_epilogue_when_the_underscore_assignment_text_appeared`, `no_epilogue_when_the_prompt_was_redefined` (Evaluated never seen), `cancel_during_the_epilogue_ends_the_run`, `the_payload_is_scrubbed_before_it_is_delivered` (password with `"` and `\`), `the_payload_does_not_count_against_the_text_cap`, `the_token_is_not_in_the_preamble_argv_or_env` (inspect what the fake saw), `the_text_output_is_identical_with_and_without_the_epilogue`.
- [ ] **Step 2:** Run `cargo test -p curlybrace-core shell::` → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS plus the whole `cargo test -p curlybrace-core`.
- [ ] **Step 5: Commit** `feat(shell): lift the last value of a run out as structured output`.

### Task 3: Parse the payload

**Files:** create `crates/core/src/shell/result.rs`, `result_tests.rs`; modify `shell/mod.rs`.

**Interfaces:**
- Produces:
  - `pub enum ShellResult { Documents { kind: DocKind, docs: Vec<bson::Document>, more: bool, total: Option<u64> }, Object { type_name: Option<String>, doc: bson::Document }, Scalar { value: bson::Bson, undefined: bool }, Values { values: Vec<bson::Bson>, more: bool, total: u64 }, TooBig { bytes: u64 }, Unsupported { type_name: Option<String> } }` where `DocKind` is `Cursor | Array`; `Documents` is used for a cursor and for an array whose every element is a document; an array with any non-document is `Values`.
  - `pub fn parse_result(payload: &str) -> Option<ShellResult>`: parses the spike's shapes (`k` = cursor | array | object | scalar | too_big | unsupported) as canonical EJSON via `bson`; `None` for anything malformed (never panics, never partially trusts); counts clamp to the 200 cap.
  - `Debug` for `ShellResult` prints shapes and counts, never document contents.

- [ ] **Step 1: Failing tests** with the spike's real payload lines as fixtures (copy them): every `k`, every BSON type from the note (`$oid`, `$date`, `$numberLong`, `$numberDecimal`, `$binary` incl. UUID subtype 04, `$timestamp`, `$regularExpression`, nested 7 levels), `an_array_with_a_scalar_is_values_not_documents`, `an_empty_cursor_is_documents_with_no_docs`, `malformed_json_is_none`, `an_unknown_kind_is_none`, `a_huge_n_is_clamped`, `debug_never_prints_document_contents`.
- [ ] **Step 2-4:** RED, implement, GREEN. **Step 5: Commit** `feat(shell): parse the structured result payload`.

### Task 4: Runtime event

**Files:** modify `runtime/shell_handlers.rs`, `runtime/mod.rs`, `runtime/command.rs`; create `runtime/shell_result_tests.rs`.

**Interfaces:**
- Consumes: Tasks 2-3 (`on_result`, `parse_result`).
- Produces: `Event::ShellResult { session: SessionId, run_id: u64, result: ShellResult }`, sent at most once per run, always BEFORE that run's `Event::ShellFinished`; `Event::session()` returns `Some`; the Debug of the event does not print contents. A payload that does not parse sends nothing (the UI then shows "no table" for that run).

- [ ] **Step 1: Failing tests** (fake mongosh, real runtime; no sleeps): `a_run_that_returns_documents_sends_shell_result_before_finished`, `a_run_with_no_epilogue_answer_sends_no_result`, `an_unparseable_payload_sends_no_result`, `the_event_debug_does_not_print_documents`, `a_cancelled_run_sends_no_result`, `a_result_never_carries_the_password` (password with `"`/`\`).
- [ ] **Step 2-4:** RED, implement, GREEN; update exhaustive matches and docs. **Step 5: Commit** `feat(runtime): deliver the structured shell result`.

### Task 5: View-model

**Files:** create `viewmodel/shell_result.rs` (+ tests); modify `viewmodel/shell.rs`, `app/shell_actions.rs`, `app/reducer.rs`.

**Interfaces:**
- Consumes: `ShellResult`.
- Produces: `pub enum OutputMode { Text, Table, Tree }` (default `Text`); `pub struct ResultView { run_id: u64, result: Option<Arc<ShellResult>>, state: ResultState }` with `ResultState = Pending | Ready | NoResult` and, on `ShellState`, `result: ResultView` and `mode: OutputMode`; methods (all immutable, returning new state): `result_arrived(run_id, ShellResult)`, `mode_set(OutputMode)`, plus `ShellState::started` clearing the result (state `Pending`) and `finished` marking `NoResult` when no result arrived; `pub fn availability(&self, mode) -> Availability` with `Available | Unavailable(Reason)`, `Reason = Pending | NoValue | NotDocuments | TooBig(bytes) | Unsupported | RunFailed`; Table is available for `Documents` and `Object` (one row); Tree for `Documents`, `Object` and `Values`; a stale `result_arrived` (older `run_id`) is ignored. Tree expand state: `expanded: Arc<HashSet<(usize, Vec<String>)>>`-style with `tree_toggled(doc_index, path)`, `tree_all_set(bool)`; table: `table_columns` reuse, cap 50 columns with a "more columns" count. `ShellAction::{SetOutputMode(OutputMode), TreeToggle(usize, Vec<String>), TreeExpandAll, TreeCollapseAll}`.
- Reuse (read them first): `viewmodel/table.rs::table_columns`, `viewmodel/cell.rs::cell_text`, `viewmodel/tree.rs::{flatten_doc, TreeRow, row_value_text, type_label_key}`; do not copy their logic.

- [ ] **Step 1: Failing tests:** `a_new_run_clears_the_previous_result`, `a_stale_result_is_ignored`, `table_is_available_for_documents_and_an_object_only`, `tree_is_available_for_values_too`, `every_unavailable_reason_is_reported` (scalar, undefined, too_big, unsupported, run failed, pending), `the_mode_survives_a_new_run`, `a_finished_run_without_a_result_is_no_result`, `columns_are_the_union_in_first_seen_order_capped_at_50`, `tree_toggle_and_expand_all`.
- [ ] **Step 2-4:** RED, implement (app: route `Event::ShellResult` like `ShellOutput`; ignore other sessions; wire the actions), GREEN. **Step 5: Commit** `feat(shell): result state and output modes in the view-model`.

### Task 6: The Table / Tree views

**Files:** create `views/shell_result.rs`; modify `views/shell.rs` (and `views/shell_layout.rs` only if the output pane sizing needs it), `locales/en.json`, `tests/accessibility_shell.rs`.

**Interfaces:**
- Consumes: Task 5 state/actions; `views/table.rs` and `views/tree.rs` (read them: they are coupled to the Documents tab's edit/delete state — extract a small read-only drawing core both can share, or write thin read-only variants that reuse `cell_text`/`flatten_doc`; do not copy the editing code).
- Produces: above the output a segmented control `Text | Table | Tree` (each a labelled, keyboard-reachable button; the current one marked selected; unavailable ones disabled with the reason as hover text and the reason also shown as text in the pane when selected-but-unavailable); Table = sticky header + virtualised rows, 200 rows max, truncated cells with full value on hover, "N more columns" note; Tree = one collapsible tree per document with Expand all / Collapse all; a note line for `more` ("first N documents shown, more in the cursor") and for `total` ("showing 200 of N"). No clipboard/export in this slice. Keys under `shell.result.*`: `mode.{text,table,tree}`, `mode_label`, `unavailable.{pending,no_value,not_documents,too_big,unsupported,run_failed}`, `more_in_cursor`, `showing_of`, `more_columns`, `expand_all`, `collapse_all`, `empty`, `row_count`, plus any cell/summary keys the reused code needs.

- [ ] **Step 1: Failing accessibility tests** (accesskit, like the history ones): `the_mode_switch_has_three_labelled_buttons`, `unavailable_modes_are_disabled_with_their_reason`, `selecting_table_shows_a_column_header_and_rows`, `selecting_tree_shows_expandable_rows`, `table_shows_the_reason_when_the_result_is_not_documents`, `the_more_in_cursor_note_is_announced`, `a_result_of_200_documents_renders_only_visible_rows` (virtualisation), `the_output_pane_keeps_its_minimum_height_with_the_history_panel_open` (extend the existing layout test), `switching_modes_emits_set_output_mode`.
- [ ] **Step 2-4:** RED, implement, GREEN (`cargo test -p curlybrace-ui --test accessibility_shell`, the locale-usage test, and the whole UI crate). **Step 5: Commit** `feat(shell): table and tree views of a run's result`.

### Task 7: Real-mongosh tests, docs, verification

- [ ] **Step 1:** Extend `crates/core/tests/local_shell.rs` (gated like the others, throwaway mongod only): `a_cursor_result_has_the_cursor_shape` (asserts the `cursor` kind, `more`, document count — this is the canary for a mongosh upgrade renaming internals), `find_to_array_is_documents`, `a_scalar_is_a_scalar`, `an_error_sends_no_result`, `a_password_with_quote_and_backslash_never_appears_in_text_or_result`, `a_spoofed_marker_is_plain_text`, `assigning_underscore_sends_no_result`. Add to `scripts/integration-tests.sh` if the suite list needs it.
- [ ] **Step 2:** README: a paragraph on the Table/Tree switch (what shows a table, that it reflects the script's last value, the 200-document / 4 MiB limits, what has no table and why). SECURITY.md: the side channel carries query results through the same masking as the text, the per-run token, and the escaped-password fix; note the mongosh-internals dependency and that a mongosh upgrade can reduce it to "no table". `testing.md` if it lists the gated suites.
- [ ] **Step 3:** Gates on CI's toolchain: `cargo +1.87.0 clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --workspace`; if any snapshot of the Shell tab exists, regenerate (`CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture`) then rerun without the flag.
- [ ] **Step 4: Commit** `docs(shell): describe the table and tree output`.

## My calls (change if wrong)

- The text view stays the default and the chosen mode is remembered per session; I do not auto-switch to Table when documents arrive.
- Table also works for a single object (one row); an array of scalars gets Tree only.
- The Table/Tree show exactly what the text pane showed: for a cursor, the first batch (at most `displayBatchSize`, 20 by default), with a note when more exist; no extra server reads. The "continue the cursor to 200 docs" variant from the spike is not used.
- Documents that a script *prints* (`printjson`, `forEach(printjson)`) have no table: only the last returned value does (an add-on via a `printjson` override is possible later).
- No clipboard/export of the table in this slice.
- Because mongosh keeps no `_` after an error, an error run has no table.
- Dependence on mongosh internals (`_currentIterationResult`) is accepted; it degrades to "no table" and a gated test catches a breaking upgrade.
