# Aggregate options, UI and saved (slice 4b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** The person sets aggregate options on the draft and sees them used: an **Options** section (collapsed by default) with a form, its problems in words, an "Options (N)" indicator, the hint picker fed by the collection's indexes, the options sent with every preview, count, explain and confirmed run (4a's `PipelineOptions::default()` placeholders replaced), counts and plan marked out of date when the options change, a status line naming the read preference and the hint, the run dialog listing what it sends, and saved pipelines that keep their options. Copy as code is only **blocked** by an invalid form here; the code itself learns options in 4c.

**Architecture:** The draft holds the applied value `AggregateState.options: PipelineOptions` and a per-session `AggregateState.options_ui: OptionsUi` (the form with its typed text and per-field problems, the open flag, the hint list). A form edit never sends anything by itself: discrete controls apply at once, typed text applies on Enter, on focus loss or after the same pause as a stage edit (`EDIT_PAUSE_SECS`). Applying a valid form that differs from `options` goes through `changed()`, so the request number, the "modified" memo, the run dialog and a running count sequence react exactly as for a stage edit; an invalid form blocks preview, count, explain, run and copy with one reason, keeping the last good preview. The hint list is its own numbered read (`HINT_REQUEST_BASE` request space) routed ahead of the Indexes tab. All new logic lives in new modules; existing files only gain fields, variants, a parameter or a call.

**Tech Stack:** Rust 1.87 (no let-chains), egui/eframe 0.32, egui_kittest, serde. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-16-aggregation-slice-4-design.md` ("Where options live", "UI (slice 4b)", "Safety", "Error mapping" last row, i18n keys 4b, the 4b bullet). Core from 4a: `crates/core/src/pipeline_options.rs` (`PipelineOptions`, `validated`, `is_empty`, `set_count`, `HintOption::parse`, `OptionsError::error_variant`, the `MAX_*` constants), `pipeline_options_apply.rs` (`Operation`, `MaxTimeEffect`, `PipelineOptions::max_time_effect`, `applied`), `test_support/memory_aggregate.rs` (`received_options`). Model the UI on `viewmodel/aggregate_counts.rs` + `app/aggregate_counts_actions.rs` (a part of the draft with its own action enum carried inside `AggregateAction`), `viewmodel/aggregate_view.rs` (`PendingEdit` debounce), `app/index_actions.rs` (numbered `ListIndexes`), `views/aggregate_run.rs` and `tests/accessibility_aggregate_counts.rs`.

## Global Constraints

- Files I create: 200-400 lines (never over 400). Existing files near the limit get a move first (named in the task): `viewmodel/aggregate.rs` (391), `viewmodel/aggregate_counts.rs` (394), `app/aggregate_actions.rs` (380). Existing files over 400 get **no new tests**: new tests go into new sibling `*_options_tests.rs` files (`aggregate_tests.rs` 488, `aggregate_run_tests.rs` 473, `aggregate_counts_actions_tests.rs` 631, `aggregate_run_actions_tests.rs` 704, `views/aggregate_saved_tests.rs` 480, `aggregate_actions_tests.rs` 414, `aggregate_explain_actions_tests.rs` 404); they only get the mechanical edits a changed signature forces. `app/mod.rs` (1000) gains only `mod`/`pub use` lines (+4 at most), `app/reducer.rs` (719) only the one routing `if` of Task 4. `wc -l` every touched file before each commit.
- Immutable style: every op returns a new value (`Self { .., ..self }`); a keystroke handler moves the draft (`std::mem::take`, like `aggregate_saved_actions::dialog_replaced`), never clones it. Real objects over mocks (`AggregateState::default().add_stage(..)`, `PipelineOptions { .. }`, the in-memory store's `received_options`). TDD: failing test, run it, implement, run it. Coverage of new code at least 80%.
- i18n: every visible string, tooltip and accessible name is a key; keys are written as **literals** (in `match` arms, never `format!`), because `i18n_usage_tests.rs` finds keys as literals and fails on an unused or a missing one. `crates/ui/locales/en.json` stays nested and alphabetical (`en_json_is_nested_and_alphabetical_at_every_level`). A key is added in the task whose source first names it. Messages that name a limit fill `{max}` from the core constant (as `saved_message` does). The full English text is in the "en.json" section below; keys beyond the spec: `aggregate.options.collation.locale_pick`, `aggregate.saved.error.invalid_options` (report both).
- Option values are user data: never in a log line or a `Debug` that is logged; log option **names** or `OptionsError::error_variant()` only. Hand-written `Debug` impls (`ExplainState`, `CountsUi`) must not print the options they now hold.
- `cargo +1.87.0`; git as plain separate Bash calls; conventional commits ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push. Never touch `CHANGELOG.md`.
- NEVER connect to any MongoDB already on the machine (the user has others, incl. 27017; `local_e2e_flows.rs` defaults to it, so the URI is **always** set). Integration tests run only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted afterwards (Task 6).

## Review Focus

- What is sent is what is shown: every `AggregatePreview` / `AggregateCount` / `AggregateExplain` / `AggregateRun` carries the draft's applied `options`; a count sequence keeps the options it began with; the run sends the options the dialog listed (an options change closes the dialog through `changed()`; confirm re-checks the form is not blocked). The four `// 4b` placeholders are gone (`grep -rn "4b: the draft" crates/ui/src` is empty).
- An invalid form blocks preview, count, explain, run and copy with `aggregate.options.blocked`, keeps the last good preview, and drops a waiting (queued) preview so the last valid options are not sent behind the person's back; fixing the form previews again even when the options end up equal to the old ones.
- Out of date: counts and plan record the options they were made with; an options change marks both out of date and keeps their numbers; an answer for an older request is stale as before.
- Typed text is debounced like a stage edit and is applied before any other action of the same frame and when the tab is left (`flush_typing`), so Save, Run and Count never see stale form text.
- Saved pipelines: `options` additive, `VERSION` stays 1, absent when empty, unknown keys inside ignored, an invalid known value refuses the whole file (backup + recovery banner); "modified" compares stages **and** options; opening restores them; Save is refused while the form is invalid.
- Hint list: its own request space (`>= HINT_REQUEST_BASE`), never touches `state.indexes` (no banner, no Indexes-tab request numbers), reuses the Indexes tab's list when that holds this namespace, resets on a collection or connection change.
- Read-only sessions: every option usable (no `gated`); production: no extra confirmation for reads.

