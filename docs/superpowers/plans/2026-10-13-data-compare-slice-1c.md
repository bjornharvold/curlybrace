# Data Compare, Slice 1c: Multi-Pair UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let one compare cover many collection pairs: pair a whole database against another in one click, hand-pair differently named collections, give a pair its own key/filter/rules, check and run all pairs one after another, see a run overview, and open the results of any pair.

**Architecture:** The runtime protocol and runner are already pair-aware (slice 1), so this slice adds one small pure core module (`compare::pairing`) and otherwise changes only `curlybrace-ui`: the setup state becomes *defaults + a pair list*, the single-run state becomes a per-pair *run overview*, results and detail are kept per pair, and the preflight runs as a queue of the existing one-pair `Check`. No new commands or events.

**Tech Stack:** Rust, egui/eframe 0.32, `curlybrace-core` compare protocol (slice 1/1b), wgpu snapshot tests (non-blocking CI job).

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` ("Slice 1c (multiple pairs)", "Pairing helper", testing items 6b and 7, the "Slice 1 runtime, as built" notes). Issue #8. Slices 1 and 1b shipped in PRs #29 and #32.

## Decisions made while planning

1. **No protocol change.** The spec says slice 1c "changes no events". Two apparent gaps are closed in the UI instead:
   - *Multi-pair preflight:* `Check` takes one pair and `Checked` carries the pair id. The UI sends one `Check` at a time from a queue and uses a **probe `PairId`** (a fresh id from the session counter) per check, so an answer identifies its entry and a stale answer is recognisable. The "one `Check` in flight" rule from slice 1 stays.
   - *`Rows` has no run id (slice 1 limitation):* the UI never reuses a `PairId`. Every run assigns **fresh `PairId`s from `next_pair`** to the pairs it starts, and keeps `PairId -> EntryId` in the overview row. A late page for an earlier run's pair id can then never be mistaken for the new run's.
2. **Entries vs pairs.** A **pair entry** (`EntryId`, stable, user-visible, editable) is what the list holds. A **pair** (`PairId`) is one entry's appearance in one run or check. The overview row for an entry shows its latest `PairId`.
3. **A pair stores both full namespaces** (database and collection per side), so the pair list is independent of the database pickers. The pickers only feed **Add pair** and **Add all same-named**; changing a picker never touches existing entries.
4. **Defaults and overrides:** the existing top-of-form fields (key, rules, the two filters) become the **defaults**. An entry holds `PairOverrides` of `Option`s; `None` follows the default. Changing a default never overwrites a deliberate override, and `is_customised()` drives the marker in the list.
5. **Pairing rules:** a name that is a view on either side is never offered; time-series collections count as collections; `system.*` is hidden unless the user switches it on; names match exactly. `OnlyInSource`/`OnlyInTarget` names are listed as "no counterpart" and can be paired with a name the user types (a missing collection counts as empty).
6. **Run selection:** each entry has an *included* flag (default on). **Run** starts the included entries; if the preflight refused some, a second button **Run passing** starts only the included entries that passed. A refused entry that is run anyway fails on its own with `CompareKeyNotIndexed` (the runner re-checks), which is the "run all" behaviour in the spec.
7. **Re-run** selects that entry for editing, includes only it, and clears its verdict; the user changes the key and checks/runs again. Other entries' results stay addressable (the runtime keeps results per pair id until discarded).
8. **Per-pair selection** (slice 2's apply selection) does not exist yet. "Switching pair keeps each pair's own selection" is met for what exists: each pair keeps its own `ResultsView` (category, page, rows); the open detail is cleared when switching pairs.
9. **Deviations accepted, recorded in the spec:** no per-pair duration (the pure reducer has no clock, and `Summary` has no `duration_ms`); a pair that fails on the shared budget shows `CompareCapExceeded` (the runtime has no `NotRun`); progress is per running pair, not throttled.
10. **"Compare database to..."** is an entry on a database row of the Browse sidebar. It opens the session like "Compare to...", picks the source database, and picks the **same database name on the target** when the target connection differs from the source (cross-server sync is the common case); on one connection the target database is left for the user.

## Global Constraints

- Read-only slice: nothing writes to any database.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical). No literal English in views, view-models or the reducer. Report new keys.
- Immutability; files < 800 lines (aim 200-400; `app/compare.rs` is already 751, so Task 5 splits it first); functions < 50 lines; nesting <= 4.
- Keys and documents are data: never logged (slice 1 `failure_log_line` rule holds; new code adds no logging of keys, namespaces' document content or filters).
- TDD for every task. Default `cargo test --workspace --locked` stays DB-, docker- and GPU-free. Tier-1 tests are `#[ignore]` + env-gated, use only throwaway `cbit_<id>` databases and only a `cb-it-dev-*` container the test run starts itself, never the user's `local7979`/`local265` or `127.0.0.1:27017`. GitHub CI must never need MongoDB (`scripts/check-no-db-in-ci.sh` stays green).
- Commits: conventional format ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Before every push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Sub-agents default to Sonnet; the reducer/state-machine tasks (4 and 5) and the final review use a stronger model.

