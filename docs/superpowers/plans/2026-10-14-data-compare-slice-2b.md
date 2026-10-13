# Data Compare, Slice 2b: Apply UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a person act on what a compare found: tick differences, build a list of pending changes (copy to the other side, delete here), review it in a confirmation dialog that names the destination, apply it with progress and cancel, and see a per-pair outcome report.

**Architecture:** The core apply engine and `CompareCommand::Apply` exist since slice 2a. This slice adds (1) one small core extension so "all rows of a category" can be applied without the UI holding a million keys, then (2) UI-only work in `curlybrace-ui`: a selection and pending-changes view-model scoped to a `PairId`, an apply state machine (idle, reviewing, running, finished) in the reducer, and the views (checkboxes, a pending panel, a modal, a result panel). Nothing writes unless the person confirms in the modal; the reducer is the only place that builds `CompareCommand::Apply`.

**Tech Stack:** Rust, egui/eframe 0.32 (`egui::Modal`), `curlybrace-core` compare apply API (slice 2a), wgpu snapshot tests (non-blocking CI job), egui_kittest accessibility tests.

**Spec:** `docs/superpowers/specs/2026-10-12-data-compare-sync-design.md` ("Apply (slice 2, designed now)", "Slice 2a as built", "Slice 1 (single pair)" selection bullet, "Slice 1c as built"). Issue #8. Slice 2a shipped in PR #36.

## Decisions made while planning (rulings; each states the cost if wrong)