### Task 1: the options form and its problems (viewmodel)

**Files:** create `crates/ui/src/viewmodel/aggregate_options.rs` (+ `aggregate_options_tests.rs`), `crates/ui/src/viewmodel/aggregate_options_state.rs` (+ `aggregate_options_state_tests.rs`), `crates/ui/src/viewmodel/aggregate_preview_state.rs`; modify `crates/ui/src/viewmodel/mod.rs` (`pub mod aggregate_options; pub mod aggregate_options_state; mod aggregate_preview_state;`), `crates/ui/src/viewmodel/aggregate.rs` (move `Preview`, `PreviewError`, `PreviewState` + `into_last` into `aggregate_preview_state.rs` and `pub use` them back, so every `aggregate::PreviewState` path still compiles; then add the two fields and `PreviewError::Options`), `crates/ui/locales/en.json` (`aggregate.options.problem.*`).

**Produces:**
```rust
// aggregate_options.rs
pub const COMMON_LOCALES: [&str; 10] = ["simple", "en", "en_US", "de", "de@collation=phonebook", "fr", "es", "ja", "zh", "zh_Hant"];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub enum Tri { #[default] Default, On, Off }
impl Tri { pub fn of(v: Option<bool>) -> Self; pub fn value(self) -> Option<bool>; }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum OptionField { Locale, Hint, MaxTime, Comment }
#[derive(Clone, Debug, PartialEq)]
pub enum OptionsEdit {
    CollationOn(bool), Locale(String), Strength(Option<Strength>), CaseLevel(Tri), CaseFirst(Option<CaseFirst>),
    NumericOrdering(Tri), Alternate(Option<Alternate>), HintText(String), HintPicked(String), HintCleared,
    AllowDiskUse(Tri), MaxTime(String), Comment(String), ReadPreference(Option<ReadPreferenceMode>),
}
impl OptionsEdit { pub fn is_typing(&self) -> bool } // Locale, HintText, MaxTime, Comment: wait for Apply
/// The form as typed. `parsed` is recomputed by every op (never per frame).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionsForm { pub collation_on: bool, pub locale: String, pub strength: Option<Strength>,
    pub case_level: Tri, pub case_first: Option<CaseFirst>, pub numeric_ordering: Tri, pub alternate: Option<Alternate>,
    pub hint_text: String, pub allow_disk_use: Tri, pub max_time: String, pub comment: String,
    pub read_preference: Option<ReadPreferenceMode>, parsed: Result<PipelineOptions, Vec<(OptionField, OptionsError)>> }
impl OptionsForm {
    pub fn from_options(o: &PipelineOptions) -> Self;  // hint keys as ejson::doc_to_editor_json, max time as digits
    pub fn edited(self, edit: OptionsEdit) -> Self;
    pub fn parsed(&self) -> Result<&PipelineOptions, &[(OptionField, OptionsError)]>;
    pub fn problem(&self, field: OptionField) -> Option<OptionsError>;
    pub fn is_blocked(&self) -> bool;
}
pub fn problem_key(e: OptionsError) -> &'static str;  // one literal arm per variant: "aggregate.options.problem.<variant>"
pub fn problem_message(i18n: &I18n, e: OptionsError) -> String; // {max} from MAX_COMMENT_CHARS, MAX_HINT_NAME_CHARS, MAX_TIME_MS_CAP

// aggregate_options_state.rs
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionsUi { pub form: OptionsForm, pub open: bool }  // Task 4 adds `pub hint: HintListUi`
/// What applying the form did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied { Unchanged, Changed, Unblocked, Blocked }
impl AggregateState {
    pub fn options_toggled(self) -> Self;
    pub fn options_edited(self, edit: OptionsEdit) -> Self;   // form only; forgets the "modified" memo
    pub fn options_applied(self) -> (Self, Applied);          // valid + different: `Self { options, .. }.changed()`
    pub fn options_reset(self) -> (Self, Applied);            // empty form, then applied
    pub fn options_blocked(&self) -> bool;                    // self.options_ui.form.is_blocked()
    pub fn preview_blocked(self) -> Self;                     // PreviewState::Failed { error: PreviewError::Options, last }, queued: None
}
```
Fields added to `AggregateState`: `pub options: PipelineOptions` (the applied value, what every operation sends) and `pub options_ui: super::aggregate_options_state::OptionsUi`. `Applied::Unblocked`: the form is valid, the options did not change, but the pane shows `PreviewError::Options` (it was blocked), so the caller previews again.

Form rules **(my call)**: a text field that is `""` is unset; text that trims to empty is a problem (`hint_empty`, `comment_empty`), never silently dropped. Collation off ignores the locale and the collation fields (they stay in the form for when it is turned back on). Locale: `""` with collation on is `locale_empty`; other checks through `PipelineOptions { collation, .. }.validated()`. Hint: `HintOption::parse` (a `{` text is keys). Max time: digits only, parsed as `u64`, else or out of `1..=60_000` is `max_time_range`. Comment: `validated()`. Each field is checked on its own so several problems show at once; `parsed` is `Ok` only when all pass and is then `validated()` as a whole.

