# Saved aggregation pipelines (slice 2a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use checkbox syntax.

**Goal:** Save, open, rename and delete named aggregation pipelines per connection, kept in `pipelines.json`.

**Architecture:** A pure immutable data type in core (`SavedPipelines`), a UI-owned JSON store modelled on `app/tool_settings.rs` (atomic 0600 write, backup of a corrupt file, `PersistGuard`), per-session dialog/draft state in the aggregate view model, reducer actions, and toolbar views.

**Tech Stack:** Rust 1.87 (no let-chains), egui, serde_json, existing `app/persistence.rs::atomic_write`, `app/settings_file.rs`.

**Spec:** `docs/superpowers/specs/2026-11-14-aggregation-slice-2-design.md` (slice 2a section).

## Global Constraints

- Every user-facing string is an i18n key under `aggregate.saved.*` in `crates/ui/locales/en.json` (nested). Report new keys.
- Files under 400 lines (800 max); immutable style (every op returns a new value); real objects over mocks; TDD; coverage of new code at least 80%.
- Name: trimmed, 1..=80 chars, unique per connection case-insensitively. At most 200 pipelines per connection, 1 MB of stage text per pipeline, file refused at load above 5 MB (treated as corrupt).
- File: `pipelines.json` beside `connections.json`; `{ "version": 1, "connections": { "<uuid>": [ { "id", "name", "ns_hint", "saved_unix", "stages": [ { "text", "enabled" } ] } ] } }`; 0600; atomic write.
- Never log pipeline text or names; log counts and error variants only.
- Opening a saved pipeline runs nothing by itself. Slice 1 behaviour (previews never write) is unchanged.
- Run git as plain separate Bash calls; `cargo +1.87.0`; commits conventional (`feat(aggregate): ...`), ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never connect to any MongoDB (none needed in this slice).

## Review Focus

- A corrupt, truncated, oversized or wrong-version `pipelines.json` never loses the good data silently: it is backed up, the app says so, later saves are blocked if the backup failed.
- A save that fails to write keeps the draft and the in-memory list unchanged and says so.
- Two saved pipelines whose names differ only in case or surrounding spaces are refused as duplicates; unicode names work; a 80-character name fits the dialog.
- Opening with unsaved changes asks first; cancel keeps the draft; the "modified" mark is correct after save, open, edit and Stages-Text-Stages.
- Deleting or duplicating a connection removes or copies its pipelines; one connection never sees another's.

### Task 1: core data type `SavedPipelines`

**Files:** Create `crates/core/src/saved_pipeline.rs`, `crates/core/src/saved_pipeline_tests.rs`; modify `crates/core/src/lib.rs` (export).

**Interfaces:**
- Produces: `SavedStage { text: String, enabled: bool }`; `SavedPipeline { id: Uuid, name: String, ns_hint: Option<String>, saved_unix: i64, stages: Vec<SavedStage> }`; `SavedPipelines` (serde, `version: u32`, `connections: BTreeMap<Uuid, Vec<SavedPipeline>>`) with immutable ops returning `Result<SavedPipelines, SavedError>`: `saved(connection, name, ns_hint, stages, now_unix, replace: bool)`, `renamed(connection, id, name)`, `deleted(connection, id)`, `without_connection(connection)`, `with_connection_copied(from, to)`, and reads `list(connection) -> &[SavedPipeline]` (most recently saved first), `find(connection, id)`, `name_taken(connection, name, except: Option<Uuid>)`. `SavedError { EmptyName, NameTooLong, NameTaken, TooMany, TooLarge }`, a stable `error_variant()` for logs and the i18n mapping in the UI. `validate_name(&str) -> Result<String, SavedError>` returns the trimmed name.

- [ ] Failing tests: names (trim, empty, 81 chars, duplicate case-insensitive, replace=true keeps the id and updates content, rename to own name allowed), limits (201st pipeline, over 1 MB), ordering (newest first), without/copy connection, serde round trip, unknown `version` and unknown extra fields tolerated or refused as specified (wrong `version` > 1 is an error `Unsupported` handled by the loader as corrupt).
- [ ] Implement; run `cargo +1.87.0 test -p curlybrace-core saved_pipeline`; commit `feat(aggregate): add the saved pipeline data type`.

### Task 2: UI persistence of `pipelines.json`

**Files:** Create `crates/ui/src/app/pipelines_store.rs` (+ `_tests.rs`), `crates/ui/src/app/pipelines_app.rs` (state wiring); modify `crates/ui/src/app/mod.rs`, `app/state.rs` (fields), the connection delete and duplicate paths (find where `tool_settings` overrides are removed or copied, e.g. `with_connection_copied` / `without_connection` callers), recovery banner wiring (as `tools.json`).

**Interfaces:**
- Consumes: Task 1 types; `atomic_write`, `settings_file::load`, `PersistGuard`, the `Startup<T>` pattern from `tool_settings.rs`.
- Produces: `default_pipelines_path() -> ConfigPath`; `load(path, backup) -> Startup<SavedPipelines>` (refuses files over 5 MB); `saved(path, &guard, &SavedPipelines) -> Result<(), PipelinesSaveError>`; `AppState::with_pipelines_path(ConfigPath)`; `AppState.pipelines: SavedPipelines`, `pipelines_path`, `pipelines_guard`; helper `AppState::persist_pipelines(next: SavedPipelines) -> Result<(), PipelinesSaveError>` that updates `AppState.pipelines` only after the write succeeded.