1. **Category-wide actions are resolved in core.** Rows are paged (100 at a time) and only one page of keys is in the UI, so "copy all only-in-source to the target" cannot be a list of keys without paging a million keys through the UI. `CompareCommand::Apply` gains `bulk: Vec<BulkApply>` (`pair`, `category`, `kind`, `direction`); the runtime expands each into explicit `ApplyOp`s from the stored results before validation. The engine, the re-verification and the report are unchanged. Cost if wrong: one extra small core change reverted.
2. **One pending action per (pair, key).** The pending list is an ordered list of `PendingChange`s, each either `Keys(Vec<KeyTuple>)` or `Category(Category)`, with an `ApplyKind` and `Direction`. Adding an action for rows that already have one *replaces* it (adding a category item removes the explicit key items of that category; adding keys that fall under an existing category item removes them from it by splitting is NOT done: they are ignored, and the UI says so). This invariant is what makes the core's "duplicate ops are refused" rule unreachable from the UI. Cost if wrong: a refused Apply with a banner.
3. **Selection is per (PairId, Category) and lives in the view-model, not in `ResultsView`** (which is cleared by paging). It is a set of keys under a wrapper ordered by `compare_tuples`, plus an `all_in_category` flag ("every row of this tab, including pages not loaded"). Paging keeps it; a re-run, discard, remove or session close drops it (the `PairId` changes or goes away). Cost if wrong: selection lost on some navigation.
4. **Actions offered follow the row category** (mapping pinned by tests against core `kind_fits`): *only in source*: **Copy to target** (Insert, source to target), **Delete from source** (Delete, target to source); *only in target*: **Copy to source** (Insert, target to source), **Delete from target** (Delete, source to target); *different*: **Copy to target** and **Copy to source** (Replace in each direction). No other combination is offered.
5. **Typed confirmation for deletes** (spec): if any pending Delete exists, the confirm button stays disabled until the person types the *destination collection name* when all deletes target exactly one collection, the *destination database name* when they target several collections of one database, and the localized word `compare.apply.confirm_word` when they span several databases or both connections. Comparison is exact and case-sensitive on the trimmed text. The required text is shown above the input. Cost if wrong: one more dialog tweak.
6. **The dialog names destinations from a snapshot** taken when review starts (connection name, database, collection per destination, per pair), so a connection renamed or deleted meanwhile cannot change what the person confirmed; the Apply built on confirm uses the same snapshot's pair list and the pending list as reviewed (the dialog is modal, and the pending list cannot change while it is open).
7. **Apply blocks and is blocked by other work.** While an apply runs: no Start, Re-run, Remove, Discard, Find duplicates, new Check; the pending list and selection are read-only. Review/Apply is disabled while a compare run is active or a check is queued/in flight (the core also refuses, this just explains it). Run ids come from the existing `next_run` counter.
8. **After an apply, the pair is marked stale.** The stored results are not edited by core (2a ruling 9). Each affected row gets `applied: true` (a marker in the overview and in the pair header: "Changes were applied: run this pair again to see the current differences"); its pending changes and selection are cleared; its per-row actions are disabled until it is re-run (a second apply on stale results would mostly report stale). Re-running (existing flow) clears the marker.
9. **Warnings in the dialog** (computed in the view-model, shown as text, never blocking): inserts with "let the target assign new `_id`s" will not converge on a re-compare; a Replace on a pair whose `include_id` is true while its key is not `_id` keeps the destination `_id`; a Replace on a pair whose field scope is not "all" overwrites the whole destination document including fields outside the compared scope; an Insert may create a second document with the same business key when no unique index exists.
10. **Per-document actions in the detail view** add that one key to the pending list (they do not write). Field-level actions are slice 2c.
11. **Errors:** a refused `Apply` (`Failed{compare_apply}`) raises the normal failure banner and returns the state to idle (pending kept). Per-document failures come from `ApplyReport.failures` and are rendered through `errors.*` keys of their typed `CoreError` (never through `error_detail`'s raw text), with the key shown as data. `ApplyOutcomeUnknown` is explained in the result panel text.

## Global Constraints

- **Nothing writes without confirmation.** `CompareCommand::Apply` is built in exactly one reducer function (`confirm_apply`), reachable only from the `ConfirmApply` action, which is a no-op unless the state is `Reviewing` *and* the typed-confirmation check passes *and* the gates allow it. Pinned by tests that fire every other action in every state and assert no `Apply` command.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json` (nested, alphabetical, new group `compare.apply.*`, plus `compare.results.*` additions). No literal English in views, view-models or the reducer; the pseudo-locale frame test must pass for every new view. `i18n_usage_tests` rules apply (keys appear as string literals in production code; no runtime `format!` keys).
- Keys and documents are data: never logged, never in error text. Destination names shown in the dialog are the person's own connection/database/collection names.
- Immutability; files < 800 lines (aim 200-400). `app/compare.rs` is 455 lines: new reducer code goes in new files (`app/compare_apply.rs`, `app/compare_select.rs`); new view-model code in `viewmodel/compare/{selection,pending,apply}.rs`; new views in `views/compare/{select,pending,apply}.rs`. Functions < 50 lines; nesting <= 4.
- Default `cargo test --workspace --locked` stays DB-, docker- and GPU-free. Tier-1 tests are `#[ignore]` + env-gated, use only `cbit_<id>` databases and only a `cb-it-dev-*` container the test run starts itself (e.g. `cb-it-dev-2b`, removed afterwards), never the user's `local7979`/`local265` or `127.0.0.1:27017`. GitHub CI must never need MongoDB (`scripts/check-no-db-in-ci.sh` stays green).
- Commits: conventional format ending with `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`. Before every push: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`.
- Sub-agents default to Sonnet. Tasks 1, 4 and the final review use Opus (they build the write request and the state machine that gates it).

## Review Focus

1. **A write the person did not confirm.** Any path that sends `Apply` without `Reviewing` + passing confirmation (including stale actions after a re-run, session close, or a double-click on Confirm). Pinned in Tasks 4 and 7.
2. **Wrong destination or wrong kind.** Direction/kind mapping per category; the dialog naming the connection/database/collection that will actually be written (source vs target per direction); counts in the dialog equal the ops sent. Pinned in Tasks 2, 3, 4.
3. **Stale selection and pending changes.** Re-run, discard, remove, session close and replace, a late `Rows` page, a late `ApplyProgress`/`ApplyPairDone` from an earlier run id must not change visible state or select the wrong rows. Pinned in Tasks 3, 4.
4. **Selecting a million rows.** "All in category" must stay O(1) in memory and per frame; per-frame work in the results table must not scan the selection set (use membership lookups for the 100 visible rows only). Pinned in Tasks 2 and 5.
5. **Mixed outcomes shown honestly.** Applied/stale/failed/not-run counts add up to the ops sent; failures capped at 100 with a "more" note; `ApplyOutcomeUnknown` and a cancelled apply are explained, not shown as success. Pinned in Tasks 4 and 6.

## File Structure

```
crates/core/src/compare/protocol.rs          Apply gains `bulk: Vec<BulkApply>`; BulkApply type          (Task 1)
crates/core/src/runtime/compare/apply.rs     expand bulk items from stored results before validation      (Task 1)
crates/ui/src/viewmodel/compare/
  selection.rs         NEW  OrdKey, Selection (per PairId+Category), all_in_category, toggles            (Task 2)
  pending.rs           NEW  PendingChange, PendingList (replace-on-conflict), action mapping, counts      (Task 2)
  apply.rs             NEW  ApplyUi {Idle, Reviewing, Running, Finished}, Review snapshot, confirm check,
                            warnings, progress/report folding, stale marker                              (Task 3)
  mod.rs               CompareState: selection, pending, apply                                           (Tasks 2-3)
  overview.rs / rows.rs  applied marker, gates for Re-run/Remove while applying                          (Task 3)
crates/ui/src/app/
  compare_select.rs    NEW  selection and pending actions                                                (Task 4)
  compare_apply.rs     NEW  review, confirm, cancel, apply events                                        (Task 4)
  compare.rs           route new actions/events, drop the slice-2a no-op arms                            (Task 4)
crates/ui/src/views/compare/
  results.rs / select.rs   checkbox column + selection bar + per-category actions                         (Task 5)
  detail.rs            per-document "add to pending" buttons                                             (Task 5)
  pending.rs           NEW  pending panel                                                                (Task 6)
  apply.rs             NEW  review modal, progress, result panel                                         (Task 6)
crates/ui/locales/en.json                      compare.apply.*, compare.results.* additions              (Tasks 5-6)
crates/ui/tests/{snapshots,accessibility}.rs, support/compare_apply_states.rs  NEW                       (Task 7)
crates/ui/tests/local_compare_apply_flows.rs   NEW tier-1                                                (Task 8)
README.md, site/index.html, spec, notes/testing.md                                                       (Task 8)
```

---

### Task 1: Category-wide apply items in core (Opus)

**Files:**
- Modify: `crates/core/src/compare/protocol.rs`, `crates/core/src/runtime/compare/apply.rs`, the places the compiler reports (`CompareCommand::Apply { .. }` constructions in core tests and the UI)
- Test: `crates/core/src/runtime/compare/apply_tests.rs`

**Interfaces:**
- Produces:
  ```rust
  /// "Every row of this category of this pair" (resolved by the runtime from the stored results).
  #[derive(Clone, Debug, PartialEq)]
  pub struct BulkApply { pub pair: PairId, pub category: Category, pub kind: ApplyKind, pub direction: Direction }
  // CompareCommand::Apply { session, run, ops: Vec<ApplyOp>, bulk: Vec<BulkApply>, id_policy }
  ```
  Expansion rule (in `start_apply`, before the duplicate/unknown-pair checks): for each `BulkApply`, the pair must have stored results (else `Unsupported("no results for this pair")`); `kind_fits(kind, direction, has_source, has_target)` must hold for the category's rows (`OnlyInSource`: has_source only; `OnlyInTarget`: has_target only; `Different`: both) else `Unsupported("operation does not fit this category")`; each stored entry of that category becomes one `ApplyOp` appended after the explicit `ops`, in stored order. Two `BulkApply`s for the same (pair, category), or a bulk expanding to a key that also appears in `ops`, make the existing duplicate-op check refuse the request. An empty `ops` is allowed when `bulk` yields at least one op; zero ops in total is refused as today.

- [ ] **Step 1: Failing tests** (reuse the harness of `apply_tests.rs`):
  - `a_bulk_item_applies_every_row_of_the_category` (50 only-in-source rows, one `BulkApply{OnlyInSource, Insert, SourceToTarget}` -> 50 inserts, `applied == 50`).
  - `a_bulk_item_that_does_not_fit_the_category_is_refused_and_writes_nothing` (Replace on OnlyInSource; Delete SourceToTarget on OnlyInSource).
  - `a_bulk_item_for_an_unknown_pair_is_refused`.
  - `a_bulk_item_and_an_explicit_op_on_the_same_key_are_refused_as_duplicates`.
  - `two_bulk_items_for_one_category_are_refused`.
  - `bulk_and_explicit_ops_for_different_categories_apply_together` (Insert bulk + one explicit Replace).
  - `an_empty_category_expands_to_nothing_and_an_otherwise_empty_request_is_refused`.
  - `bulk_expansion_keeps_the_view_and_timeseries_destination_rules` (View destination refused for a bulk Insert).
- [ ] **Step 2: Run** — `cargo test -p curlybrace-core runtime::compare::apply -- --locked` → FAIL (field missing).
- [ ] **Step 3: Implement.** Page through the stored `ResultSet` under the pairs lock (`ResultSet::page(category, offset, limit)`), copying keys out before any await; keep every function < 50 lines (`expand_bulk`, `bulk_to_ops`). Update the spec's protocol note later in Task 8.
- [ ] **Step 4: Run** — `cargo test --workspace --locked` → PASS; clippy clean.
- [ ] **Step 5: Commit** — `feat(runtime): apply whole categories of a pair without listing their keys`

---

### Task 2: Selection and pending-changes view-models

**Files:**
- Create: `crates/ui/src/viewmodel/compare/selection.rs`, `pending.rs` (+ `selection_tests.rs`, `pending_tests.rs` wired with `#[path]` like the neighbours); modify `viewmodel/compare/mod.rs`

**Interfaces:**
- Produces:
  ```rust
  /// A key tuple ordered by core's `compare_tuples` (usable in a BTreeSet).
  #[derive(Clone, Debug, PartialEq)] pub struct OrdKey(pub KeyTuple);   // impl Eq + Ord

  #[derive(Clone, Debug, Default, PartialEq)]
  pub struct CategorySelection { pub keys: BTreeSet<OrdKey>, pub all: bool }
  #[derive(Clone, Debug, Default, PartialEq)]
  pub struct Selection { by: BTreeMap<(PairId, CatKey), CategorySelection> }   // CatKey = a small Ord copy of core Category
  impl Selection {
      pub fn toggled(&self, pair: PairId, cat: Category, key: &KeyTuple) -> Self;       // clears `all` first (all -> every loaded row except this one is NOT representable: unticking under `all` turns `all` off and keeps the page's other rows selected)
      pub fn page_selected(&self, pair: PairId, cat: Category, keys: &[KeyTuple]) -> Self;
      pub fn all_selected(&self, pair: PairId, cat: Category) -> Self;                  // sets `all`, clears keys
      pub fn cleared(&self, pair: PairId, cat: Category) -> Self;
      pub fn without_pair(&self, pair: PairId) -> Self;
      pub fn is_selected(&self, pair: PairId, cat: Category, key: &KeyTuple) -> bool;   // all || keys.contains
      pub fn count(&self, pair: PairId, cat: Category, total_in_category: usize) -> usize; // all -> total
      pub fn is_empty_for(&self, pair: PairId, cat: Category) -> bool;
  }

  #[derive(Clone, Copy, Debug, PartialEq, Eq)]
  pub enum RowAction { CopyToTarget, CopyToSource, DeleteFromSource, DeleteFromTarget }
  impl RowAction {
      pub fn kind(self) -> ApplyKind; pub fn direction(self) -> Direction;
      pub fn i18n_key(self) -> &'static str;            // literals under compare.apply.action.*
      pub fn offered_for(cat: Category) -> &'static [RowAction];   // ruling 4
  }
  #[derive(Clone, Debug, PartialEq)]
  pub enum Scope { Keys(Vec<KeyTuple>), Category(Category) }
  #[derive(Clone, Debug, PartialEq)]
  pub struct PendingChange { pub pair: PairId, pub category: Category, pub action: RowAction, pub scope: Scope }
  #[derive(Clone, Debug, Default, PartialEq)]
  pub struct PendingList { changes: Vec<PendingChange> }
  impl PendingList {
      pub fn with_action(&self, pair: PairId, cat: Category, action: RowAction, scope: Scope) -> (Self, usize /*ignored keys covered by an existing category item*/);
      pub fn without(&self, index: usize) -> Self; pub fn without_pair(&self, pair: PairId) -> Self; pub fn cleared(&self) -> Self;
      pub fn items(&self) -> &[PendingChange]; pub fn is_empty(&self) -> bool;
      pub fn op_count(&self, totals: &dyn Fn(PairId, Category) -> usize) -> u64;
      pub fn to_command_parts(&self) -> (Vec<ApplyOp>, Vec<BulkApply>);   // explicit ops + bulk items
  }
  ```
  Replace-on-conflict (ruling 2): a `Category` scope removes every `Keys` item of the same (pair, category) and any other `Category` item of it; `Keys` scope removes its keys from other `Keys` items of the same (pair, category) (dropping emptied items) and is ignored (counted) when a `Category` item covers the category.

- [ ] **Step 1: Failing tests** (pure):
  - Selection: toggle on/off, page select, `all` then `count` returns the category total without enumerating, unticking under `all` turns `all` off and keeps the other loaded rows, `cleared`, `without_pair`, per-(pair, category) independence, `OrdKey` equality for `Int32(1)` and `Int64(1)` (same key) and ordering consistent with `compare_tuples`, 100k-key selection `is_selected` stays fast (no scan: assert via `BTreeSet` only, e.g. a loop of 100 lookups completes instantly; no timing asserts).
  - `RowAction::offered_for` full table (ruling 4) and `kind()`/`direction()` agree with core `kind_fits` for every category row shape (loop over the three row shapes and every action: offered iff `kind_fits`).
  - Pending: add keys, replace-on-conflict in all four directions listed above, category item swallows keys (returns ignored count), `without`, `without_pair`, `to_command_parts` (explicit ops carry the right kind/direction; category items become `BulkApply` with the same kind/direction), `op_count` uses totals for categories and lengths for key lists, stable order of items.
- [ ] **Step 2: Run** — `cargo test -p curlybrace-ui viewmodel::compare::{selection,pending} -- --locked` → FAIL.
- [ ] **Step 3: Implement.** `CompareState` gains `selection: Selection`, `pending: PendingList` (Default); `session_dropped()` resets them; `without_results(&[PairId])` also calls `selection.without_pair` and `pending.without_pair`.
- [ ] **Step 4: Run** → PASS; clippy clean.
- [ ] **Step 5: Commit** — `feat(ui): selection and pending-changes view-models for apply`

---

### Task 3: Apply state (review, confirmation, progress, report)

**Files:**
- Create: `crates/ui/src/viewmodel/compare/apply.rs` (+ `apply_tests.rs`); modify `viewmodel/compare/{mod,overview,rows,gates}.rs`

**Interfaces:**
- Consumes: Task 2 types; core `ApplyReport`, `OpFailure`, `IdPolicy`, `Direction`.
- Produces:
  ```rust
  #[derive(Clone, Debug, PartialEq)]
  pub struct Destination { pub connection: String, pub database: String, pub collection: String }   // snapshot of names
  #[derive(Clone, Debug, PartialEq)]
  pub struct ReviewPair { pub pair: PairId, pub entry: EntryId, pub destinations: Vec<(RowAction, Destination)>, pub counts: KindCounts }
  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)] pub struct KindCounts { pub insert: u64, pub replace: u64, pub delete: u64 }
  #[derive(Clone, Debug, PartialEq)]
  pub struct Review { pub pairs: Vec<ReviewPair>, pub id_policy: IdPolicy, pub confirm_text: String, pub required: Option<RequiredText>, pub warnings: Vec<Warning> }
  pub enum RequiredText { Literal(String) /*collection or database name*/, Word /*localized word*/ }
  pub enum Warning { GeneratedIds, KeepsDestinationId, WholeDocumentOverwrite, MaybeDuplicateBusinessKey }   // each has i18n_key()
  pub enum ApplyUi {
      Idle,
      Reviewing(Review),
      Running { run: RunId, total: u64, rows: Vec<ApplyRow>, cancel_requested: bool },
      Finished { reports: Vec<(PairId, ApplyReport)>, cancelled: bool, ops_sent: u64 },
  }
  pub struct ApplyRow { pub pair: PairId, pub entry: EntryId, pub done: u64, pub total: u64, pub report: Option<ApplyReport> }
  impl Review {
      pub fn confirm_enabled(&self, word: &str) -> bool;    // required satisfied (exact, trimmed) 
      pub fn required_text<'a>(&'a self, word: &'a str) -> Option<&'a str>;
  }
  impl ApplyUi { pub fn review_of(...) ; pub fn progressed(self, run, pair, done, total) -> Self; pub fn pair_done(self, run, pair, report) -> Self; pub fn apply_done(self, run) -> Self; pub fn is_busy(&self) -> bool; }
  ```
  Also: `RunRow` (overview) gains `applied: bool` (set when a pair finishes an apply that wrote anything or failed; cleared by a new run of the entry); view-model gates: `can_rerun`/`can_remove`/`can_discard` false while `apply.is_busy()`; new `gates.can_review` = ready session, pending non-empty, every pending pair Completed and not `applied`, no compare run active, no check queued/in flight, not busy.
  `build_review(state_parts) -> Review` computes: per pending item the destination from the direction (`SourceToTarget` writes the *target* connection/namespace of the entry, `TargetToSource` the *source*), `KindCounts` from keys lengths and category totals, `required` per ruling 5 (collections/databases of all Delete items; spanning connections or databases -> `Word`), warnings per ruling 9 (from the entries' settings: `include_id`, key, field scope; `id_policy` is part of `Review` and `GeneratedIds` appears only when `id_policy == Generate` and there is an Insert).

- [ ] **Step 1: Failing tests** (pure):
  - Destination mapping for both directions across two pairs with different source/target connections and namespaces; names are the snapshot (rename the connection in the input afterwards: review unchanged).
  - `KindCounts` equal the ops that `to_command_parts` + category totals would send (property over random pending lists using the Task 2 builder).
  - `required` table: one delete collection -> that collection name; two collections same db -> the db name; two dbs -> `Word`; deletes across both connections -> `Word`; no deletes -> `None`.
  - `confirm_enabled`: exact match true; wrong case false; trimmed whitespace true; empty false; `None` -> true.
  - Warnings table per ruling 9 incl. no warning for a plain `_id`-keyed full-scope replace.
  - State machine: `Running` folds progress for the right `(run, pair)` only (a stale run id or unknown pair is ignored), `pair_done` stores the report, `apply_done` -> `Finished` with totals; a cancelled flag when `cancel_requested`; counts `applied+stale+not_planned+failed+not_run == total` is shown by `ops_sent`.
  - Gates: `can_review` false in each blocking situation (no pending, pair `applied`, pair not completed, run active, check queued, apply busy); `can_rerun`/`can_remove` false while busy.
- [ ] **Step 2: Run** — FAIL; **Step 3: Implement**; **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(ui): apply review, confirmation and progress state`

---

### Task 4: Reducer: selection, pending, review, confirm, events (Opus)

**Files:**
- Create: `crates/ui/src/app/compare_select.rs`, `compare_apply.rs` (+ tests under `app/compare_tests/apply.rs`, `select.rs`); modify `app/compare.rs` (route only), `app/compare_run.rs` if it owns `Rows` handling, `app/banner.rs` only if a test demands

**Interfaces:**
- Produces new `CompareAction` variants: `ToggleRow(KeyTuple)`, `SelectPage`, `SelectAllInCategory`, `ClearSelection`, `AddToPending(RowAction)`, `AddKeyToPending(KeyTuple, RowAction)` (from the detail), `RemovePending(usize)`, `ClearPending`, `ReviewApply`, `SetConfirmText(String)`, `SetIdPolicy(IdPolicy)`, `ConfirmApply`, `CancelReview`, `CancelApply`, `DismissApplyResult`.
- Behaviour: selection actions apply to `open_pair`/`results.category`; `AddToPending` takes the selection of the open (pair, category) (keys or `all` -> `Scope::Category`), refuses if empty or the action is not `offered_for` the category or the pair is `applied`; clears that selection afterwards. `ReviewApply` builds the `Review` snapshot (needs the connection names: take them from `AppState::connection(id)` at this moment) when `can_review`; otherwise no-op. `ConfirmApply` is the ONLY place that creates `CompareCommand::Apply { session, run: next_run, ops, bulk, id_policy }`: preconditions re-checked (state is `Reviewing`, `confirm_enabled`, `can_review` gates still hold); on success state -> `Running` (run = next_run, `next_run += 1`, total = review op count), pending kept until `ApplyDone`. Events: `ApplyProgress`/`ApplyPairDone`/`ApplyDone` fold into `ApplyUi` only when `run` matches the running run and the session is the current one (late/stale events dropped); on `ApplyPairDone`/`ApplyDone` the affected rows get `applied = true`, their selection and pending are cleared and `ApplyDone` -> `Finished`. `Failed { context: "compare_apply" }` -> failure banner + `ApplyUi::Idle`, pending kept. `CancelApply` sends `CompareCommand::Cancel { session, run }` once and sets `cancel_requested`. Session close/replace/`session_dropped` -> `Idle`. Re-run/Remove/Discard for a busy apply are refused (gates already). `Start`/`RunCheck` refused while busy.

- [ ] **Step 1: Failing tests** (reducer, with the existing helpers `act`, `ev`, `finished_with_results`, `with_rows`; add a fixture `with_pending(...)` in `compare_tests/apply.rs`):
  - **No unconfirmed write** (Review Focus 1): for every `CompareAction` variant in every `ApplyUi` state, assert no `Command::Compare(CompareCommand::Apply{..})` except `ConfirmApply` from `Reviewing` with a passing confirmation; double `ConfirmApply` sends exactly one; `ConfirmApply` after the pair was re-run/discarded/session closed sends none; wrong typed text sends none; `SetConfirmText` alone never sends.
  - Selection: toggle/select page/all in category/clear; selection survives paging (`Page`) and category switch; dropped on re-run/discard/remove/session close; a late `Rows` for an old pair does not alter selection.
  - Pending: `AddToPending` per category with the offered actions only (others no-op), replace-on-conflict through the reducer, `all` becomes a category item, selection cleared after add, refused on `applied` pairs.
  - Review: snapshot names, required text, warnings; `ReviewApply` no-op when gates fail (run active, check in flight, pair not completed).
  - Confirm: command content equals the pending list (`ops` + `bulk`, `id_policy`, fresh `run`), state `Running`; `Cancel` sends one `Cancel{run}`.
  - Events: progress/pair-done/apply-done fold; stale run id ignored; session mismatch ignored; `ApplyDone` -> `Finished`, rows `applied`, pending cleared, selection cleared; `Failed{compare_apply}` -> banner, `Idle`, pending kept; late events after `DismissApplyResult` ignored.
  - Busy gates: `Start`, `RunCheck`, `Rerun`, `RemovePair`, `DiscardPair`, `FindDuplicates` refused while `Running`.
  - `compare_tests/frames.rs` pseudo-locale test is updated in Task 6/7, not here.
- [ ] **Step 2: Run** — `cargo test -p curlybrace-ui app::compare_tests -- --locked` → FAIL; **Step 3: Implement** (route in `app/compare.rs`, remove the slice-2a no-op arms and their comment, add exhaustive routing); **Step 4: Run** → PASS (whole workspace).
- [ ] **Step 5: Commit** — `feat(ui): reducer for selection, pending changes and confirmed apply`

---

### Task 5: Results table: checkboxes, selection bar, per-document actions

**Files:**
- Create: `crates/ui/src/views/compare/select.rs`; modify `views/compare/results.rs`, `views/compare/detail.rs`, `views/compare/mod.rs`, `locales/en.json`

**Interfaces:**
- Consumes: `Selection`, `RowAction::offered_for`, gates (`can_review` family), `CompareAction` from Task 4.
- Behaviour: each results row gets a leading checkbox bound to `selection.is_selected` (membership lookup for the visible rows only); a header checkbox selects the loaded page; a selection bar above the table shows "N selected" (`count`), **Select all N in this tab** (shows the category total), **Clear**, and one button per `RowAction::offered_for(category)` (disabled with `on_disabled_hover_text` reason when nothing is selected, the pair is `applied`, a run/check is active or an apply is busy). The detail view's title row gets the same actions for its one key (`AddKeyToPending`), disabled with reasons likewise. A pending badge on the pair header shows how many changes are pending for this pair. The pair header shows the "changes were applied, run again" note when `applied`.

- [ ] **Step 1: Failing tests:** kittest tests in `tests/accessibility.rs` (`mod compare_apply_selection`): checkbox per row has an accessible label (key text), toggling emits `ToggleRow`, header checkbox emits `SelectPage`, **Select all** button emits `SelectAllInCategory` and shows the total, action buttons present exactly per category (table from ruling 4) and disabled states with their hover reason, detail actions emit `AddKeyToPending`; unit test (view-model side) that building the 100 visible rows never iterates the whole selection (use a 1,000,000-key `all` flag selection: instant).
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement** + en.json keys (`compare.results.select_all`, `…select_page`, `…selected`, `…clear`, `compare.apply.action.copy_to_target|copy_to_source|delete_from_source|delete_from_target`, `compare.apply.reason.*` for disabled reasons, `compare.apply.applied_note`, `compare.apply.pending_badge`); **Step 4: Run** → PASS, `i18n_usage_tests` green.
- [ ] **Step 5: Commit** — `feat(ui): select differences and queue copy and delete actions`

---

### Task 6: Pending panel, review dialog, progress and result panel

**Files:**
- Create: `crates/ui/src/views/compare/pending.rs`, `views/compare/apply.rs`; modify `views/compare/{mod,overview}.rs`, `locales/en.json`

**Interfaces / behaviour:**
- **Pending panel** (in the overview and in the pair view): grouped by pair (entry labels), each item "Copy N documents to <destination name>" / "Delete N documents from <…>" with a remove button, totals, **Review changes…** (enabled by `can_review`, reason shown when not) and **Clear**. Counts for category items use the stored `Counts`.
- **Review modal** (`egui::Modal`, id `compare_apply_review`; state lives in `ApplyUi::Reviewing`, actions only through `CompareAction`): title, per-pair table (pair labels, per kind counts, destination "connection / database.collection"), the totals, warnings (each with its key), an id-policy toggle shown only when there are inserts ("Keep the original `_id`s" default / "Let the destination assign new `_id`s"), the typed-confirmation row when `required` is `Some` (label shows the required text via `t_with`, `TextEdit` bound to `confirm_text`, emits `SetConfirmText`), **Apply** (`confirm_enabled`) and **Cancel**. Escape / `modal.should_close()` emits `CancelReview`. No keyboard shortcut triggers Apply (Enter does not confirm).
- **Progress / result:** while `Running`: per-pair progress bars from `done/total`, a spinner, **Cancel** (`CancelApply`, disabled once requested with "cancelling" label). `Finished`: summary line (applied / skipped because changed since the compare / not applicable / failed / not run) with icons *and* labels, per-pair rows, up to 100 failures listed as key (data) + error text through `i18n.error(&CoreError)` for the typed variants (never `error_detail`), a "more failures not shown" note when `more_failures`, an explanation line for `ApplyOutcomeUnknown` and for a cancelled apply, a prominent "Run these pairs again to see what is left" note, and **Dismiss**.
- Everything uses literal i18n keys under `compare.apply.*` (title, columns, kinds, warnings, required-text prompt, confirm word, outcome labels, notes).

- [ ] **Step 1: Failing tests:** kittest (`tests/accessibility.rs`, `mod compare_apply_dialog`): modal shows destination names and counts from a fixture; Apply disabled until the exact required text is typed (collection, database and word variants); typing emits `SetConfirmText`; Apply emits `ConfirmApply` once; Cancel/Escape emit `CancelReview`; id-policy toggle only with inserts; warnings rendered; progress shows `done/total` and Cancel emits `CancelApply`; finished panel shows counts that add up, failures with key text and capped note, `ApplyOutcomeUnknown` explanation; pseudo-locale frame test (`compare_tests/frames.rs`, add a new file `frames_apply.rs` because `frames.rs` is already 644 lines) renders the pending panel, the modal, progress and the result with every string translated (`assert_translated`).
- [ ] **Step 2: Run** → FAIL; **Step 3: Implement**; **Step 4: Run** → PASS (+ `i18n_usage_tests`).
- [ ] **Step 5: Commit** — `feat(ui): review dialog, progress and result panels for apply`

---

### Task 7: Snapshots and accessibility pass

**Files:**
- Create: `crates/ui/tests/support/compare_apply_states.rs`; modify `crates/ui/tests/snapshots.rs`, `tests/accessibility.rs` (only if a gap appears); new PNGs under `tests/snapshots/`

- [ ] **Step 1:** state builders (script the reducer like `compare_multi_states.rs`): `selection_in_results` (rows ticked, bar visible), `pending_panel` (two pairs, one with category item), `review_with_delete` (typed-name prompt, warnings), `apply_running` (two pairs, partial progress), `apply_finished_mixed` (applied/stale/failed with failure list and the re-run note), `pair_marked_applied`.
- [ ] **Step 2:** add `shot!` entries (GPU-gated `#[ignore]`, `CURLYBRACE_SNAPSHOTS=1`); generate baselines locally; **eyeball every PNG** (truncation at narrow widths, disabled states visible, icons paired with labels) and fix layout issues found before committing the baselines.
- [ ] **Step 3:** run the full local snapshot tier and the default suite; record that the `ui-snapshots` CI job is non-blocking and may fail on CI's GPU path (it already does on `main`).
- [ ] **Step 4: Commit** — `test(ui): snapshots for the apply flow`

---

### Task 8: Tier-1 flow, docs, landing page

**Files:**
- Create: `crates/ui/tests/local_compare_apply_flows.rs`; modify `crates/ui/tests/support/compare.rs` (helpers), the spec, `docs/superpowers/notes/testing.md`, `README.md`, `site/index.html`

- [ ] **Step 1: Tier-1 tests** (`#[ignore]`, env-gated like `local_compare_flows.rs`; own container `cb-it-dev-2b` on port 27120; only `cbit_<id>` databases; helpers in `support/compare.rs` use gestures only: tick rows, click an action, open Review, type the name, click Apply, wait for Finished):
  - `copy_whole_collection_to_an_empty_target_then_rerun_shows_no_differences` (Select all in tab + Copy to target; bulk path through the real runtime).
  - `delete_extras_requires_the_typed_name_and_removes_only_selected_rows`.
  - `replace_changed_documents_and_converge`.
  - `a_document_changed_after_the_compare_is_reported_stale_and_left_alone`.
  - `a_unique_index_collision_is_listed_as_a_failure_and_neighbours_apply`.
  - `cancel_a_large_apply_reports_not_run_and_marks_the_pair_applied`.
  - `two_pairs_in_one_apply_report_separately`.
- [ ] **Step 2: Docs.** Spec: "Slice 2b as built" (selection/pending model, rulings 1-11, the `bulk` protocol addition, what the UI shows, limits: no field-level actions yet, selection of "all" is by category not by filter, results are stale after apply), update the Apply section's "Pending changes" and the protocol listing; notes/testing.md for the new tier-1 file. README and `site/index.html`: compare can now copy and delete differences (with confirmation, per-document outcomes); keep "Still to build: field-level changes and exporting reports"; the "New in the next release" wording stays until a release is cut (then remove temporal phrases, as noted in the spec).
- [ ] **Step 3: Verify** — `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`, `scripts/check-no-db-in-ci.sh`, tier-1 files against `cb-it-dev-2b`; remove the container; `docker ps` shows only `local7979` and `local265`.
- [ ] **Step 4: Commit** — `test(ui): tier-1 apply flows; docs and landing page for slice 2b`

---

## Self-review

- **Spec coverage (Apply section):** pending changes built from selection + action (Tasks 2, 4, 5), per-pair selection kept when switching pairs (Task 2/4), confirmation naming target connection/database/collection with the typed name for deletes (Tasks 3, 6), counts grouped by pair (Task 6), id policy choice (Tasks 3, 6), progress and cancel (Tasks 4, 6), per-document outcomes and failures (Task 6), across pairs (Tasks 3, 4), re-run converges / stale marker (Tasks 3, 4, 6). **Deferred to 2c:** field-level `$set`/`$unset`. **Deferred to slice 3:** export. **New core piece:** `bulk` items (Task 1).
- **Placeholders:** none; where a test says "reuse helper X" the helper exists (named in the UI survey: `act`, `ev`, `finished_with_results`, `with_rows`, `compare_multi_states.rs`).
- **Type consistency:** `RowAction` (Task 2) drives `PendingChange` (2), `Review` (3), actions (4) and buttons (5); `BulkApply` (Task 1) is produced only by `PendingList::to_command_parts` (Task 2); `ApplyUi` (3) is mutated only by the reducer (4) and read by the views (5, 6).