- [ ] Failing tests in `aggregate_options_tests.rs`: `the_default_form_parses_to_the_empty_options`, `from_options_round_trips_every_field` (a fully set `PipelineOptions` -> form -> `parsed()` equal), `an_empty_text_field_is_unset_and_blank_text_is_a_problem`, `collation_off_ignores_the_locale_and_its_fields`, `a_locale_problem_is_reported_on_the_locale_field` (`""`, `en US`, 33 chars), `a_hint_text_starting_with_a_brace_is_a_key_pattern` (`{"a": 1}` keys, `a_1` name, `{"a": 2}` `hint_keys_invalid`, `{a` `hint_json`), `max_time_takes_digits_from_1_to_60000` (`0`, `60001`, `1.5`, `-3`, `ten` refused), `a_comment_with_a_newline_is_comment_control`, `several_fields_report_their_problems_together`, `hint_picked_replaces_the_text_and_cleared_empties_it`, `typing_edits_wait_for_apply_and_the_others_do_not` (`is_typing` per variant), `every_problem_has_its_own_literal_key` (all ten `OptionsError` variants map to distinct keys that exist in `en.json`, read with `include_str!`), `problem_messages_fill_the_limit_from_the_constant`.
- [ ] Failing tests in `aggregate_options_state_tests.rs`: `a_valid_change_applies_through_changed` (request bumped, `options` replaced, `saved` memo forgotten, run dialog closed, a running count sequence cancelled), `applying_the_same_options_changes_nothing` (request unchanged, `Applied::Unchanged`), `an_invalid_form_keeps_the_last_applied_options` (`Applied::Blocked`, `options` unchanged), `preview_blocked_keeps_the_last_good_preview_and_drops_the_queued_one`, `fixing_the_form_after_a_block_is_unblocked_even_with_equal_options`, `reset_empties_the_form_and_applies`, `toggling_opens_and_closes_without_touching_the_request`, `the_default_draft_has_no_options_and_a_closed_section`.
- [ ] `wc -l crates/ui/src/viewmodel/aggregate.rs` (must be under 400 after the move and the fields); `cargo +1.87.0 test -p curlybrace-ui aggregate_options` and `cargo +1.87.0 test -p curlybrace-ui i18n`; commit `feat(aggregate): add the options form and its problems`.

### Task 2: send the options and block on an invalid form (preview, count, explain, run, copy)

**Files:** create `crates/ui/src/app/aggregate_options_actions.rs` (+ `aggregate_options_actions_tests.rs`), `crates/ui/src/app/aggregate_preview_send.rs` (move `previewed`, `target`, `flushed`, `sent`, `preview_command` out of `aggregate_actions.rs`, `pub(super)` re-used from there), `crates/ui/src/viewmodel/aggregate_stage_count.rs` (move `StageCount` + its `impl`s out of `aggregate_counts.rs`, `pub use`d back), `crates/ui/src/app/aggregate_options_wiring_tests.rs`, `crates/ui/src/viewmodel/aggregate_options_stale_tests.rs`; modify `app/mod.rs` (`mod aggregate_options_actions; mod aggregate_preview_send; pub use aggregate_options_actions::OptionsAction;`), `app/aggregate_actions.rs` (`AggregateAction::Options(OptionsAction)` taken early like `Explain`/`Counts`, and listed in the exhaustive match), `app/aggregate_counts_actions.rs`, `app/aggregate_explain_actions.rs`, `app/aggregate_run_actions.rs`, `viewmodel/aggregate_counts.rs`, `viewmodel/aggregate_counts_plan.rs`, `viewmodel/aggregate_explain.rs`, `viewmodel/aggregate_explain_block.rs`, `viewmodel/code_export.rs`, `viewmodel/code_export_memo.rs`, `viewmodel/aggregate_run_availability.rs`, `viewmodel/aggregate_view.rs` (`counts_stale` passes the options), the views that match on the block enums (`views/aggregate_counts.rs`, `views/aggregate_explain.rs`, `views/code_export_entry.rs`, `views/aggregate_run.rs`, `views/aggregate_preview.rs` for `PreviewError::Options`), the tests and shot fixtures the changed signatures touch (`viewmodel/aggregate_counts_tests.rs`, `aggregate_explain_tests.rs`, `aggregate_view_tests.rs`, `app/aggregate_counts_actions_tests.rs`, `app/aggregate_explain_actions_tests.rs`, `views/aggregate_counts_tests.rs`, `views/aggregate_explain_tests.rs`, `tests/support/explain_shots.rs`, `tests/support/counts_shots.rs`: add `&PipelineOptions::default()`), `crates/ui/locales/en.json` (`aggregate.options.blocked`).