## Review Focus

1. **Stale answers across pairs and runs.** A `Checked`, `Progress`, `PairDone`, `Rows` or `Detail` for an entry that was edited, removed, re-run or whose session was replaced must not change visible state. Pinned in Tasks 3, 4 and 5 (fresh-`PairId` tests, queue-invalidation tests).
2. **Defaults overwriting overrides.** Changing the default key/rules/filter must leave every overriding entry unchanged and every non-overriding entry following it. Pinned in Task 2.
3. **One run slot, many pairs.** Cancel mid-run, a failing pair between two good ones, a session closed mid-run, Start while running, Start with zero included entries. Pinned in Tasks 4 and 5.
4. **Whole-database pairing edge cases.** Empty databases, only-one-side names, views, `system.*`, names differing only by case, hundreds of collections (list stays usable, nothing quadratic per frame). Pinned in Tasks 1 and 6.
5. **Memory and budget.** Discarding a row releases its results and sends `Discard` for the right `PairId`; removing an entry that has results discards them; re-running discards the old `PairId`. Pinned in Task 5.

## File Structure

```
crates/core/src/compare/
  pairing.rs                        NEW  suggest(), Suggestion, PairingOptions                           (Task 1)
  mod.rs                            + pub mod pairing                                                     (Task 1)
crates/ui/src/viewmodel/compare/
  pairs.rs                          NEW  EntryId, PairEntry, PairOverrides, PairList, resolution          (Task 2)
  pairing_view.rs                   NEW  DatabasePairing: suggestions to rows, include_system, add-all   (Task 2)
  checks.rs                         NEW  CheckQueue, EntryCheck (per-entry verdicts)                      (Task 3)
  overview.rs                       NEW  RunOverview, RunRow, PairStatus, Totals                          (Task 3)
  results.rs                        ResultsView gains `pair: PairId`; PAIR_ID use removed                 (Task 3)
  mod.rs                            CompareState: pairs, checks, overview, results map, open_pair         (Task 3)
  setup.rs                          SetupForm keeps pickers + defaults; PAIR_ID removed                   (Task 2)
crates/ui/src/app/
  compare.rs                        actions + session/setup handling (split, < 800 lines)                 (Task 5)
  compare_pairs.rs                  NEW  pair-list actions, queue checks                                  (Task 5)
  compare_run.rs                    NEW  Start/Cancel/Progress/PairDone/RunDone/Rows/Detail per pair      (Task 5)
  compare_duplicates.rs             per-pair duplicates                                                    (Task 5)
  actions.rs, viewmodel/sidebar.rs  SidebarAction::CompareDatabaseTo                                      (Task 7)
crates/ui/src/views/compare/
  pairs.rs                          NEW  pair list table, add/add-all/no-counterpart, per-pair editor     (Task 6)
  setup.rs                          defaults section                                                       (Task 6)
  preflight.rs                      per-entry verdicts, Run / Run passing                                  (Task 6)
  overview.rs                       NEW  run overview + totals + row actions                               (Task 6)
  results.rs, mod.rs                back to overview, open pair                                           (Task 6)
  ../sidebar.rs                     database context menu                                                  (Task 7)
crates/ui/locales/en.json           compare.pairs.*, compare.overview.*, sidebar.compare_database_to      (Tasks 6, 7)
crates/ui/tests/                    snapshot states + tier-1 flow                                         (Task 8)
docs/ README.md site/index.html     docs                                                                   (Task 9)
```

---

### Task 1: Pairing helper (core)

**Files:**
- Create: `crates/core/src/compare/pairing.rs`
- Modify: `crates/core/src/compare/mod.rs` (add `pub mod pairing;`)

