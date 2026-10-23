# Production Connection Guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a saved connection carry a preset colour and a separate "production" flag, show both wherever the connection is named, and ask for a prominent confirm-button warning before every write to a production connection.

**Architecture:** Two new `#[serde(default)]` fields on `ConnectionConfig` (`color`, `production`). The UI reducer parks every mutating command for a production connection in `AppState.write_warning` instead of sending it; a modal then confirms (sends it once) or cancels (drops it). Colour is rendered by a pure view-model mapping (rgb + glyph + label key) so colour never stands alone.

**Tech Stack:** Rust workspace (`curlybrace-core`, `curlybrace-ui` on egui/eframe), serde JSON connection file, i18n in `crates/ui/locales/en.json`.

**Spec:** GitHub issue #47, as narrowed in the brainstorm on 2026-10-23 (below, "Global Constraints"). No separate design document.

## Global Constraints

- Warning is a **confirm button** dialog, not a typed connection name (user decision; overrides the ticket's typed confirmation).
- **Every** write on a production connection warns: insert, edit (replace), clone, delete, compare apply.
- `production` is its own checkbox, independent of colour. Colour is optional.
- Colour is one of six presets: red, orange, yellow, green, blue, purple. No free picker.
- Colour is never the only signal: each colour has a glyph and a translated name; production has a text badge.
- New fields are `#[serde(default)]`; old connection files load unchanged; no secrets in them.
- Read-only lock is **out of scope** (file a follow-up ticket). Typed confirmation is out of scope.
- The warning is a UI confirmation; the core does not enforce it.
- Every visible string is an i18n key in `crates/ui/locales/en.json`; report the new keys when done.
- Immutable style: state transitions return new values. Files stay under 800 lines.
- Work on a feature branch and open a PR against `main` (the repo has no integration branch).
- Verification before commit: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.

## Review Focus

- A connection file whose `color` holds a value this build does not know must still load (colour falls back to none), not fail the whole list. Test in Task 1.
- A second write attempted while a warning is open must be ignored, not replace or queue behind the parked one. Test in Task 4.
- A write that would be ignored anyway (another mutation in flight, no collection selected, not connected) must not open a warning. Test in Task 4.
- Confirming a parked write after its session was closed or another session was activated must send nothing. Test in Task 4.
- Duplicating a production connection keeps it production (safe default). Test in Task 1.
- Delete on a production connection shows one dialog, not two (the existing delete confirm carries the production notice). Test in Task 4 and Task 5.

---

### Task 1: Connection model fields

**Files:**
- Modify: `crates/core/src/connection.rs` (struct at line 43, `new` at line 64)
- Test: `crates/core/src/connection.rs` test module (or the module's existing test file; follow where its current tests live)

**Interfaces:**
- Produces: `pub enum ConnectionColor { Red, Orange, Yellow, Green, Blue, Purple }` (derives `Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize`) and `pub const ALL: [ConnectionColor; 6]` as `ConnectionColor::ALL`; `ConnectionConfig { color: Option<ConnectionColor>, production: bool }`.

- [ ] **Step 1: Write failing tests** `old_file_without_new_fields_loads_as_uncoloured_non_production`, `color_and_production_round_trip_through_json`, `unknown_color_value_loads_as_none` (a JSON config with `"color": "chartreuse"` deserialises with `color == None` and every other field intact), `duplicate_keeps_color_and_production`, `new_is_non_production_and_uncoloured`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core connection` and confirm they fail to compile or fail.
- [ ] **Step 3: Implement** the enum and the two fields (`#[serde(default)]`). For `color`, add `#[serde(default, deserialize_with = "lenient_color")]` where `lenient_color` deserialises an `Option<serde_json::Value>`-style lenient read and returns `None` for an unrecognised string. Set both in `new`. Fix every struct-literal construction the compiler flags (the compiler lists them).
- [ ] **Step 4: Run** `cargo test -p curlybrace-core` and confirm pass.
- [ ] **Step 5: Commit** `feat(connections): add colour and production flag to the connection model`

### Task 2: Edit form and dialog

**Files:**
- Modify: `crates/ui/src/viewmodel/connection_form.rs` (`ConnectionForm` line 14, `to_config` line 175, `from_config` line 220)
- Modify: `crates/ui/src/views/connections_edit.rs` (`show_fields`)
- Modify: `crates/ui/locales/en.json`
- Test: `crates/ui/src/viewmodel/connection_form.rs` tests; the existing connection-dialog UI test file for the new widgets

**Interfaces:**
- Consumes: Task 1 fields.
- Produces: `ConnectionForm { color: Option<ConnectionColor>, production: bool }` mapped both ways; keys `connections.color`, `connections.color_none`, `connections.production`, `connections.production_hint`.

- [ ] **Step 1: Write failing tests** `form_round_trips_color_and_production` (config → `from_config` → `to_config` equal), `default_form_is_uncoloured_non_production`, and a dialog test that the colour combo (labelled `connections.color`) and the production checkbox (labelled `connections.production`) exist and toggling them changes the saved config.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui connection_form` and confirm failure.
- [ ] **Step 3: Implement** the two form fields and mappings; in `show_fields` add a colour `ComboBox` (options: none + `ConnectionColor::ALL`, each rendered with its glyph and translated name via Task 3's `color_tag`, so build Task 3's mapping first if the compiler needs it — see ordering note) and a production `checkbox` with the hint label. Name both accessibly, as `auth_combo` does.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui` and confirm pass.
- [ ] **Step 5: Commit** `feat(connections): edit colour and production flag in the connection dialog`

> Ordering: **execute Task 3 before Task 2.** Task 2's colour combo renders options with Task 3's `color_tag`; the tasks are otherwise independent. Tasks 4–6 follow in numeric order.

### Task 3: Colour tag view-model and display

**Files:**
- Create: `crates/ui/src/viewmodel/connection_tag.rs` (register in `viewmodel/mod.rs`)
- Modify: `crates/ui/src/app/session.rs` (`Root`, `root()` at line 327), `crates/ui/src/views/sidebar_roots.rs` (`show_title_line`), `crates/ui/src/views/documents.rs` (`show_connection_header`), `crates/ui/src/app/panels.rs` (call sites at lines 157 and 174)
- Modify: `crates/ui/locales/en.json`
- Test: `connection_tag.rs` tests; `crates/ui/src/app/session_labels_tests.rs` for `Root`

**Interfaces:**
- Produces: `pub struct ColorTag { pub rgb: [u8; 3], pub glyph: &'static str, pub name_key: &'static str }`; `pub fn color_tag(c: ConnectionColor) -> ColorTag`; `pub struct ConnectionBadge { pub color: Option<ColorTag>, pub production: bool }`; `pub fn badge_of(cfg: &ConnectionConfig) -> ConnectionBadge`; `Root { color: Option<ConnectionColor>, production: bool }`; `AppState::active_badge(&self) -> ConnectionBadge`. Keys: `connection.color.red|orange|yellow|green|blue|purple`, `connection.production_badge`.

- [ ] **Step 1: Write failing tests** `every_color_has_a_distinct_glyph_and_name_key` (six unique glyphs, six unique keys), `every_color_name_key_exists_in_en_json`, `badge_of_reflects_config`, `root_carries_color_and_production_of_its_connection`, `embedded_session_has_no_badge`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui connection_tag` and confirm failure.
- [ ] **Step 3: Implement** the mapping (glyphs: ● ▲ ■ ◆ ★ ⬢ in colour order; pick readable rgb values with sufficient contrast on both egui themes) and `Root` fields. In `show_title_line` draw the glyph in the colour before the name plus the `connection.production_badge` text when production; in `show_connection_header` do the same. Give the coloured glyph an accessible label of the colour name.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui` and confirm pass, including the i18n usage test.
- [ ] **Step 5: Commit** `feat(connections): show colour tags and the production badge on tree roots and the connection header`

### Task 4: Write guard in the reducer

**Files:**
- Create: `crates/ui/src/app/write_guard.rs` (register in `app/mod.rs`)
- Modify: `crates/ui/src/app/state.rs` (add field, clear it in `session_cleared`), `crates/ui/src/app/actions.rs` (`mutation` at line 258, `DocAction::Delete` at line 194), `crates/ui/src/app/reducer.rs` (`Input` at line 20, `update` at line 41)
- Test: `crates/ui/src/app/write_guard_tests.rs`

**Interfaces:**
- Consumes: `AppState::connection`, `AppState::active`, `Pending`.
- Produces: `pub struct ParkedWrite { pub command: Command, pub pending: Pending, pub kind: WriteKind }`; `pub enum WriteKind { Insert, Replace, Clone, Delete }`; `AppState.write_warning: Option<ParkedWrite>`; `pub enum WriteWarningAction { Confirm, Cancel }`; `Input::WriteWarning(WriteWarningAction)`; `AppState::active_is_production(&self) -> bool`; `mutation(state, editor, kind, guard, build)` where `guard` is `Guard::Warn` or `Guard::Confirmed`.

- [ ] **Step 1: Write failing tests**, using real `ConnectionConfig`s from the existing mock objects with `production = true`: `insert_replace_clone_on_production_park_instead_of_sending` (no command emitted, `pending` stays `None`, `write_warning` holds the command and `WriteKind`); `non_production_sends_immediately_unchanged`; `confirm_sends_the_parked_command_once_and_sets_pending`; `cancel_drops_it_and_sends_nothing`; `second_write_while_warning_open_is_ignored`; `no_warning_when_write_would_be_ignored` (pending in flight, no selection, not connected); `confirm_after_session_closed_or_switched_sends_nothing`; `confirmed_delete_does_not_warn_twice` (a `DocAction::Delete` arriving from the delete confirm sends directly); `session_cleared_drops_a_parked_write`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui write_guard` and confirm failure.
- [ ] **Step 3: Implement.** In `mutation`, after the existing gating (connected, selection present, no pending, no warning open), build the command; if `guard == Warn` and the active connection is production, park it and return no commands; otherwise send as today. `DocAction::Delete` passes `Guard::Confirmed`; Clone and editor saves pass `Warn`. `Confirm` re-checks that the parked command's `session` equals `state.session_id()`, the session is still open and `pending.is_none()`, then sends and sets `pending`; any failed check drops the parked write silently (nothing sent). `Cancel` clears it. `Input::Activate`/`Disconnect` need no change because `Confirm` re-validates.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui` and confirm pass.
- [ ] **Step 5: Commit** `feat(connections): park writes to production connections until the user confirms`

### Task 5: Warning modal and delete notice

**Files:**
- Create: `crates/ui/src/views/write_warning.rs` (register in `views/mod.rs`)
- Modify: `crates/ui/src/app/panels.rs` (`show_modals` at line 188), `crates/ui/src/views/documents.rs` (`show_delete_confirm` at line 73)
- Modify: `crates/ui/locales/en.json`
- Test: `crates/ui/src/views/write_warning.rs` tests, `documents.rs` tests

**Interfaces:**
- Consumes: `AppState.write_warning`, `WriteWarningAction`, `ConnectionBadge`.
- Produces: `pub fn show_write_warning(ctx, parked: &ParkedWrite, connection: &str, badge: &ConnectionBadge, i18n) -> Option<WriteWarningAction>`; keys `write_warning.title`, `write_warning.message_insert|replace|clone|delete` (each with `{connection}`), `write_warning.confirm`, `write_warning.production_line`.

- [ ] **Step 1: Write failing tests** `warning_text_names_the_connection_and_the_action` (for each `WriteKind`, the rendered message contains the connection name), `cancel_button_has_initial_focus_and_escape_cancels` (fail closed, as the host-key prompt does), `confirm_button_confirms`, `delete_dialog_shows_production_line_only_for_production`, `all_new_keys_exist_in_en_json`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui write_warning` and confirm failure.
- [ ] **Step 3: Implement** the modal (heading with production badge and colour glyph, message, Cancel focused first, then Confirm) and draw it from `show_modals` when `write_warning` is `Some`, emitting `Input::WriteWarning`. Add the production line to the delete confirm when the active connection is production.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui` and confirm pass.
- [ ] **Step 5: Commit** `feat(connections): show the production warning dialog before writes`

### Task 6: Compare apply notice and final checks

**Files:**
- Modify: `crates/ui/src/views/compare/apply.rs` (the review dialog); read `crates/ui/src/viewmodel/compare/review.rs` for how the target connection is known
- Modify: `crates/ui/locales/en.json`
- Test: `crates/ui/src/viewmodel/compare/review_tests.rs` or the dialog's own tests

**Interfaces:**
- Consumes: `ConnectionBadge` (Task 3), `AppState::connection`.
- Produces: the apply review dialog shows `write_warning.production_line` with the target's name when the target connection is production. The existing typed confirmation word is **unchanged** (it already exists; this task only adds the notice).

- [ ] **Step 1: Write failing tests** `review_names_production_target` and `review_has_no_production_line_for_non_production_target`.
- [ ] **Step 2: Run** the new tests and confirm failure.
- [ ] **Step 3: Implement** the notice in the review dialog, passing the target's `ConnectionBadge` in from the caller.
- [ ] **Step 4: Run the full gate:** `cargo fmt --all --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked`. Expected: all green. Check coverage stays at or above 80% for the touched modules.
- [ ] **Step 5: Review loop**, per the repo workflow: run the `code-review` skill on `git diff main...HEAD`, fix findings, and re-review each fix until a pass finds nothing new. After any change to `mutation`, re-walk every caller (Insert, Replace, Clone, Delete, and compare apply, which does not use `mutation`) and state what the guard means for each.
- [ ] **Step 6: Commit, push with `-u`, and open a PR against `main`** titled `feat(connections): colour tags and a production warning`, listing the new i18n keys and noting the read-only lock is deferred; file the follow-up ticket for it.