**Produces:**
```rust
// app/aggregate_options_actions.rs
#[derive(Clone, Debug, PartialEq)]
pub enum OptionsAction { Toggle, Edit(OptionsEdit), Apply, Reset }  // Task 4 adds HintListWanted
pub(super) fn on_action(state: AppState, action: OptionsAction) -> Out;
// Edit: draft moved, options_edited; a non-typing edit then applies. Apply/Reset: Changed | Unblocked
// => aggregate_preview_send::previewed; Blocked => preview_blocked, no command; Unchanged => nothing.

// aggregate_preview_send.rs: preview_command takes `options: PipelineOptions` (the draft's), and
// `previewed` refuses to send while `draft.options_blocked()` (=> `preview_blocked`).

// viewmodel/aggregate_counts.rs
pub struct CountSend { pub request: u64, pub stage_index: usize, pub pipeline: Vec<Document>, pub options: PipelineOptions }
impl CountsUi {
    pub fn begun(self, plan: CountPlan, stages: &[Stage], keys: &[u64], options: &PipelineOptions) -> (Self, Option<CountSend>);
    pub fn is_out_of_date(&self, stages: &[Stage], options: &PipelineOptions) -> bool; // counted_for holds both
}
// aggregate_counts_plan.rs: CountsBlock::Options; count_plan checks draft.options_blocked() first; CountsKey gains `blocked: bool`.

// viewmodel/aggregate_explain.rs: Loading and ExplainDone gain `options: PipelineOptions`
impl ExplainUi {
    pub fn begun(self, pipeline: impl Into<Fingerprint>, options: &PipelineOptions) -> (Self, u64, ExplainVerbosity);
    pub fn is_out_of_date(&self, current: Option<&[Document]>, options: &PipelineOptions) -> bool;
}
// code_export.rs: CodeBlock::Options; stages_exported and explain_request return it first when blocked;
// CodeKey (code_export_memo.rs) and ExplainKey gain `blocked: bool`.
// aggregate_run_availability.rs: RunDisabled::Options, checked fresh in can_run after TextNotApplied.
```
Placement in the draft: the explain is sent with `state.aggregate.options.clone()`; a count sequence takes the options at `Start` and every later `CountSend` carries those (an options change cancels the sequence via `changed()`); the run's `confirmed` sends `state.aggregate.options.clone()` and refuses (sends nothing) when `options_blocked()`. Every view arm for the new block variants shows `i18n.t("aggregate.options.blocked")`.

