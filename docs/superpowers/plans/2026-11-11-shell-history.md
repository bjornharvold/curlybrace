# Shell tab history (slice C) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep a per-connection history of the scripts the Shell tab ran, let the person recall one into the editor, and purge/copy it with the connection like the other per-connection data.

**Architecture:** A new `shell_history` table in the existing `history.sqlite` (schema version 3), written by the core runtime when a run ends (so the UI never handles the write), read by a new `ShellHistory` command, cleared by `ClearShellHistory`, copied on duplicate by `CopyShellHistory`, and deleted by the existing `purge_connection`. The UI keeps the list in `ShellState` and shows it in a collapsible History panel in the Shell tab.

**Tech Stack:** Rust, rusqlite (already used by `crates/core/src/history`), egui, the existing `Command`/`Event` runtime.

**Spec:** `docs/superpowers/specs/2026-11-10-shell-tab-design.md` (sections "History" under "Decisions made by the builder's lead" and slice C). Slice A+B are merged (#141).

## Global Constraints

- Stores the script text that actually ran (the selection, or the whole script), a Unix-seconds timestamp and a status. **Never output.** Last **200** per connection. Keyed by connection id (`ConnectionConfig.id`), not session.
- Purged when the connection is deleted (same rule and same `PurgeHistory` command as usage history: only after the delete reached the disk); a duplicate gets a deep copy of the source's history.
- Every user-visible string is an i18n key in `crates/ui/locales/en.json` under `shell.history.*` (nested); report the new keys.
- Immutability in the view-model; files 200-400 lines, 800 max; TDD; real objects over mocks (real temp SQLite files, as `history/db_tests.rs` does).
- A script the person pasted a password into is stored as written; the UI and SECURITY.md say so. The history file stays `0600` (already enforced in `HistoryDb::open`).
- History is optional: with no history database (`history_path: None`, open failure) nothing is recorded, reads return nothing, and the panel says history is unavailable. No error banner.
- Runs that never executed a script are not recorded: refused runs, `StartFailed`, `ConnectFailed`.
- Do not edit `CHANGELOG.md` (release-please). Commits: conventional, with the two attribution trailers. Never connect to a MongoDB already running on the machine; use a throwaway `mongod` on a spare port if a real server is needed (none is needed for this slice).

## Review Focus

- A script of 5 MB (or one containing NUL / invalid UTF-8-safe text) must not break the table or the UI list: scripts over `MAX_STORED_SCRIPT_BYTES` (32 KiB) are not stored (the run still happens).
- Running the same script twice in a row must not fill the list with duplicates: the newest row replaces an identical *directly preceding* one.
- A purged (deleted) connection must not get a history row back from a run that finishes after the delete (`purged` set in `HistoryDb`).
- A file written by this build (version 3) opened by an older build is left alone (`TooNew`); a version 1/2 file migrates to 3 without losing series or shapes.
- Duplicate a connection that has 200 history rows while a run on the source is finishing: copy is one transaction; the copy never exceeds 200.
- Recalling an entry while a run is in progress is allowed (it only edits the buffer); recalling never runs anything.

## File Structure

- Create `crates/core/src/history/shell_history.rs` (+ `shell_history_tests.rs`): the table SQL, `ShellRunStatus`, `ShellHistoryEntry`, `HistoryDb` methods.
- Modify `crates/core/src/history/db_schema.rs` (v3 migration), `history/mod.rs` (exports), `history/db.rs` (`purge_connection`), `db_schema_v2_tests.rs` or a new `db_schema_v3_tests.rs`.
- Modify `crates/core/src/runtime/{mod.rs,command.rs,shell_handlers.rs,indexes.rs}` and tests `runtime/shell_history_tests.rs` (new): commands, events, record on finish.
- Create `crates/ui/src/viewmodel/shell_history.rs` (+ tests); modify `viewmodel/shell.rs`, `app/shell_actions.rs`, `app/shell_state.rs`, `app/connection_ops.rs`/`app/mod.rs` (duplicate hook).
- Create `crates/ui/src/views/shell_history.rs`; modify `views/shell.rs`, `locales/en.json`; add to `tests/accessibility_shell.rs`.
- Modify `README.md`, `SECURITY.md`, `docs/superpowers/notes/testing.md` if it lists the shell tests.

---

### Task 1: Schema v3 and the store

**Files:** Create `history/shell_history.rs`, `history/shell_history_tests.rs`; modify `history/db_schema.rs`, `history/mod.rs`, `history/db.rs`.

**Interfaces:**
- Produces:
  - `pub const SHELL_HISTORY_LIMIT: usize = 200;` and `pub const MAX_STORED_SCRIPT_BYTES: usize = 32 * 1024;`
  - `pub enum ShellRunStatus { Succeeded, ScriptErrors, Incomplete, Cancelled, CapReached, Failed }` (`Clone, Copy, Debug, PartialEq, Eq`), with `fn as_str(self) -> &'static str` / `fn parse(&str) -> Option<Self>` and `pub fn from_outcome(&Outcome) -> Option<Self>` (`None` for `ConnectFailed`, `StartFailed`; `Exited(0)` → `Succeeded`; other exit codes and `Signalled` → `Failed`).
  - `pub struct ShellHistoryEntry { pub id: i64, pub ran_at_secs: i64, pub script: String, pub status: ShellRunStatus }`
  - On `HistoryDb`: `record_shell_run(&self, connection: Uuid, script: &str, status: ShellRunStatus, now_secs: i64) -> Result<()>`, `shell_history(&self, connection: Uuid) -> Result<Vec<ShellHistoryEntry>>` (newest first, at most 200), `clear_shell_history(&self, connection: Uuid) -> Result<()>`, `copy_shell_history(&self, from: Uuid, to: Uuid) -> Result<()>`.
  - `SCHEMA_VERSION = 3`; `purge_connection` also deletes the connection's `shell_history` rows.

- [ ] **Step 1: Write failing tests** in `shell_history_tests.rs` (real temp file via the same helper `db_tests.rs` uses): `a_run_is_recorded_and_read_back_newest_first`, `only_the_last_200_are_kept`, `an_identical_directly_preceding_script_is_replaced_not_added` (same script, new timestamp and status; a different script in between keeps both), `a_script_over_the_size_cap_is_not_stored`, `history_is_per_connection`, `clear_removes_only_that_connections_rows`, `copy_makes_an_independent_deep_copy_capped_at_200`, `purge_connection_removes_shell_history_and_blocks_later_records` (record after `purge_connection` is ignored until `allow_recording`), `from_outcome_maps_every_variant` (all nine `Outcome` variants), plus in a v3 schema test: `a_version_2_file_migrates_to_3_keeping_series_and_shapes`, `a_version_4_file_is_too_new`, `opening_twice_migrates_once`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core history::` → FAIL (items undefined).
- [ ] **Step 3: Implement** the table `shell_history(id INTEGER PRIMARY KEY AUTOINCREMENT, connection TEXT NOT NULL, ran_at_secs INTEGER NOT NULL, script TEXT NOT NULL, status TEXT NOT NULL)` with `INDEX shell_history_by_connection (connection, id)`, `migrate_to_v3` modelled on `migrate_to_v2` (immediate transaction, re-read `user_version` inside). Insert, dedupe-the-preceding-row and trim-to-200 in one transaction under the connection lock; honour `is_purged`. Keep `shell_history.rs` self-contained like `shapes.rs`.
- [ ] **Step 4: Run** `cargo test -p curlybrace-core history::` → PASS; `cargo +1.87.0 clippy -p curlybrace-core --all-targets -- -D warnings` clean (CI uses Rust 1.87: no let-chains).
- [ ] **Step 5: Commit** `feat(history): store shell run history in history.sqlite`.

### Task 2: Runtime commands, events and recording

**Files:** modify `runtime/mod.rs`, `runtime/command.rs`, `runtime/shell_handlers.rs`, `runtime/indexes.rs`; create `runtime/shell_history_tests.rs`.

**Interfaces:**
- Consumes: Task 1's `HistoryDb` methods and `ShellRunStatus::from_outcome`.
- Produces:
  - `Command::ShellHistory { session: SessionId }` → `Event::ShellHistory { session: SessionId, entries: Vec<ShellHistoryEntry> }` (empty when history is off).
  - `Command::ClearShellHistory { session: SessionId }` → re-sent `Event::ShellHistory` (empty list).
  - `Command::CopyShellHistory { from: Uuid, to: Uuid }` (no answer, like `PurgeHistory`; a failure is logged).
  - `context()` keys `"shell_history"`, `"clear_shell_history"`, `"copy_shell_history"`; `is_write()` false for the first, false for the others (they touch only the local file, like `PurgeHistory`); update every exhaustive match in `command.rs`.
  - Recording: in `shell_handlers::start`'s `on_done`, after the slot is released and `ShellFinished` is sent, push a record job (never waiting) on the runtime's one ordered shell-history queue (`runtime/shell_history_queue.rs`), which also runs `ShellHistory`, `ClearShellHistory` and `CopyShellHistory` in send order; it calls `record_shell_run(run.cfg.id, &script, status, now)` when `from_outcome` is `Some` and then sends `Event::ShellHistoryChanged { session, stored }`. The script is `run.script` (what actually ran). A failure is logged (`log::warn!` without the script text) and never changes the outcome.

- [ ] **Step 1: Failing tests** (`shell_history_tests.rs`, fake `mongosh` like `shell_tests.rs`, real temp history file): `shell_finished_is_not_delayed_by_the_record` (a held file lock delays only `ShellHistoryChanged`), `a_read_sent_after_shell_history_changed_sees_the_row`, `a_refused_run_is_not_recorded`, `a_cancelled_run_is_recorded_as_cancelled`, `history_off_records_nothing_and_reads_empty`, `shell_history_command_answers_with_the_connections_entries`, `clear_shell_history_answers_with_an_empty_list`, `copy_shell_history_copies_between_connection_ids`, `a_run_finishing_after_its_connection_was_purged_is_not_recorded`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core runtime::shell_history` → FAIL.
- [ ] **Step 3: Implement** with the queue's worker running each job through `indexes::blocking` (one at a time, a panic caught per job).
- [ ] **Step 4: Run** the new tests, then `cargo test -p curlybrace-core runtime::` → PASS. No sleeps in tests: wait on events.
- [ ] **Step 5: Commit** `feat(runtime): record and serve shell run history`.

### Task 3: View-model

**Files:** create `viewmodel/shell_history.rs` + tests; modify `viewmodel/shell.rs`.

**Interfaces:**
- Consumes: `ShellHistoryEntry`, `ShellRunStatus`.
- Produces:
  - `pub struct HistoryView { pub available: bool, pub open: bool, pub entries: Arc<Vec<ShellHistoryEntry>>, pub confirm_clear: bool }` with `Default`; field `pub history: HistoryView` on `ShellState`.
  - `ShellState::history_loaded(&self, entries: Vec<ShellHistoryEntry>) -> Self`, `history_toggled(&self) -> Self`, `history_unavailable(&self) -> Self`, `history_entry_recalled(&self, id: i64) -> Self` (replaces `script` with the entry's text, clears the selection, leaves output and status untouched; unknown id → unchanged), `clear_requested(&self) -> Self` (sets `confirm_clear`), `clear_cancelled(&self) -> Self`, `clear_confirmed(&self) -> (Self, bool)` (second value: whether to send the command).
  - `pub fn first_line(script: &str, max_chars: usize) -> String` (first non-empty line, `…` appended when cut; char-boundary safe) for the list rows.

- [ ] **Step 1: Failing tests:** `recalling_replaces_the_script_and_clears_the_selection`, `recalling_while_running_only_edits_the_buffer`, `an_unknown_id_changes_nothing`, `clear_needs_a_confirm_step`, `cancelling_the_clear_keeps_the_entries`, `first_line_skips_blank_lines_and_cuts_on_a_char_boundary` (multi-byte text), `loading_replaces_the_list_and_marks_available`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** (clone-and-modify, no mutation). **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(shell): history state in the shell view-model`.

### Task 4: App wiring

**Files:** modify `app/shell_actions.rs`, `app/shell_state.rs`, `app/mod.rs` (event handling), `app/connection_ops.rs` or the duplicate call site; tests in `app/shell_history_tests.rs` (new, small).

**Interfaces:**
- Consumes: Task 2 commands/events, Task 3 state methods.
- Produces: `ShellAction::{ToggleHistory, RecallEntry(i64), ClearHistory, ConfirmClear, CancelClear}` (next to the existing shell actions).
- Behaviour: only while the History panel is open is the history read: opening the panel, opening the Shell tab with the panel open, and every `Event::ShellHistoryChanged` of the session send `Command::ShellHistory` (a closed panel reads nothing and reads fresh when opened); `Event::ShellHistory` for a session updates that session's `ShellState`; a connection duplicate sends `Command::CopyShellHistory { from: source, to: new }` for each new saved id (same shape as `copy_tools_to_new`, run only when the duplicate was saved); delete already purges via `PurgeHistory` (assert it in a test, do not re-implement).

- [ ] **Step 1: Failing tests:** `opening_the_shell_tab_requests_the_history`, `a_finished_run_requests_the_history_again`, `a_history_event_for_another_session_does_not_touch_this_one`, `recall_puts_the_script_in_the_editor_and_runs_nothing` (no `ShellRun` command emitted), `confirmed_clear_sends_the_command_once`, `a_saved_duplicate_copies_the_history_and_an_unsaved_one_does_not`, `a_delete_purges_the_history_through_purge_history`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** `cargo test -p curlybrace-ui app::` → PASS.
- [ ] **Step 5: Commit** `feat(shell): load, recall, clear and copy history in the app`.

### Task 5: The History panel

**Files:** create `views/shell_history.rs`; modify `views/shell.rs`, `locales/en.json`; extend `tests/accessibility_shell.rs`.

**Interfaces:**
- Consumes: Task 3 `HistoryView`, Task 4 actions.
- Produces: a collapsible "History" section below the editor toolbar (above the output pane): header button toggles it; each row shows time (relative, from `ran_at_secs`), a status badge and `first_line`; clicking a row emits `RecallEntry`; a "Clear history" button with the inline confirm/cancel pair. When `available == false` the section shows the unavailable hint instead of a list. Rows have stable ids from the entry id; row activation works from the keyboard.
- i18n keys under `shell.history.*`: `title`, `toggle_open`, `toggle_close`, `empty`, `unavailable`, `recall_hint`, `clear`, `clear_confirm`, `clear_confirm_yes`, `clear_cancel`, `plaintext_note` (says scripts are stored as written, including anything pasted into them), and `status.{succeeded,script_errors,incomplete,cancelled,cap_reached,failed}`. No literal strings in the view.

- [ ] **Step 1: Failing accessibility tests** in `accessibility_shell.rs` (accesskit tree, like the existing ones): `the_history_toggle_is_a_labelled_button`, `each_history_row_is_a_labelled_button_with_its_status`, `the_unavailable_and_empty_states_are_announced`, `clear_history_asks_before_it_clears`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui --test accessibility_shell` → FAIL. **Step 3: Implement** the view and keys. **Step 4: Run** → PASS; also run `cargo test -p curlybrace-ui` for the locale-key coverage test.
- [ ] **Step 5: Commit** `feat(shell): history panel with recall and clear`.

### Task 6: Docs, snapshots, full verification

- [ ] **Step 1:** README: one paragraph on history (what is stored, 200 per connection, recall, clear, off when `history.sqlite` is unavailable). SECURITY.md: scripts are stored in plain text in `history.sqlite` (`0600`) exactly as written, including any secret the person typed into a script; output is never stored; delete the connection or use Clear history to remove it. Update `docs/superpowers/notes/testing.md` if it enumerates shell tests.
- [ ] **Step 2:** Regenerate screenshot baselines only if the panel changes an existing snapshot: `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo test -p curlybrace-ui --test snapshots -- --include-ignored --nocapture`, then rerun without the update flag.
- [ ] **Step 3:** Full gates on CI's toolchain: `cargo +1.87.0 clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo test --workspace` (all pass; the three clippy errors that exist on newer toolchains are not in scope).
- [ ] **Step 4: Commit** `docs(shell): describe the shell history`.

## My calls (change if wrong)

- Records in core when the run ends, not from the UI: the UI never holds a write path to the history file, and a run that outlives its tab is still recorded.
- Identical directly preceding scripts are collapsed into one row (newest timestamp and status).
- A run that was cancelled, hit the output cap or had script errors is recorded, with its status; runs that never executed are not.
- Scripts over 32 KiB are not stored.
- Clear history has a confirm step; recall has none because it only edits the buffer.
- Search, pinning, per-entry delete and export are out of scope for this slice.