**Interfaces:**
- Consumes: `crate::store::CollectionKind` (`Collection`, `View`, `Timeseries`).
- Produces:

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Suggestion {
    Matched { name: String },
    OnlyInSource { name: String },
    OnlyInTarget { name: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PairingOptions {
    pub include_system: bool,
}

/// Matches collection names between two databases. Output order: all `Matched` by name, then
/// `OnlyInSource` by name, then `OnlyInTarget` by name (byte order, so it is deterministic).
pub fn suggest(
    source: &[(String, CollectionKind)],
    target: &[(String, CollectionKind)],
    opts: PairingOptions,
) -> Vec<Suggestion>
```

Rules: names match exactly (case-sensitive); a name that is a `View` on **either** side is dropped entirely; `system.*` names are dropped unless `include_system`; duplicate names within one input list count once.

- [ ] **Step 1: Write the failing tests** in `pairing.rs` (`#[cfg(test)] mod tests`), using a helper `fn c(names: &[&str]) -> Vec<(String, CollectionKind)>` (all `Collection`) and `fn with(kind, name)`:
  - `matches_same_names_and_sorts_groups`: source `[b, a, x]`, target `[a, b, y]` gives `[Matched a, Matched b, OnlyInSource x, OnlyInTarget y]`.
  - `names_are_case_sensitive`: `Users` vs `users` are `OnlyInSource Users`, `OnlyInTarget users`.
  - `view_on_either_side_is_never_offered`: view `v` on source with a collection `v` on target gives nothing for `v`; view only on target likewise.
  - `timeseries_counts_as_a_collection`: `Timeseries` `t` on both sides is `Matched t`.
  - `system_collections_hidden_unless_enabled`: `system.profile` absent by default, `Matched system.profile` with `include_system: true`.
  - `duplicate_input_names_count_once`.
  - `empty_inputs_give_nothing` and `one_empty_side_lists_everything_as_only_in_the_other`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-core compare::pairing` and confirm the tests fail to compile/fail.
- [ ] **Step 3: Implement** with two `BTreeMap<&str, CollectionKind>` (dedupe + sort), filter views/system, then walk the union.
- [ ] **Step 4: Run** the tests; all pass. Run clippy for the crate.
- [ ] **Step 5: Commit** `feat(compare): add collection pairing helper (slice 1c)`.

---

### Task 2: Pair list, overrides and database pairing (view-model)

**Files:**
- Create: `crates/ui/src/viewmodel/compare/pairs.rs`, `pairs_tests.rs`, `pairing_view.rs`, `pairing_view_tests.rs`
- Modify: `crates/ui/src/viewmodel/compare/setup.rs` (remove `PAIR_ID`; `SetupForm::pair()` takes the id), `viewmodel/compare/mod.rs` (modules + re-exports)

**Interfaces:**
- Consumes: `SetupForm`, `SideForm`, `RulesForm`, `SetupError`, `Namespace`, `PairSpec`, `PairId`, Task 1's `Suggestion`.
- Produces:

```rust
pub type EntryId = u32;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PairOverrides {
    pub key_text: Option<String>,
    pub source_filter: Option<String>,
    pub target_filter: Option<String>,
    pub rules: Option<RulesForm>,
}
impl PairOverrides { pub fn is_customised(&self) -> bool }

#[derive(Clone, Debug, PartialEq)]
pub struct PairEntry {
    pub id: EntryId,
    pub source: Namespace,
    pub target: Namespace,
    pub included: bool,
    pub overrides: PairOverrides,
}
impl PairEntry {
    /// The defaults with this entry's overrides applied, as a form that can parse itself.
    pub fn effective(&self, defaults: &SetupForm) -> SetupForm;
    pub fn issues(&self, defaults: &SetupForm, same_connection: bool) -> Vec<SetupError>;
    pub fn spec(&self, id: PairId, defaults: &SetupForm, same_connection: bool) -> Result<PairSpec, SetupError>;
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PairList { pub entries: Vec<PairEntry>, pub next_entry: EntryId, pub editing: Option<EntryId> }
impl PairList {
    pub fn added(self, source: Namespace, target: Namespace) -> Self;   // ignores an exact duplicate (same source+target)
    pub fn removed(self, id: EntryId) -> Self;                          // clears `editing` if it was this entry
    pub fn included(self, id: EntryId, on: bool) -> Self;
    pub fn only_included(self, id: EntryId) -> Self;                     // Re-run
    pub fn with_overrides(self, id: EntryId, o: PairOverrides) -> Self;
    pub fn editing(self, id: Option<EntryId>) -> Self;
    pub fn get(&self, id: EntryId) -> Option<&PairEntry>;
}

/// The database-level pairing helper's view: what the helper offers and what is already paired.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DatabasePairing { pub include_system: bool }
impl DatabasePairing {
    pub fn rows(&self, setup: &SetupForm, list: &PairList) -> Vec<PairingRow>;
    pub fn matched_to_add(&self, setup: &SetupForm, list: &PairList) -> Vec<(Namespace, Namespace)>; // not yet paired
}
pub enum PairingRow { Matched { name: String, paired: bool }, NoCounterpart { side: Side, name: String } }
```

The spec resolved with `spec()` keeps the existing validation (view refused, key, filters, rules, same-collection) and the id passed in; `SameCollection` is raised only when `same_connection` and both namespaces and parsed filters are equal. Adding a pair whose collection is a view (per `SideForm::collections`) is refused by the reducer with `SetupError::ViewSelected(side)` (Task 5); the view-model helper `SetupForm::pick_pair(&self) -> Result<(Namespace, Namespace), SetupError>` returns it.

- [ ] **Step 1: Write failing tests** (`pairs_tests.rs`, `pairing_view_tests.rs`):
  - `added_assigns_stable_increasing_ids` and `added_ignores_an_exact_duplicate_pair`.
  - `removed_clears_editing_when_it_was_the_edited_entry`.
  - `defaults_apply_to_entries_without_overrides`: change `key_text` in defaults, the entry's `spec()` key follows.
  - `changing_a_default_never_overwrites_an_override`: entry overrides key `sku`; default key changes `_id` -> `code`; entry spec key stays `sku`, a sibling entry follows `code`. Same for each of source filter, target filter, rules.
  - `is_customised_reports_any_override_and_none_for_default_entries`.
  - `spec_uses_the_given_pair_id_and_each_entrys_namespaces`.
  - `same_collection_only_raised_on_one_connection`.
  - `invalid_override_key_reports_key_error_for_that_entry_only`.
  - `only_included_unticks_the_others`.
  - Pairing view: `rows_list_matched_then_no_counterpart_in_core_order`, `matched_row_is_marked_paired_once_added`, `include_system_toggles_system_collections`, `matched_to_add_skips_already_paired_names`, `views_are_not_offered` (uses the kinds in `SideForm::collections`), `no_database_picked_gives_no_rows`.
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui viewmodel::compare` and confirm failure.
- [ ] **Step 3: Implement.** `effective()` builds a `SetupForm` from the defaults, replaces each side's `database`/`collection` with the entry's namespace and the filter texts/key/rules with overrides where `Some`; `spec()` calls `effective(..).pair()` then sets `id` and does the same-connection check. Remove `PAIR_ID` from `setup.rs` (the compiler lists the remaining users, which Tasks 3 and 5 own: keep a temporary `#[deprecated]-free private const` in the old files only until those tasks land, then delete).
- [ ] **Step 4: Run** the view-model tests; `cargo build -p curlybrace-ui` must still compile.
- [ ] **Step 5: Commit** `feat(compare): pair list and database pairing view-models (slice 1c)`.

---

### Task 3: Check queue, run overview, per-pair results (view-model)

**Files:**
- Create: `viewmodel/compare/checks.rs`, `checks_tests.rs`, `overview.rs`, `overview_tests.rs`
- Modify: `viewmodel/compare/results.rs` (+ tests), `viewmodel/compare/mod.rs`, `viewmodel/compare/preflight.rs` (reuse `PreflightView`), the existing view-model tests that built `RunView`

**Interfaces:**
- Consumes: Task 2's `EntryId`/`PairList`, `PreflightView`, `Preflight`, `Summary`, `PairOutcome`, `CoreError`.
- Produces:

```rust
// checks.rs
#[derive(Clone, Debug, Default, PartialEq)]
pub enum EntryCheck { #[default] Unchecked, Queued, Checking(PairId), Done(PreflightView, Preflight), Failed }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CheckQueue { pub entries: BTreeMap<EntryId, EntryCheck> }
impl CheckQueue {
    pub fn queued(ids: &[EntryId]) -> Self;                     // all Queued
    pub fn in_flight(&self) -> Option<(EntryId, PairId)>;
    pub fn next_to_send(&self) -> Option<EntryId>;              // first Queued, only when nothing is Checking
    pub fn sending(self, id: EntryId, probe: PairId) -> Self;   // Queued -> Checking(probe)
    pub fn answered(self, probe: PairId, p: Preflight) -> (Self, bool); // false when `probe` is not the in-flight one (stale)
    pub fn failed(self, probe: PairId) -> Self;                 // Checking -> Failed
    pub fn verdict(&self, id: EntryId) -> Option<&PreflightView>;
    pub fn all_settled(&self, ids: &[EntryId]) -> bool;         // every id Done or Failed
    pub fn allows_start(&self, id: EntryId) -> bool;            // Done with PreflightView::allows_start
    pub fn invalidated(self) -> Self;                           // everything back to Unchecked (an edit)
}

// overview.rs
#[derive(Clone, Debug, PartialEq)]
pub enum PairStatus {
    Pending,
    Running { scanned_source: u64, scanned_target: u64, differences: u64 },
    Completed(Summary),
    Cancelled(Summary),
    Failed(CoreError),
    Discarded,
}
#[derive(Clone, Debug, PartialEq)]
pub struct RunRow { pub entry: EntryId, pub pair: PairId, pub run: RunId, pub status: PairStatus }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunOverview { pub rows: Vec<RunRow>, pub active: Option<ActiveRun> } // one row per entry that has run
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveRun { pub run: RunId, pub cancel_requested: bool }
impl RunOverview {
    pub fn started(self, run: RunId, pairs: &[(EntryId, PairId)]) -> (Self, Vec<PairId>);
    //   replaces the row of each entry (returns the superseded PairIds so the reducer can Discard them),
    //   all new rows Pending, keeps rows of entries not in this run
    pub fn progressed(self, run: RunId, pair: PairId, p: (u64, u64, u64)) -> Self;  // only the active run's Pending/Running row
    pub fn pair_done(self, run: RunId, pair: PairId, o: PairOutcome) -> Self;       // Running/Pending -> terminal; ignored if run/pair do not match
    pub fn run_done(self, run: RunId) -> Self;                                      // clears `active`; any still-Pending row of that run becomes Cancelled(zero) only if cancelled, else untouched
    pub fn cancelling(self) -> Self;
    pub fn discarded(self, pair: PairId) -> Self;                                   // Completed/Cancelled/Failed -> Discarded
    pub fn removed_entry(self, entry: EntryId) -> (Self, Option<PairId>);
    pub fn row_for(&self, entry: EntryId) -> Option<&RunRow>;
    pub fn totals(&self) -> Totals;      // only_in_source, only_in_target, different, identical, completed, failed, cancelled
    pub fn is_running(&self) -> bool;    // active.is_some()
    pub fn can_cancel(&self) -> bool;
}
```

`ResultsView` gains `pub pair: PairId` (set by `ResultsView::new(pair, &summary)`), and `request()`/`select()` use it; the file no longer imports `PAIR_ID`.

- [ ] **Step 1: Write failing tests:**
  - Checks: `queue_sends_one_at_a_time_in_order`, `next_to_send_is_none_while_one_is_checking`, `answer_for_an_unknown_probe_is_stale`, `answered_stores_the_preflight_verdict`, `failed_marks_only_the_in_flight_entry`, `invalidated_resets_everything_including_in_flight`, `all_settled_needs_every_id`, `refused_verdict_does_not_allow_start`, `empty_both_allows_start`.
  - Overview: `started_replaces_rows_of_rerun_entries_and_returns_the_old_pair_ids`, `started_keeps_rows_of_entries_not_in_the_run`, `progress_for_another_run_or_pair_is_ignored`, `pair_done_for_a_wrong_pair_id_is_ignored`, `a_failing_pair_leaves_its_neighbours_untouched`, `run_done_clears_active`, `totals_sum_completed_pairs_only`, `cancel_marks_cancel_requested_once`, `discarded_only_applies_to_terminal_rows`, `removed_entry_returns_the_pair_id_that_needs_a_discard`.
  - Results: existing tests updated to pass a pair id; add `request_and_select_use_the_views_own_pair_id`.
- [ ] **Step 2: Run** the view-model tests; confirm failure.
- [ ] **Step 3: Implement** the three files. Replace `RunView`/`cancel_requested`/`check_in_flight`/`check_stale`/`preflight`/`last_check` in `CompareState` with `checks: CheckQueue`, `overview: RunOverview`, `pairs: PairList`, `pairing: DatabasePairing`, `results: BTreeMap<PairId, ResultsView>`, `open_pair: Option<PairId>`, `detail: Option<DetailView>`, `next_pair: PairId` (starts at 1, never reused), keep `session`, `connections`, `next_session`, `next_run`, `setup`, `duplicates`. `session_dropped()` keeps the id counters (`next_session`, `next_run`, `next_pair`). Re-implement the `can_*` predicates on the new fields (`can_start` = ready, nothing running, no check Queued/Checking, at least one included entry with a valid spec, and every included entry settled with `allows_start`; `can_start_passing` = same but only passing entries need to exist). Update `gate_tests.rs` and the other tests that build `CompareState` accordingly.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui viewmodel`. The reducer (`app/`) will not compile until Task 5; **keep the workspace compiling in this task** by adapting `app/compare.rs` minimally (map the old single-pair behaviour onto a one-entry list) *or* land Tasks 3 and 5 on one commit sequence with the build green at each commit. Decision: adapt minimally here so every commit builds.
- [ ] **Step 5: Commit** `feat(compare): check queue, run overview and per-pair results state (slice 1c)`.

---

### Task 4: Split the reducer (mechanical)

**Files:**
- Modify/Create: `crates/ui/src/app/compare.rs` -> `compare.rs` (actions, session, setup), `compare_run.rs` (run and result events), keep `compare_duplicates.rs`; `compare_tests/` re-pointed.

No behaviour change. Move `start`, `cancel`, `moved`, `select`, `discard`, `progress`, `pair_done`, `run_done`, `rows_arrived`, `detail_arrived`, `settle`, `first_page` into `compare_run.rs` (`pub(super)` re-exports as needed); `compare.rs` keeps `CompareAction`, `on_action`, `on_event`, session and setup handling.

- [ ] **Step 1:** run `cargo test -p curlybrace-ui` and note the baseline pass count.
- [ ] **Step 2:** move the code; fix imports; no test edits except module paths.
- [ ] **Step 3:** run the same command; the pass count is identical; clippy clean; both files < 500 lines.
- [ ] **Step 4: Commit** `refactor(compare): split the compare reducer by concern (slice 1c)`.

(Task 3's minimal adaptation lives in the files created here; do Task 4 *before* Task 3 if the implementer prefers: the tasks are order-independent as long as every commit builds. Execution order: 1, 2, 4, 3, 5, 6, 7, 8, 9.)

---

### Task 5: Reducer for pairs, queued checks, multi-pair runs

**Files:**
- Create: `crates/ui/src/app/compare_pairs.rs`, tests `compare_tests/pairs.rs`, `compare_tests/multirun.rs`
- Modify: `app/compare.rs`, `app/compare_run.rs`, `app/compare_duplicates.rs`, `app/compare_tests/*` (existing single-pair tests now drive a one-entry list)

**Interfaces:**
- Consumes: Tasks 2-3 state, protocol commands/events unchanged.
- Produces new `CompareAction` variants:

```rust
AddPair,                                // from the pickers' source/target collections
AddAllSameNamed,                        // DatabasePairing::matched_to_add
AddUnmatched { side: Side, name: String, as_name: String }, // pair a no-counterpart collection with a typed name on the other side
RemovePair(EntryId),
IncludePair(EntryId, bool),
EditPair(Option<EntryId>),
SetPairOverrides(EntryId, PairOverrides),
ToggleSystemCollections(bool),
RunCheck,                               // queues the included entries
Start,                                  // runs the included entries that are not refused (button "Run")
StartPassing,                           // runs only included entries that passed
Cancel,
OpenPair(EntryId),                      // show that pair's results (needs a Completed row)
BackToOverview,
DiscardPair(EntryId),
Rerun(EntryId),
FindDuplicates(EntryId),                // per failed pair; replaces the slice 1b single-pair action
```

Rules the reducer must honour (each pinned by a test):

- **Edits invalidate:** any change to the defaults, an entry, the include flags that changes the set to run, or removing an entry calls `checks.invalidated()` and `duplicates.superseded()`. A `Checked` for a probe that is no longer in flight is dropped, and the queue is not resumed.
- **Check queue:** `RunCheck` queues every included entry with a valid spec (entries with issues are not queued; they show their issue). The reducer sends the first `Check` with `id = next_pair` (probe) and `next_pair += 1`; each `Checked` stores the verdict and sends the next. `Failed { context: "compare_check" }` marks that entry `Failed`, shows the banner once, and continues with the queue. Never more than one `Check` in flight.
- **Start:** refused unless ready, not running, no check Queued/Checking, and at least one entry would run. For each started entry: a fresh `PairId` from `next_pair`; the spec comes from `PairEntry::spec(pair_id, ..)`; `Discard` is sent for every superseded `PairId` of those entries (`RunOverview::started` returns them) and their `ResultsView`s are removed; `open_pair`/`detail` are cleared if they pointed at one of them. One `Start { run, pairs }` carries all pairs in list order. `StartPassing` filters to entries with `checks.allows_start`.
- **Run events:** `Progress` updates only the matching active row; `PairDone` settles the row, creates the pair's `ResultsView` (+ first `Rows` request) for `Completed`, none for `Cancelled`/`Failed`, and a `Failed` pair raises the banner (`compare_start`) **only the first time in a run**; `RunDone` clears `active`. A `Failed { context: "compare_start" }` while a run is active marks every still-Pending row of that run `Failed(error)` and clears `active` (the runtime refused the whole start).
- **Cancel:** sends `Cancel` once; rows still Pending get their terminal events from the runtime (no local guessing).
- **Results/detail:** `Rows`/`Detail` are accepted only for a pair that has a `ResultsView` and only when it is the exact outstanding request (existing rule) **and** the pair is `open_pair` for `Detail`. `OpenPair` sets `open_pair`, requests the current page if the view has none loaded; switching pair clears `detail`. `BackToOverview` clears `open_pair` and `detail`.
- **Discard / remove / re-run:** `DiscardPair` sends `Discard { pair }` for the row's current `PairId`, removes the `ResultsView`, marks the row `Discarded`. `RemovePair` does the same for its row, then removes the entry. `Rerun(entry)` = `only_included(entry)`, `editing(Some(entry))`, `checks.invalidated()`, discard of its old results as for `Start`, and `BackToOverview`.
- **Find duplicates:** `FindDuplicates(entry)` sends `DuplicateKeys` for that entry's spec **using the row's failed `PairId`** only if the row is `Failed(CompareDuplicateKey)`, no lookup is in flight and the entry's verdict allows it; `DuplicatesView` records which entry it belongs to and shows only under that row; a lookup for a different entry than the displayed one replaces the panel.
- **Adding:** `AddPair` needs both collections picked, neither a view (otherwise `SetupError::ViewSelected` shown via the existing setup issues, no state change); `AddAllSameNamed` adds `matched_to_add` in one state transition and clears nothing else.

- [ ] **Step 1: Write failing reducer tests** (`compare_tests/pairs.rs`, `compare_tests/multirun.rs`; use the existing `frames.rs` helpers for sessions and events):
  - Pairs: `add_pair_appends_an_entry_and_invalidates_checks`, `add_pair_refuses_a_view`, `add_all_same_named_adds_every_matched_name_once` (running it twice adds nothing), `add_unmatched_pairs_with_a_typed_name`, `remove_pair_discards_its_results_and_clears_open_pair`, `editing_a_default_invalidates_checks_but_not_overrides`.
  - Checks: `run_check_sends_one_check_at_a_time_with_distinct_probe_ids`, `checked_for_an_older_probe_is_dropped_and_does_not_resume_the_queue`, `a_failed_check_continues_with_the_next_entry`, `invalid_entries_are_not_queued`.
  - Run: `start_assigns_fresh_pair_ids_and_sends_one_start_in_list_order`, `second_run_never_reuses_a_pair_id`, `start_is_refused_with_nothing_included`, `start_is_refused_while_checking_or_running`, `start_passing_skips_refused_entries`, `a_failing_pair_between_two_good_ones_does_not_stop_them` (events: Done, Failed, Done, RunDone), `banner_is_raised_once_per_run_for_failed_pairs`, `cancel_is_sent_once_and_pending_rows_end_by_events`, `late_rows_for_a_replaced_pair_id_are_dropped`, `late_detail_after_switching_pair_is_dropped`, `session_failed_start_marks_pending_rows_failed`, `rerun_discards_the_old_pair_id_and_keeps_other_results`, `discard_pair_sends_discard_for_the_rows_current_pair_id`, `events_for_a_replaced_session_are_dropped` (existing).
  - Duplicates: `find_duplicates_is_offered_only_for_a_duplicate_key_failure_of_that_entry`, `duplicates_answer_for_another_entry_is_ignored`.
  - Existing slice 1/1b reducer tests continue to pass with a one-entry list (they are updated, not deleted; any test whose meaning disappeared is replaced by the closest multi-pair equivalent).
- [ ] **Step 2: Run** `cargo test -p curlybrace-ui app::compare`; confirm failures.
- [ ] **Step 3: Implement** `compare_pairs.rs` (pair-list actions, queue driving) and extend `compare_run.rs`/`compare_duplicates.rs`. Keep every function < 50 lines; put shared helpers (`ready`, `none`, `with_compare`) in `compare.rs` as `pub(super)`.
- [ ] **Step 4: Run** the full `cargo test -p curlybrace-ui` and clippy; fix.
- [ ] **Step 5: Commit** `feat(compare): multi-pair reducer (pair list, queued checks, per-pair runs) (slice 1c)`.

---

### Task 6: Views

**Files:**
- Create: `views/compare/pairs.rs`, `views/compare/overview.rs`
- Modify: `views/compare/setup.rs`, `views/compare/preflight.rs`, `views/compare/running.rs` (folds into `overview.rs`; delete when empty), `views/compare/results.rs`, `views/compare/duplicates.rs`, `views/compare/mod.rs`, `crates/ui/locales/en.json`

**Interfaces:**
- Consumes: Tasks 2-5. Views render state and return a `CompareAction`; they hold no decisions.
- Produces (screens):
  1. **Setup:** pickers (source/target database; collection pickers for **Add pair**), **Defaults for all pairs** (key, rules, the two filters), **Pairs** table: columns include-checkbox, source `db.collection`, target `db.collection`, key (effective), a marker when customised (icon + text), verdict/issue, buttons **Edit settings** and **Remove**. Under the table: **Add all same-named collections**, the *System collections* switch, and the list of **No counterpart** names each with a text field + **Pair with this name**.
  2. **Per-pair editor** (when `pairs.editing` is set): key, two filters, rules, each with *Use default* to clear that override; shows the entry's issue.
  3. **Preflight:** per-entry verdict rows (ok / needs index (side) / empty both / failed / not checked) and **Check**, **Run**, **Run passing** (only when some included entry was refused).
  4. **Run overview:** one row per entry that has run: status (icon + label, never colour alone), counts per category, error text for a failed pair, progress numbers for the running pair; a totals row; **Cancel** (with "cancelling" feedback); row actions **Open** (Completed only), **Discard**, **Re-run**, **Find duplicate keys** (Failed on a duplicate key only; the slice 1b panel renders under that row).
  5. **Pair results:** the slice 1 results and detail panels for `open_pair`, with a **Back to overview** button and the pair's names as a heading.
- Large lists: pair table and overview rows use `ScrollArea::show_rows` so hundreds of pairs stay cheap; per-frame work is O(visible rows) except one pass for totals, which is memoised on the overview's identity (a pure function of `RunOverview`, recomputed only when it changes: compare with the previous frame's stored value in `ui.data`).

New i18n keys (all under `compare.`, alphabetical in `en.json`; report the final list): `pairs.add_all_same_named`, `pairs.add_pair`, `pairs.column.included`, `.key`, `.source`, `.status`, `.target`, `pairs.customised`, `pairs.defaults_title`, `pairs.edit_settings`, `pairs.empty`, `pairs.no_counterpart`, `pairs.no_counterpart_hint` (states that indexes and collection options are not copied), `pairs.pair_with_name`, `pairs.remove`, `pairs.show_system`, `pairs.use_default`, `overview.cancelling`, `.column.*`, `.discard`, `.open`, `.rerun`, `.status.pending`, `.running`, `.completed`, `.cancelled`, `.failed`, `.discarded`, `.totals`, `preflight.run`, `preflight.run_passing`, `preflight.entry_*`, `results.back_to_overview`.

- [ ] **Step 1: Write failing tests:** view-model helpers needed by the views get unit tests first (status -> i18n key and icon id mapping in `viewmodel/compare/overview.rs`, `PairingRow` ordering already in Task 2); add a locale test asserting every key used in `views/compare/*.rs` exists in `en.json` (extend the existing locale test if there is one) and that `en.json` stays nested and alphabetical.
- [ ] **Step 2: Run** the tests; confirm failure.
- [ ] **Step 3: Implement** the views.
- [ ] **Step 4: Run** `cargo test -p curlybrace-ui`, clippy, `cargo fmt`.
- [ ] **Step 5: Commit** `feat(compare): multi-pair setup, preflight, overview and per-pair results views (slice 1c)`.

---

### Task 7: "Compare database to..." entry

**Files:**
- Modify: `viewmodel/sidebar.rs` (+ tests), `views/sidebar.rs`, `app/actions.rs`, `app/compare.rs`, `locales/en.json` (`sidebar.compare_database_to`), `app/compare_tests/entry.rs`

**Interfaces:** `SidebarAction::CompareDatabaseTo(String)`; `pub fn compare_database_to_action(db: &str) -> SidebarAction`; reducer `compare_database_to(state, db)` (gated like `compare_to`: not connected shows the existing banner).

- [ ] **Step 1: Write failing tests:** `compare_database_to_action_names_the_database`; reducer: `opens_a_session_and_picks_the_source_database`, `picks_the_same_database_on_the_target_when_the_connections_differ`, `leaves_the_target_database_unpicked_on_one_connection`, `not_connected_shows_the_banner`, `an_open_session_for_the_same_connections_is_reused_and_keeps_the_pair_list`.
- [ ] **Step 2-4:** run (fail), implement (database header gets a right-click menu, same pattern as `collection_menu`), run (pass).
- [ ] **Step 5: Commit** `feat(sidebar): add "Compare database to..." entry (slice 1c)`.

---

### Task 8: Snapshots and tier-1 flow

**Files:**
- Modify/Create: `crates/ui/tests/support/compare_states.rs`, `crates/ui/tests/snapshots/*.png` (regenerate the existing 17 only where the layout changed), `crates/ui/tests/local_compare_flows.rs`

- [ ] **Step 1: Snapshot states** (wgpu, non-blocking CI job): `compare_pairs_setup` (defaults + three pairs, one customised, no-counterpart list), `compare_overview_mixed` (completed, failed on duplicate key with the panel, running, pending, discarded; totals row), `compare_pair_results` (back button + pair heading). Existing single-pair snapshots are regenerated (setup now shows the pair list) and reviewed by eye.
- [ ] **Step 2: Tier-1 flow** (`#[ignore]` + env-gated, `cbit_*` databases, `cb-it-dev-*` container only): two throwaway databases with four collections: identical, different, source-only, and one with a duplicate key; drive the real runtime with the reducer: Add all same-named, Check, Run, assert the overview rows (Completed / Completed / Failed(CompareDuplicateKey) / ...), open a pair and page its rows, Find duplicate keys for the failed row, Re-run it with a different key and see it complete, cancel a second run mid-way (large collection) and assert one terminal state per row and `RunDone`.
- [ ] **Step 3:** run the tier-1 script locally (`scripts/integration-tests.sh`), the default `cargo test --workspace --locked`, and `scripts/check-no-db-in-ci.sh`.
- [ ] **Step 4: Commit** `test(compare): multi-pair snapshots and tier-1 flow (slice 1c)`.

---

### Task 9: Docs

**Files:** spec "Slice 1c" and deviations sections, `README.md` feature list, testing notes, `site/index.html` (compare section mentions comparing whole databases; keep "New in the next release" wording until a release is cut).

- [ ] **Step 1:** update the spec: mark slice 1c as built, record decisions 1, 8, 9 and 10 above, and update the "Slice 1 runtime, as built" `Rows` note (closed by never reusing a `PairId`).
- [ ] **Step 2:** README and site wording ("compare two collections or whole databases").
- [ ] **Step 3:** run `cargo test --workspace --locked`; commit `docs(compare): document multi-pair compare (slice 1c)`.

---

## Execution order and review

Tasks run in this order: 1, 2, 4, 3, 5, 6, 7, 8, 9 (every commit builds). Task reviews: Tasks 3 and 5 (state machines, stale-answer handling) use a stronger reviewer; the others use Sonnet. After Task 9: one whole-branch review on the strongest model, ONE fix wave, then open the PR with "Refs #8" (the issue stays open for slices 2 and 3).

## Known limitations carried forward

Workspace reachable only from the Browse UI; "Compare to..." and "Compare database to..." replace a session for different connections without confirmation; closing a session does not abort an in-flight `DuplicateKeys` call; no per-pair duration; `Summary`-based progress is not throttled; slice 2 still needs structured path segments in `FieldChange`.