- [ ] Failing tests with temp dirs: missing file gives empty store; round trip; corrupt JSON is backed up (0600) and an empty store loads with a recovery banner; backup failure blocks saves; oversized file treated as corrupt; file mode is 0600; deleting a connection removes its pipelines and duplicating copies them (through the real app paths); a failed write leaves `AppState.pipelines` unchanged.
- [ ] Implement; commit `feat(aggregate): persist saved pipelines in pipelines.json`.

### Task 3: view model state and actions

**Files:** Modify `crates/ui/src/viewmodel/aggregate.rs` (keep it under 400 lines: put new state in `viewmodel/aggregate_saved.rs` + tests), `viewmodel/aggregate_actions` enum (`AggregateAction`).

**Interfaces:**
- Consumes: Task 1 types; the draft (`AggregateState`: stages, keys, enabled flags, mode).
- Produces: `SavedUi { opened: Option<(Uuid, String)>, baseline: Option<Vec<SavedStage>>, dialog: Option<SavedDialog> }` on `AggregateState` (per session, carried across connection loss like the draft); `SavedDialog { Save{name, replace_prompt}, ConfirmOpen{id}, Rename{id, name}, ConfirmDelete{id} }`; `AggregateAction::{ SaveClicked, SaveNameEdited(String), SaveConfirmed, OpenMenuPicked(Uuid), OpenConfirmed, RenameStarted(Uuid), RenameEdited(String), RenameConfirmed, DeleteAsked(Uuid), DeleteConfirmed, DialogCancelled }`; `AggregateState::is_modified() -> bool` (draft stages vs baseline, ignoring keys, so Stages-Text-Stages round trips of unchanged text are not modified); `AggregateState::as_saved_stages() -> Vec<SavedStage>`; `AggregateState::opened_from(saved: &SavedPipeline) -> AggregateState` (keeps unique stage keys, resets preview to Idle so auto refresh runs, clears queued/in-flight bookkeeping correctly).

- [ ] Failing tests: is_modified after edit/toggle/move/add/remove/open/save; dialogs open and cancel without touching the draft; opening replaces the draft and keys stay unique.
- [ ] Implement; commit `feat(aggregate): add the saved pipeline view model`.

### Task 4: reducer

**Files:** Create `crates/ui/src/app/aggregate_saved_actions.rs` (+ tests); modify `app/aggregate_actions.rs` (dispatch), banner/error mapping.

**Interfaces:** Consumes Tasks 2 and 3. Produces the handling of each new `AggregateAction`: Save (validate name, replace prompt when taken, persist, set opened+baseline, close dialog, toast/status), Open (ask first when `is_modified()`), Rename, Delete (confirm), error mapping `SavedError` and `PipelinesSaveError` to i18n keys shown inline in the dialog (never a banner for a name problem; a write failure keeps the dialog and the draft).

- [ ] Failing reducer-level tests using a real `AppState` with a temp `pipelines.json`: save then open on a fresh state; replace prompt; write failure (read-only temp dir) keeps the dialog open and the list unchanged; open asks when modified; delete of the opened pipeline clears `opened` but keeps the draft; two sessions on two connections never see each other's pipelines.
- [ ] Implement; commit `feat(aggregate): save, open, rename and delete pipelines`.

### Task 5: views and i18n

**Files:** Create `crates/ui/src/views/aggregate_saved.rs` (+ tests); modify `views/aggregate.rs` (toolbar: Save button, Open menu, opened name + "modified" mark), `crates/ui/locales/en.json` (`aggregate.saved.*`).

**Interfaces:** Consumes Tasks 3 and 4. Produces `show_saved_toolbar(ui, &AggregateState, &SavedPipelines, connection, i18n) -> Vec<AggregateAction>` and `show_saved_dialog(ctx, ...)` with the Save dialog (name field with live validation message and 80-char limit), the replace prompt, the open-with-changes prompt, rename, and delete confirmation. All controls named for accessibility; Open menu lists name plus collection hint, newest first; disabled with a reason when the list is empty.

- [ ] Failing tests (egui_kittest, like `views/aggregate_*` tests): buttons named, dialogs keyboard operable (Enter confirms, Escape cancels, focus lands in the name field), empty list state, long names elided with a tooltip.
- [ ] Implement; commit `feat(aggregate): add the save and open controls`.

### Task 6: layout, accessibility, snapshots, docs

**Files:** modify/create tests under `crates/ui/tests/` (accessibility for the toolbar and dialogs at 1000x700 and 800x500, snapshot of the toolbar with an opened pipeline, light and dark); regenerate only the baselines that legitimately change (aggregate tab ones) and report any other; README Aggregate section (saved pipelines now exist; update the "Not yet" bullet, roadmap wording), `site/index.html` feature text is updated in a separate docs PR after release, not here.

- [ ] Run `cargo +1.87.0 fmt --all`, `clippy --workspace --all-targets --locked -- -D warnings`, `test --workspace --locked`; commit `test(aggregate): cover the saved pipeline layout and accessibility` and `docs(readme): describe saved pipelines`.