- [ ] Failing tests in `aggregate_options_actions_tests.rs` (reducer level, a connected session with a collection, `AppState` built as `aggregate_actions_tests.rs` does): `a_discrete_edit_previews_with_the_new_options` (the `AggregatePreview` command's `options` equal the edit), `a_typed_edit_sends_nothing_until_apply`, `apply_with_an_invalid_form_sends_nothing_and_shows_the_block`, `an_invalid_form_drops_the_queued_preview` (one in flight, one queued, then Apply invalid, then the in-flight answer: no command), `fixing_the_form_previews_again`, `reset_previews_without_options`, `toggle_sends_nothing`, `options_work_on_a_read_only_session` (preview sent with the options, no banner), `a_production_preview_with_options_needs_no_confirmation`, `options_actions_in_a_batch_send_one_preview` (`Batch([Edit(MaxTime), Apply, Edit(Comment), Apply])`).
- [ ] Failing tests in `aggregate_options_wiring_tests.rs`: `a_count_sequence_sends_the_options_it_began_with` (every `AggregateCount` of the sequence), `an_explain_carries_the_draft_options`, `a_confirmed_run_carries_the_draft_options`, `an_options_change_closes_the_run_dialog`, `a_blocked_form_refuses_count_explain_and_run` (no command, and `can_run` is `Disabled(RunDisabled::Options)`), `copy_as_code_is_blocked_by_an_invalid_form` (`stages_exported` is `Err(CodeBlock::Options)`), `the_default_options_send_what_4a_sent` (every command's `options` is `PipelineOptions::default()` for a draft never touched).
- [ ] Failing tests in `aggregate_options_stale_tests.rs`: `an_options_change_marks_the_counts_out_of_date_and_keeps_their_numbers`, `an_options_change_marks_the_plan_out_of_date`, `the_same_options_again_are_not_out_of_date`, `an_answer_to_a_preview_sent_with_older_options_is_stale` (request numbering), `the_memos_answer_again_when_the_form_blocks_or_unblocks` (count plan, explain request and code block each evaluate once more, not per repaint).
- [ ] `wc -l` the touched files (`aggregate_actions.rs`, `aggregate_counts.rs` under 400); `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(aggregate): send the draft's options and block on an invalid form`.

### Task 3: saved pipelines keep their options

**Files:** create `crates/core/src/saved_pipeline_options_tests.rs` (registered in `saved_pipeline.rs` with `#[cfg(test)] #[path = ...] mod options_tests;`), `crates/ui/src/app/pipelines_options_disk_tests.rs` (registered next to `pipelines_disk_tests` in `app/mod.rs`, one `mod` line), `crates/ui/src/viewmodel/aggregate_saved_options_tests.rs` (registered in `aggregate_saved.rs`), `crates/ui/src/app/aggregate_saved_options_tests.rs` (registered in `aggregate_saved_actions.rs`); modify `crates/core/src/saved_pipeline.rs`, `crates/ui/src/viewmodel/aggregate_saved.rs`, `crates/ui/src/app/aggregate_saved_actions.rs`, `crates/ui/tests/support/aggregate_shots.rs` and `crates/ui/src/app/aggregate_saved_actions_tests.rs` (`options: PipelineOptions::default()` in the `SavedPipeline` literals), `crates/ui/locales/en.json` (`aggregate.saved.error.invalid_options`).

**Produces:**
```rust
// saved_pipeline.rs: drop `Eq` from SavedPipeline and SavedPipelines (PipelineOptions holds a Document)
pub struct SavedPipeline { .., #[serde(default, skip_serializing_if = "PipelineOptions::is_empty")] pub options: PipelineOptions }
/// What a save stores. `From<Vec<SavedStage>>` (no options) keeps every existing caller compiling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SavedDraft { pub stages: Vec<SavedStage>, pub options: PipelineOptions }
impl SavedPipelines {
    pub fn saved(&self, connection: Uuid, name: &str, ns_hint: Option<String>, content: impl Into<SavedDraft>,
        now_unix: i64, replace: bool) -> Result<Self, SavedError>; // a struct, not an 8th parameter (clippy::too_many_arguments)
}
// aggregate_saved.rs
pub struct SavedUi { .., pub baseline_options: PipelineOptions }  // `Eq` dropped from the derive
impl AggregateState {
    pub fn as_saved_draft(&self) -> SavedDraft;  // as_saved_stages + self.options
    // is_modified: a blocked form counts as modified; with a baseline: stages or options differ;
    // without one: stages or options are non-empty. opened_from: options and form restored
    // (OptionsForm::from_options), `open` and the hint list kept. saved_as: baseline_options = options.
}
```
`save_confirmed` refuses with `aggregate.saved.error.invalid_options` when `draft.options_blocked()` (like `invalid_text`) and saves `draft.as_saved_draft()`. `VERSION` stays 1; `checked_list` needs no change (the options validate on deserialize).

- [ ] Failing core tests in `saved_pipeline_options_tests.rs`: `an_entry_without_options_loads_with_none`, `options_are_written_in_mongodb_spelling` (exact JSON of one entry: `collation.locale`, `hint.name`, `allowDiskUse`, `maxTimeMS`, `comment`, `readPreference`), `an_entry_without_options_writes_no_options_key`, `unknown_keys_inside_options_are_ignored`, `an_invalid_option_refuses_the_whole_file` (`strength: 9`, `maxTimeMS: 0`, `readPreference: "tagged"`, `hint: {"keys": {}}`, each), `the_refusal_holds_no_value` (the error text has none of the bad values), `a_file_with_options_still_says_version_one_and_reads_without_them` (deserialize the written file into a local mirror of the version 1 shape without `options`), `saved_keeps_the_options_and_replace_overwrites_them`, `a_vec_of_stages_still_saves_with_no_options`, `a_copied_connection_keeps_the_options`.
- [ ] Failing UI tests: `aggregate_saved_options_tests.rs`: `opening_restores_the_options_and_the_form`, `opening_a_pipeline_saved_before_options_clears_them`, `an_options_change_marks_the_draft_modified_and_back_unmarks_it`, `a_blocked_form_counts_as_modified`, `an_unsaved_draft_with_only_options_is_modified`, `saved_as_takes_the_options_as_the_baseline`, `the_modified_memo_is_forgotten_by_an_options_edit`. `app/aggregate_saved_options_tests.rs`: `save_stores_the_draft_options`, `save_is_refused_while_the_form_is_invalid` (dialog stays, `error == Some("aggregate.saved.error.invalid_options")`, file untouched), `resaving_an_opened_pipeline_replaces_its_options`. `pipelines_options_disk_tests.rs`: `options_survive_a_restart` (write, load with `pipelines_store::load`, equal), `a_file_with_an_invalid_option_is_backed_up_and_the_list_starts_empty` (backup file present, recovery banner as for garbage).
- [ ] `cargo +1.87.0 test -p curlybrace-core saved_pipeline` and `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(aggregate): save pipelines with their options`.

### Task 4: the hint index list

**Files:** create `crates/ui/src/viewmodel/aggregate_hint_list.rs` (+ `aggregate_hint_list_tests.rs`), `crates/ui/src/app/aggregate_hint_actions.rs` (+ `aggregate_hint_actions_tests.rs`); modify `viewmodel/mod.rs`, `app/mod.rs` (`mod aggregate_hint_actions;`), `app/reducer.rs` (one early `if is_hint_list_event(&ev) { return aggregate_hint_actions::on_event(state, ev); }` beside the preview/explain/counts ones, ahead of the big match), `app/aggregate_options_actions.rs` (new variant `OptionsAction::HintListWanted` => `aggregate_hint_actions::wanted`; `Toggle` opening a `Failed` list makes it `NotAsked` again), `viewmodel/aggregate_options_state.rs` (`OptionsUi.hint`), `viewmodel/aggregate.rs` (`on_collection_changed` resets the hint list: one line).

**Produces:**
```rust
// aggregate_hint_list.rs
/// Hint-list requests are numbered from here: the Indexes tab counts up from 0 and never gets near it,
/// so an answer is routed by its number alone and the two never settle each other's requests.
pub const HINT_REQUEST_BASE: u64 = 1 << 62;
pub fn is_hint_request(request: u64) -> bool;
#[derive(Clone, Debug, Default, PartialEq)]
pub enum HintList { #[default] NotAsked, Loading { request: u64 }, Loaded(Arc<[String]>), Failed }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintListUi { pub list: HintList, next: u64 }
impl HintListUi {
    pub fn wants_list(&self) -> bool;                       // NotAsked
    pub fn requested(self) -> (Self, u64);                  // Loading under HINT_REQUEST_BASE + next; next += 1
    pub fn from_names(self, names: Vec<String>) -> Self;    // Loaded (from the Indexes tab or an answer)
    pub fn arrived(self, request: u64, names: Vec<String>) -> Self; // only the awaited request
    pub fn failed(self, request: u64) -> Self;              // only the awaited request
    pub fn retried(self) -> Self;                           // Failed -> NotAsked
    pub fn reset(self) -> Self;                             // NotAsked, counter kept (never reused)
    pub fn names(&self) -> &[String];
}
// aggregate_hint_actions.rs
pub(super) fn wanted(state: AppState) -> Out;  // not wanted: nothing. Indexes tab holds the selected ns
    // (state.indexes.ns == selected, !loading, list non-empty): from_names(spec names), no command.
    // Else with `target()`: Command::ListIndexes { session, ns, request }. No target: Failed.
pub(super) fn is_hint_list_event(ev: &Event) -> bool; // Indexes / RequestFailed{context: "list_indexes"} with a hint request
pub(super) fn on_event(state: AppState, ev: Event) -> Out; // with_session; answers for another ns than the selected one are stale;
    // a failure of any kind (a lost server included) is `Failed` inline: no banner, no session failure
    // (the preview's own read notices a lost server).
```

- [ ] Failing tests in `aggregate_hint_list_tests.rs`: `requests_are_numbered_above_the_base_and_never_reused`, `only_the_awaited_answer_loads_the_list`, `a_late_failure_of_an_older_request_changes_nothing`, `reset_forgets_the_list_but_not_the_counter`, `retried_only_moves_a_failed_list`.
- [ ] Failing tests in `aggregate_hint_actions_tests.rs`: `wanted_reuses_the_indexes_tab_list_for_the_same_collection` (no command), `wanted_sends_one_list_indexes_otherwise` (request `>= HINT_REQUEST_BASE`), `wanted_twice_sends_once`, `the_answer_fills_the_hint_list_and_leaves_the_indexes_tab_alone` (`state.indexes` unchanged, field by field), `a_failure_is_inline_and_raises_no_banner` (`banner == None`, session still connected, also for `CoreError::Unreachable`), `an_answer_for_another_collection_is_stale`, `an_indexes_tab_answer_never_reaches_the_hint_list`, `a_collection_change_resets_the_list`, `a_parked_session_s_answer_lands_in_its_own_draft`, `without_a_connection_the_list_fails_and_reopening_asks_again`.
- [ ] `cargo +1.87.0 test -p curlybrace-ui hint` and `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(aggregate): list the collection's indexes for the hint`.

### Task 5: the Options section, indicator, status line and run dialog (views)

**Files:** create `crates/ui/src/views/aggregate_options.rs` (the toggle, the section layout, the collation group, Reset), `crates/ui/src/views/aggregate_options_fields.rs` (hint, allowDiskUse, maxTimeMS, comment, read preference; the tri-state and enum combos; the text field with its problem line), `crates/ui/src/views/aggregate_options_tests.rs` (kittest, in-crate like `views/aggregate_run_tests.rs`), `crates/ui/src/viewmodel/aggregate_options_view.rs` (+ tests inline or `_tests.rs`), `crates/ui/src/views/aggregate_options_status_tests.rs`; modify `views/mod.rs`, `views/aggregate.rs` (toggle in the header after Refresh; `with_due_options` beside `with_due_edit`; `flush_typing` also yields the waiting `Apply`, batched), `views/aggregate_stage.rs` and `views/aggregate_text.rs` (a `lead` closure drawn first inside their scroll area, so the section scrolls with the editor share), `viewmodel/aggregate_view.rs` (`pub options_typed_at: Option<f64>`: one field), `views/aggregate_preview.rs` (status line, `timeout_set`), `views/aggregate_explain.rs` (`timeout_set` for a lowered explain bound), `views/aggregate_run.rs` (`show_run_dialog(ctx, dialog, options: &PipelineOptions, badge, i18n)`: `run.applies` + `run.primary`), `crates/ui/locales/en.json` (every remaining `aggregate.options.*` key).

**Produces:**
```rust
// views/aggregate_options.rs
pub const TWO_COLUMNS_FROM: f32 = 900.0;
pub(super) fn show_options_toggle(ui: &mut egui::Ui, draft: &AggregateState, i18n: &I18n) -> Option<AggregateAction>;
    // "Options" (tonal) or "Options (N)" = toggle_count with n = options.set_count() (filled_button: the accent);
    // WidgetInfo::selected(WidgetType::Button, true, open, label) so accesskit reports Toggled; hover = toggle_hint
pub(super) fn show_options_section(ui: &mut egui::Ui, draft: &AggregateState, i18n: &I18n) -> Vec<AggregateAction>;
    // nothing when closed; ui.columns(2) at width > TWO_COLUMNS_FROM (collation left, the rest right), else one column;
    // emits OptionsAction::HintListWanted while open and draft.options_ui.hint.wants_list()
// views/aggregate_options_fields.rs
pub(super) struct Typed { pub text: Option<String>, pub commit: bool } // commit on Enter or focus lost
pub(super) fn typed_field(ui: &mut egui::Ui, label: &str, text: &str, hint: &str, problem: Option<String>) -> Typed;
pub(super) fn tri_combo(ui: &mut egui::Ui, id: &str, label: &str, value: Tri, i18n: &I18n) -> Option<Tri>;
// viewmodel/aggregate_options_view.rs
impl AggregateView {
    pub fn options_typed(self, now: f64) -> Self;
    pub fn take_options_due(self, now: f64, force: bool) -> (Self, bool); // true: emit OptionsAction::Apply
}
pub fn options_status(options: &PipelineOptions) -> Vec<(&'static str, Vec<(&'static str, String)>)>; // status.read_from {mode}, status.hint {name}
pub fn run_option_labels(options: &PipelineOptions) -> Vec<&'static str>; // label keys of what applied(Run) carries
pub fn read_preference_key(mode: Option<ReadPreferenceMode>) -> &'static str;
pub fn timeout_key(options: &PipelineOptions, op: Operation, default_key: &'static str) -> &'static str; // timeout_set when max_time_effect(op) == Lowered
```
Controls, in Tab order, each named by its label key: Collation (checkbox), Locale (text, placeholder `locale_hint`) + Common locales (combo of `COMMON_LOCALES`, `locale_pick`), Strength (combo: `server_default` + `strength_value.1..5`), Case level, Case first, Numeric ordering, Alternate (combos, `server_default`/`tri.*`/`*_value.*`); Existing indexes (combo of `HintListUi::names`, with `hint.loading` / `hint.list_failed` in its place), Hint (text, `hint.placeholder`), Clear (`hint.clear`), a `hint.not_listed` note when the list is loaded and the typed name is not in it (a note, never a block: the server answers `errors.hint_not_found`); Allow disk use (tri) with `allow_disk_use.preview_note`; Time limit (text, `max_time.placeholder` with `{preview}`/`{run}` from `PREVIEW_MAX_TIME_MS`/`AGGREGATE_RUN_MAX_TIME_MS` in seconds) with `max_time.clamped` (`{limit}`) when `max_time_effect(Operation::Preview) == Capped`; Comment (text) with `comment_sent_note`; Read preference (combo, `read_preference.value.*`); Reset options. Problems show under their field with `problem_message`, in the error colour. Status: under the preview status lines while the pane is not idle, one line per `options_status` entry; `PreviewError::Options` reads `blocked`. Run dialog: `run.applies` with `{options}` = the translated labels joined by `", "`, and `run.primary` when a read preference other than `Primary` is set.

- [ ] Failing tests in `views/aggregate_options_tests.rs`: `the_toggle_reads_options_then_options_with_the_count`, `the_toggle_reports_its_expanded_state`, `a_closed_section_draws_nothing`, `typing_a_locale_emits_an_edit_and_apply_after_the_pause`, `enter_applies_the_typed_text_at_once`, `another_action_in_the_frame_applies_the_typing_first`, `leaving_the_tab_flushes_the_typed_options`, `a_problem_shows_under_its_field_in_words`, `allow_disk_use_says_explain_and_run_only`, `a_time_limit_above_ten_seconds_says_previews_stop_there`, `the_hint_combo_lists_the_indexes_and_picking_sets_the_hint`, `the_hint_list_says_loading_and_could_not_list`, `a_name_not_among_the_listed_indexes_is_noted`, `the_open_section_asks_for_the_hint_list`, `reset_emits_reset`, `the_section_has_two_columns_when_wide_and_one_when_narrow`.
- [ ] Failing tests in `views/aggregate_options_status_tests.rs`: `the_status_line_names_the_read_preference_and_the_hint` (name, and keys as compact JSON), `no_option_no_status_line`, `a_blocked_preview_says_why_and_keeps_the_last_result`, `a_timeout_under_a_lowered_limit_says_you_set_it` (preview and explain; an unchanged limit keeps `aggregate.error.timeout`), `the_run_dialog_lists_the_options_it_sends`, `the_run_dialog_states_the_primary_rule_only_for_a_non_primary_read_preference`.
- [ ] `wc -l crates/ui/src/views/aggregate*.rs crates/ui/src/viewmodel/aggregate_view.rs`; `cargo +1.87.0 test -p curlybrace-ui`; commit `feat(aggregate): show the options section, indicator and status`.

### Task 6: accessibility, layout, snapshots, end to end, README, full check

**Files:** create `crates/ui/tests/accessibility_aggregate_options.rs`, `crates/ui/tests/aggregate_options_layout.rs`, `crates/ui/tests/support/options_shots.rs` (registered in `tests/support/mod.rs`), `crates/ui/tests/local_aggregate_options_flows.rs`; modify `crates/ui/tests/aggregate_snapshots.rs` (three shots), `tests/snapshots/aggregate_options*.png` (new), `README.md`.

- [ ] `accessibility_aggregate_options.rs` (real English locale, driven like `accessibility_aggregate_counts.rs`): `every_options_control_has_a_role_and_a_name`, `tab_walks_the_section_in_reading_order` (toggle, collation, locale, common locales, strength, case level, case first, numeric ordering, alternate, existing indexes, hint, clear, allow disk use, time limit, comment, read preference, reset), `the_toggle_is_toggled_and_named_with_the_count`, `a_problem_is_text_under_its_field`, `every_option_is_usable_on_a_read_only_connection`, `the_run_dialog_reads_the_options_and_the_primary_rule`.
- [ ] `aggregate_options_layout.rs` (accesskit bounds of the real app, as `aggregate_layout.rs`): `at_800x500_the_open_section_is_one_column_and_nothing_overlaps_or_leaves_the_window`, `at_1000x700_the_open_section_is_two_columns_and_nothing_overlaps`, `the_open_section_scrolls_with_the_stage_list_and_the_preview_keeps_its_place` (preview heading within `PREVIEW_BUDGET`).
- [ ] Snapshots (`options_shots.rs`: the three-stage draft, section open, collation `en` strength 2, hint `status_1`, maxTimeMS 5000, read preference `secondaryPreferred`, one problem on the comment): `aggregate_options` (1000x700), `aggregate_options_dark`, `aggregate_options_small` (`Shot::sized` 800x500). Write them with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1 cargo +1.87.0 test -p curlybrace-ui --test aggregate_snapshots -- --include-ignored --nocapture`, look at each PNG, then run again without `UPDATE_SNAPSHOTS` (all pass, the existing aggregate PNGs unchanged).
- [ ] `local_aggregate_options_flows.rs` (`#[ignore]` + `local_enabled`, own `cbit_*` database dropped at the end, the `Fixture` of `local_e2e_flows.rs`): `a_collation_set_in_the_section_changes_the_preview` (strength 2: `$match {name: "abc"}` shows `ABC` and `abc`), `a_hint_on_a_missing_index_says_so_in_the_preview` (`errors.hint_not_found` text), `the_hint_combo_lists_the_collection_s_indexes`, `a_saved_pipeline_reopens_with_its_options`. Throwaway server, from the worktree:
```bash
PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
DBPATH=$(mktemp -d /private/tmp/claude-501/cbit-4b.XXXXXX)
/opt/homebrew/bin/mongod --dbpath "$DBPATH" --bind_ip 127.0.0.1 --port "$PORT" --fork --logpath "$DBPATH/log" --pidfilepath "$DBPATH/pid"
CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" cargo +1.87.0 test -p curlybrace-ui --test local_aggregate_options_flows -- --ignored
kill "$(cat "$DBPATH/pid")"; rm -rf "$DBPATH"
```
- [ ] README: in **Aggregate**, a new bullet after the preview bullets: the Options section (what each option does, collapsed by default, "Options (N)"), what each operation sends (allowDiskUse only on Explain and Run; the time limit only lowers, 10 s previews / 60 s runs; the run always on the primary), the comment reaches the server's logs and profiler, an invalid option blocks preview, count, explain, run and copy with the reason, the status line, saved pipelines keep options (an older build opens the file and drops them only when it re-saves that pipeline), a read preference no member satisfies can answer as "took too long" before the server-selection wait ends. Copy as code does **not** include options yet (4c). Roadmap lines (`Aggregation builder` built / in progress): options built, code export of options and the explain hint picker next.
- [ ] Full check: `cargo +1.87.0 fmt --all`, `cargo +1.87.0 clippy --workspace --all-targets --locked -- -D warnings`, `cargo +1.87.0 test --workspace --locked`; coverage of the new modules at least 80% (`cargo llvm-cov` if installed, else list the tests per public function in the PR body); `wc -l` of every touched file (report `app/mod.rs` and `app/reducer.rs` before and after); `grep -rn "4b: the draft" crates/ui/src` empty.
- [ ] Commit `test(aggregate): accessibility, layout and snapshots of the options` (tests + PNGs) and `docs(aggregate): describe aggregate options in the README`. Report the new i18n keys (the two beyond the spec named).

## en.json (under `aggregate`, between `no_stages` and `preview`; plus one key in `aggregate.saved.error`)

```json
"options": {
  "allow_disk_use": { "label": "Allow disk use", "preview_note": "Applies to Explain and Run only. Previews and counts never spill to disk." },
  "blocked": "Fix the options to preview, count, explain, run or copy this pipeline.",
  "collation": {
    "alternate": "Alternate",
    "alternate_value": { "non_ignorable": "Non-ignorable", "shifted": "Shifted" },
    "case_first": "Case first",
    "case_first_value": { "lower": "Lower case first", "off": "Off", "upper": "Upper case first" },
    "case_level": "Case level",
    "enabled": "Collation",
    "locale": "Locale",
    "locale_hint": "simple, or a locale such as en, de or fr_CA",
    "locale_pick": "Common locales",
    "numeric_ordering": "Numeric ordering",
    "server_default": "Server default",
    "strength": "Strength",
    "strength_value": { "1": "1: base letters", "2": "2: and accents", "3": "3: and case", "4": "4: and punctuation", "5": "5: identical" }
  },
  "comment": { "label": "Comment", "placeholder": "Text to find this pipeline in the server's logs" },
  "comment_sent_note": "The comment is sent to the server and shows in its logs and profiler.",
  "hint": { "clear": "Clear", "existing": "Existing indexes", "label": "Hint", "list_failed": "Could not list the indexes of this collection.", "loading": "Listing indexes...", "not_listed": "No index of this collection has this name.", "placeholder": "Index name or key pattern" },
  "max_time": { "clamped": "Previews, counts and explains still stop at {limit} s.", "label": "Time limit (ms)", "placeholder": "Default: {preview} s, runs {run} s" },
  "problem": {
    "comment_control": "The comment cannot hold a newline, a tab or another control character.",
    "comment_empty": "The comment is only spaces. Type a comment or clear the field.",
    "comment_too_long": "The comment can be at most {max} characters.",
    "hint_empty": "The hint is only spaces or holds a control character. Type an index name or clear the field.",
    "hint_json": "The key pattern is not valid JSON.",
    "hint_keys_invalid": "Each key needs 1, -1 or an index type such as text; $natural must be the only key.",
    "hint_name_too_long": "An index name can be at most {max} characters.",
    "locale_empty": "Type a locale, or turn the collation off.",
    "locale_invalid": "This is not a locale. Use simple, or letters such as en or fr_CA, with an optional @collation=name.",
    "max_time_range": "Type a whole number of milliseconds from 1 to {max}."
  },
  "read_preference": { "label": "Read preference", "value": { "default": "Connection default", "nearest": "nearest", "primary": "primary", "primary_preferred": "primaryPreferred", "secondary": "secondary", "secondary_preferred": "secondaryPreferred" } },
  "reset": "Reset options",
  "run": { "applies": "This run sends these options: {options}", "primary": "A run always reads and writes on the primary, whatever the read preference says." },
  "status": { "hint": "hint: {name}", "read_from": "Read from {mode}" },
  "timeout_set": "It took longer than the time limit you set in Options. The last result stays on screen.",
  "toggle": "Options",
  "toggle_count": "Options ({n})",
  "toggle_hint": "Collation, index hint, disk use, time limit, comment and read preference for this pipeline",
  "tri": { "default": "Server default", "off": "Off", "on": "On" }
}
// aggregate.saved.error.invalid_options: "An option has a problem. Fix it in Options before you save."
```
Written one key per line in the file, as the rest of `en.json` (the inline objects above are for reading only).
