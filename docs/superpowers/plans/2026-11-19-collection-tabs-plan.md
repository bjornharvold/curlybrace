# Collection tabs, split view and session restore (#159), Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development. Steps use checkbox syntax.

**Goal:** Close #159 in four PRs:

1. a per-tab state model with no behaviour change;
2. tabs;
3. split view;
4. session restore.

**Architecture:** A second level of the existing foreground swap.

- **Sessions do not change.** `SessionView`, `take_view`, `restore_view`, `activated` and `with_session` stay as they are. Each open session has one *current tab*, whose fields live where they live today. Its other tabs are parked in `AppState.parked_tabs` as boxed `TabView`s.
- **Swapping.** `take_tab` and `restore_tab` swap the tab-level fields of the foreground. `with_tab` composes `with_session` with a tab swap.
- **Routing.** Answers find their tab by the number they echo: `owner_of(state, session, &Event)` returns the tab whose own state awaits it.
  - Numbers never repeat app-wide: every tab feature draws them from one process-wide source.
  - The core echoes a number on `Find`, `ExplainFind` and `EstimatedCount`.
  - A miss falls back to today's path, the session's current tab.
- **Restore** is `workspace.json` (v1, 0600, `atomic_write`, backup and guard). Its tabs materialise dormant when the person connects.

**Tech stack:** Rust 1.87 (no let-chains), egui/eframe 0.32, egui_kittest, `serde_json`, `mongodb` 3.x. No new dependency.

**Spec:** `docs/superpowers/specs/2026-11-19-collection-tabs-design.md`.

**Binding earlier designs:**
- `2026-10-14-multiple-sessions-design.md`: the swap; `Results` matched by session, ns and query;
- `2026-10-23-read-only-connections-design.md`;
- `2026-11-13-aggregation-builder-design.md`: the one-preview-in-flight rule, now per tab;
- `2026-11-18-browse-header-design.md`: 28 pt controls, 4 pt `ROW_GAP`, the 800x500 and 1000x700 budgets.

**Reuse (do not duplicate):**
- **Sessions:** `AppState::{activated, with_session, session_dropped, failed, opened_with, aggregate_for_reopen, session_badge, open_label}`; `SessionView`.
- **Aggregate:** `AggregateState::{on_collection_changed, on_connection_reset}`; `viewmodel::aggregate_flight`.
- **Counters already app-wide:** `viewmodel::aggregate_counts::NEXT_REQUEST`; `guard_request`; `bulk_requests`. Hint numbers keep their `HINT_REQUEST_BASE` (`viewmodel/aggregate_hint_list.rs`).
- **Persistence:** `persistence::{atomic_write, back_up, PersistGuard, PersistError, Recovery}`, `settings_file::{load_read, json_object}`, `AppState::with_recovery_notice`, and the `pipelines_store` handling of a file from a newer version.
- **Views and widgets:** `theme::{tabs_inline, CONTROL_HEIGHT, ROW_GAP}`, `views::connection_badge`, `views::context_menu::row_context_menu`, `views::sidebar_copy::click_mode`.
- **Tests:** `tests/support::{shots, actions, fixture, wait}`, `layout_audit_800.rs`'s `page_sized`, and the `connections_secrets_audit.rs` approach.

## Global Constraints

- **Toolchain:** `cargo +1.87.0` for everything. No let-chains (`if let .. && let ..` does not compile on 1.87). CI pins 1.87.
- **File sizes:**
  - Files I create: 200-400 lines typical, never over 400.
  - Over-800 files must not grow. `crates/core/src/runtime/mod.rs` (1064) and `crates/core/src/runtime/handlers.rs` (801) end each commit at net 0 or less: move a self-contained item out to compensate. `crates/ui/src/app/bulk_actions.rs` (832) gets nothing. `crates/ui/src/app/mod.rs` (1021) gets only `mod` lines plus at most the two one-line calls named in T19.
  - Near the cap: `crates/ui/tests/snapshots.rs` (797) gets exactly one `#[path] mod` line; new shots go in `crates/ui/tests/snapshot_suites/`.
  - Over-400 files get only mechanical edits; new code goes to a new file: `reducer.rs` 715, `session.rs` 452, `state.rs` 505, `panels.rs` 626, `documents.rs` 610, `views/sidebar.rs` 440.
  - `wc -l` every touched file before each commit.
- **Immutable style:** ops return new values (`Self { .., ..self }`). The swaps use local `std::mem::take` only, as `take_view` does. The one exception is the process-wide request number source (T3), which follows the existing `aggregate_counts::NEXT_REQUEST` precedent.
- **Debug stack:** `AppState` moves by value; the deepest event path is near the debug-build stack limit. `TabView` is boxed in `parked_tabs`. `with_tab`, `take_tab`, `restore_tab` and `owner_of` are `#[inline(never)]`. No new arm goes into `reducer::on_event`'s or `on_session_event`'s match; routing is done before them, in their own frames.
- **TDD:** failing test, see it red, implement, see it green.
- **Real objects over mocks:** real `Page`, `Query` and `AggregateState` values; `InMemoryStore` for kittests; the real runtime for e2e.
- **Coverage:** at least 80% of new code (`cargo llvm-cov` is installed).
- **i18n:**
  - Every visible string, tooltip and accessible name is a key, written as a literal (`crates/ui/src/i18n_usage_tests.rs`).
  - `crates/ui/locales/en.json` is nested and alphabetical at every level, and unused keys fail. New top-level `tabs` goes between `sidebar` and `tools`.
  - A key is added in the task whose source first names it.
- **Privacy:** never log, `Debug`-print into a log line, or put into an error a query text, a stage text, a document or a namespace's data. File errors log the `PersistError` kind only.
- **Real servers:** NEVER connect to any MongoDB already running on the machine. Real-server tests run only against a throwaway `/opt/homebrew/bin/mongod` on a spare localhost port with a temp dbpath, stopped by its own pid (never `pkill mongod`) and deleted after:
  ```bash
  PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')
  DBPATH=$(mktemp -d /private/tmp/claude-501/cbit-tabs.XXXXXX)
  /opt/homebrew/bin/mongod --dbpath "$DBPATH" --bind_ip 127.0.0.1 --port "$PORT" --fork --logpath "$DBPATH/log" --pidfilepath "$DBPATH/pid"
  CURLYBRACE_IT=1 CURLYBRACE_IT_MONGODB_URI="mongodb://127.0.0.1:$PORT" cargo +1.87.0 test -p curlybrace-ui --test <file> --locked -- --ignored
  kill "$(cat "$DBPATH/pid")"; rm -rf "$DBPATH"
  ```
  New real-server test files are added by hand to `scripts/integration-tests.sh`.
- **Snapshots:** regenerate only with `CURLYBRACE_SNAPSHOTS=1` (and `UPDATE_SNAPSHOTS=1` to overwrite). CI runs only the `snapshots` test file (`cargo test -p curlybrace-ui --test snapshots -- --include-ignored`), so new shots must be compiled into that binary: `crates/ui/tests/snapshot_suites/*.rs`, included from `snapshots.rs`. A subdirectory is not auto-discovered as its own test target. Leave no stray `*.new.png` or `*.diff.png`.
- **Docs and changelog:** never edit `CHANGELOG.md` or older specs and plans.
- **Git:** plain, separate Bash calls. Conventional commits ending with
  `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>` and
  `Claude-Session: https://claude.ai/code/session_013Huvskh9CgrLjorzkd6uzc`. Never push.
- **Branches:** one per slice, each with a PR to `main`. Slices 1-3 say "Part of #159". Slice 4 says "Closes #159".

## Review Focus

- **Slice 1 changes nothing a person can see.**
  - The transcripts from T0 are byte-identical before and after.
  - No existing test assertion was edited beyond the mechanical list in the PR: the `request` field on literals, and a literal counter value replaced by one read from the state.
- **A late answer never lands in the wrong tab.**
  - Numbers are unique per feature across tabs.
  - `owner_of` is an exhaustive match on `Event`, so a new event must be classified.
  - A miss goes to the session's current tab, whose own staleness check drops it.
- **"Every tab of the session"** is reached at each site listed in the spec, each with a two-tab test.
- **Restore:**
  - no command leaves the reducer between launch and the person's Connect;
  - after `Connected`, only `ListDatabases` and the metadata reads of an Indexes tab are sent;
  - no secret, URI, document or editor text is in `workspace.json`.
- **Writes:**
  - `pending` stays one per connection;
  - a confirmed production write whose tab closed sends nothing;
  - read-only refusals are unchanged.

---

## Slice 1: the tab model, no behaviour change (branch `refactor/159-tab-model`)

### T0: characterisation transcripts, on unchanged code (written first)

**Files:**
- create `crates/ui/src/app/tab_characterisation_tests.rs` (registered with `#[cfg(test)] #[path]` from `app/tabs.rs` once T4 creates it; until then from `reducer.rs`'s test module, one line);
- create `crates/ui/src/app/transcript_support.rs` (`#[cfg(test)]`).

**Produces:**
```rust
/// One reducer step's observable effect: the commands sent (request numbers replaced by their
/// order of first appearance) and a fixed projection of the state.
pub(crate) struct Step { pub input: &'static str, pub commands: Vec<String>, pub state: String }
pub(crate) fn transcript(steps: Vec<(&'static str, Input)>, start: AppState) -> Vec<Step>;
fn normalised(commands: &[Command], seen: &mut Vec<u64>) -> Vec<String>; // Debug, numbers -> #n
fn projection(state: &AppState) -> String; // active, open, status per session, selected_ns,
    // issued/page query, loading_page, page len+key, tab, view_mode, indexes.{ns,loading},
    // aggregate.{in_flight.is_some, queued.is_some, preview kind}, editor.is_some, pending,
    // banner kind, workspace
```

- [ ] Write the tests, which record **today's** behaviour into checked-in golden text under `crates/ui/src/app/testdata/` (one file per test):
  - `scripted_browse_session_transcript_is_unchanged`. Steps: connect A, `Connected`, `Databases`, open `db.c`, `Results`, run a filter, `Results`, next page, edit and save, `Mutated`, `Results`, switch to Indexes, `Indexes`, switch to Aggregate, add a stage, `AggregatePreviewed`, an explain, a lost server, reconnect, `Connected`.
  - `background_session_transcript_is_unchanged`. Two sessions: A active, B parked. B gets `Collections`, `Results` for its own query, a stale `Results`, `Indexes` for an old request, and a lost server.
  - `stale_answers_transcript_is_unchanged`: an older `Results`, an older `Indexes`, an older preview, a `Failed{find}`, and an answer after Disconnect.
- [ ] Run them on unchanged code. They pass and write the golden files.
- [ ] Commit `test(tabs): record reducer transcripts before the tab model`.

### T1: the core echoes a request on Find, ExplainFind and EstimatedCount

**Files:**
- modify `crates/core/src/runtime/mod.rs`:
  - add `request: u64` to `Command::{Find, ExplainFind, EstimatedCount}` and `Event::{Results, Explain, EstimatedCount}`;
  - net 0 or less: move `ShellRefusal` and its `impl`s out to the new `crates/core/src/runtime/shell_refusal.rs`, re-exported with `pub use`.
- modify `crates/core/src/runtime/command.rs`:
  - add the three commands to `Command::request()`;
  - add `Event::request(&self) -> Option<u64>`, mirroring it for every numbered answer.
- modify `crates/core/src/runtime/handlers.rs` (801; changes in place, net 0): pass `request` through to the three answers.
- mechanical edits to the core tests and `crates/testkit` that build these literals (`request: 0`, or the sent number).

**Produces:**
```rust
impl Event {
    /// The number a numbered answer echoes (`Results`, `Explain`, `EstimatedCount`, `Indexes`,
    /// `IndexHistory`, `QueryShapes`, `GuardChecked`, `Aggregate{Previewed,Explained,Counted}`,
    /// `BulkCount`, `BulkSample`, `RequestFailed`; `AggregateRan` gives its `run`).
    pub fn request(&self) -> Option<u64>;
}
```

- [ ] Failing tests (core):
  - `find_results_echo_the_request`;
  - `a_failed_find_is_a_request_failure_with_its_number` (`RequestFailed { context: "find", request }`);
  - `explain_echoes_the_request`;
  - `estimated_count_echoes_the_request`;
  - `event_request_echoes_command_request_for_every_numbered_pair` (a table of command/answer pairs through the in-memory runtime);
  - extend `command.rs`'s tests: `request()` is `Some` for the three.
- [ ] Implement. Run `cargo +1.87.0 test -p curlybrace-core`. `wc -l` mod.rs and handlers.rs (net 0 or less).
- [ ] Commit `feat(runtime): echo a request number on find, explain and estimated count`.

### T2: the UI keeps today's handling of the three failures

**Files:**
- modify `crates/ui/src/app/reducer.rs`: in `numbered_read_failed` only, an early arm re-dispatches `RequestFailed { context: "find" | "explain_find" | "estimated_count" }` as `Event::Failed { session, context, error }` through `on_session_event`. No arm is added to the big matches.
- modify `crates/ui/src/app/actions.rs` (`issue_find`), `explain_actions.rs`, `aggregate_run_actions.rs:89` and `index_write_actions.rs:71`: send `request` (temporarily `0`; T3 numbers them).
- add `request: 0` to the UI test literals that build these events (mechanical; listed in the PR).

- [ ] Failing tests (`crates/ui/src/app/reducer_request_failed_tests.rs`):
  - `a_find_request_failure_raises_the_same_banner_as_a_find_failure`;
  - `a_lost_server_on_a_find_request_fails_the_root_like_before`;
  - `an_explain_request_failure_leaves_the_run_without_a_verdict_like_before`;
  - `an_estimated_count_request_failure_reads_count_unavailable_like_before`.
- [ ] Implement. The T0 transcripts are unchanged.
- [ ] Commit `refactor(ui): treat numbered find, explain and count failures as before`.

### T3: one process-wide source of request numbers for tab features

**Files:**
- create `crates/ui/src/app/request_numbers.rs`:
  ```rust
  //! The one source of the numbers a tab's reads carry: unique for the whole process, so an answer
  //! names exactly one waiting tab (see `tab_routing`). Same rule as `aggregate_counts::NEXT_REQUEST`.
  pub fn next() -> u64; // static AtomicU64 starting at 1, Ordering::Relaxed
  ```
- modify the allocation sites, each `self.x + 1` / `wrapping_add(1)` becoming `request_numbers::next()`:
  - `viewmodel/indexes.rs:133` (`requested`);
  - `viewmodel/queries.rs:154`;
  - `viewmodel/aggregate.rs:91,295`;
  - `viewmodel/aggregate_explain.rs:137,200`;
  - `app/aggregate_run_actions.rs:126` (`run`);
  - `viewmodel/aggregate_hint_list.rs:43` (`HINT_REQUEST_BASE + request_numbers::next()`; the `next` field goes);
  - the three senders of T2.
  - Find any other site with `rg -n 'request.*(\+ 1|wrapping_add\(1\))|run.*wrapping_add' crates/ui/src`. Guard and bulk keep their `AppState` counters, which are already app-wide.
- add `issued_request: u64` beside `issued_query`, to `AppState`, `SessionView` and its take/restore (exhaustive). T4 moves it to the tab. The verdict state records the number its `ExplainFind` carried.
- modify `reducer.rs` `Event::Results` arm: `current` additionally requires `request == state.issued_request`. Safe: with unique numbers, an identical query sent twice now takes only the newer answer, which is today's intent.

- [ ] Failing tests (`crates/ui/src/app/request_numbers_tests.rs`):
  - `numbers_never_repeat_across_threads` (8 threads x 1000);
  - `two_index_states_never_send_the_same_number`;
  - `two_drafts_never_send_the_same_preview_number`;
  - `a_results_answer_for_an_older_identical_query_is_stale`.
- [ ] Update the existing tests that asserted a literal counter value so they read the value from the state. Each one is listed in the PR body.
- [ ] Run `cargo +1.87.0 test -p curlybrace-ui`. The T0 transcripts are unchanged (numbers are normalised).
- [ ] Commit `refactor(ui): number every tab read from one process-wide source`.

### T4: `TabId`, `TabView`, the strip and the swap

**Files:**
- create `crates/ui/src/app/tabs.rs` (≤400 lines): `TabId`, `TabView`, `take_tab`, `restore_tab`, `with_tab`, `tab_opened`, `tab_closed`, `tabs_of`.
- create `crates/ui/src/app/tab_strip.rs` (≤300 lines): `TabStrip` (pure).
- create `crates/ui/src/app/tabs_tests.rs`, `tab_strip_tests.rs`.
- modify `state.rs`: two fields, `tabs: TabStrip` and `parked_tabs: BTreeMap<TabId, Box<TabView>>`. Update the doc comment's field list. Clear both in `session_cleared`.
- modify `session_actions.rs` `connect`: a newly opened session gets one tab (`tabs.opened_for(session)`). A reconnect keeps its tab.
- modify `session.rs` `session_dropped`: also drop the session's parked tabs and its strip entries (one call).
- modify `app/mod.rs`: `mod` lines only.

**Produces:**
```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(pub u64);

/// The tab-level fields; the current tab of each session keeps them in the session's view.
#[derive(Debug, Default)]
pub struct TabView {
    pub ns: Option<Namespace>,              // swapped with `sidebar.selected`
    pub find: FindBarState, pub issued_query: Query, pub issued_request: u64,
    pub page_query: Query, pub loading_page: bool,
    pub page: Option<Page>, pub page_key: Option<PageKey>,
    pub verdict: VerdictState, pub guard: Box<GuardState>,
    pub tree: TreeState, pub view_mode: ViewMode, pub tab: CollectionTab,
    pub indexes: IndexesState,              // sampler status copied on fan-out
    pub aggregate: Box<AggregateState>, pub aggregate_view: Box<AggregateView>,
    pub editor: Option<EditorState>,
    pub freshness: Freshness,               // slice 2 uses it; slice 1 always `Fresh`
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Freshness { #[default] Fresh, Stale, Dormant }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TabStrip {
    pub order: Vec<TabId>,
    pub active: Option<TabId>,
    pub owner: BTreeMap<TabId, Uuid>,
    pub current: BTreeMap<Uuid, TabId>,     // the session's tab whose fields live in its view
    pub mru: Vec<TabId>,                    // most recent first
    next: u64,
}
impl TabStrip {
    pub fn opened_for(self, session: Uuid, after: Option<TabId>) -> (Self, TabId);
    pub fn closed(self, tab: TabId) -> Self;   // also fixes active (right, else left neighbour)
    pub fn activated(self, tab: TabId) -> Self; // also bumps mru
    pub fn moved(self, tab: TabId, to: usize) -> Self;
    pub fn of_session(&self, session: Uuid) -> Vec<TabId>; // strip order
}
impl AppState {
    #[inline(never)] fn take_tab(&mut self) -> TabView;          // local mutation only
    #[inline(never)] fn restore_tab(&mut self, t: TabView);       // exhaustive destructure
    /// Makes `tab` its session's current tab (swapping with `parked_tabs`), with the session active.
    pub(crate) fn tab_current(self, tab: TabId) -> AppState;
    /// Runs `f` with `tab` current and its session active, then puts both back.
    #[inline(never)] pub(crate) fn with_tab(self, tab: TabId, f: impl FnOnce(AppState) -> Out) -> Out;
    pub(crate) fn tabs_of(&self, session: Uuid) -> Vec<TabId>;
}
```

- [ ] Failing tests:
  - `take_then_restore_is_identity` (every field set to a non-default);
  - `a_connected_session_has_exactly_one_tab_and_it_is_current`;
  - `reconnect_keeps_the_tab_id`;
  - `disconnect_drops_the_session_s_tabs`;
  - `tab_current_swaps_fields_and_ns`;
  - `with_tab_on_a_parked_session_s_parked_tab_restores_both`;
  - `with_tab_on_an_unknown_tab_changes_nothing`;
  - strip: `closing_active_focuses_right_then_left`, `mru_order`, `moved_clamps`, `ids_never_repeat`.
- [ ] Implement. T0 transcripts unchanged; the full suite is green.
- [ ] Commit `refactor(ui): a tab model behind the session view`.

### T5: answers find their tab

**Files:**
- create `crates/ui/src/app/tab_routing.rs` (≤300 lines) and `tab_routing_tests.rs`.
- modify `reducer.rs`: in `on_event`, after the `!state.is_open(session)` drop and before `with_session`, call `tab_routing::routed(state, session, ev)`. It returns either the event to handle as today, or the result of `with_tab(owner, ...)`. This is one call line in a separate `#[inline(never)]` frame; no new match arm.
- modify `aggregate_actions::on_event` and the other `is_*_event` early handlers (`aggregate_hint_actions`, `aggregate_explain_actions`, `aggregate_counts_actions`, `aggregate_run_actions`) to go through `routed` the same way: one line each.

**Produces:**
```rust
/// The parked (non-current) tab of `session` whose own state awaits `ev`; `None` means the
/// session's current tab takes it, as before the tab model. Exhaustive over `Event`.
#[inline(never)]
pub(super) fn owner_of(state: &AppState, session: Uuid, ev: &Event) -> Option<TabId>;
impl TabView {
    /// Per event kind: Results -> issued_request; Explain -> verdict's request; EstimatedCount ->
    /// builder or run dialog; GuardChecked -> guard; Indexes/IndexHistory -> indexes.request or
    /// hint list; QueryShapes -> queries.request; AggregatePreviewed -> in_flight;
    /// AggregateExplained -> explain; AggregateCounted -> counts.awaits; AggregateRan -> run;
    /// RequestFailed -> by context, the same fields.
    pub fn awaits(&self, ev: &Event) -> bool;
}
```

- [ ] Failing tests (two tabs built with `tab_opened` and no UI):
  - `two_tabs_on_different_collections_keep_independent_results`;
  - `two_tabs_on_the_same_collection_and_query_take_only_their_own_answer`;
  - `an_answer_for_a_closed_tab_is_dropped`;
  - `a_preview_answer_goes_to_the_draft_that_sent_it_and_releases_only_its_slot`;
  - `the_one_preview_in_flight_rule_holds_per_tab`;
  - `an_indexes_answer_goes_to_its_tab`;
  - `a_hint_list_answer_goes_to_its_tab`;
  - `a_count_answer_goes_to_its_tab`;
  - `a_guard_answer_goes_to_its_tab`;
  - `a_find_request_failure_goes_to_its_tab`;
  - `an_answer_nobody_awaits_goes_to_the_current_tab_as_before`;
  - `with_one_tab_per_session_every_event_goes_to_the_current_tab` (table over every `Event` variant).
- [ ] Implement. Add a debug test that runs the deepest path (a lost server during an Aggregate count, editor open, two tabs, the parked tab owning the count) without overflowing the stack.
- [ ] Commit `feat(tabs): route each answer to the tab that awaits it`.

### T6: the sites that reach every tab of a session

**Files:**
- create `crates/ui/src/app/tab_fanout.rs` (≤300 lines) and `tab_fanout_tests.rs`.
- modify, one call each:
  - `session.rs` `failed`;
  - `reducer_lost.rs` `connection_lost`;
  - `session_actions.rs` `connect`;
  - `reducer.rs` `mutated` and the `IndexChanged` / `AdminChanged` / `Sampler` arms (through helpers, not new arms);
  - `actions.rs` (`Event::Databases` / `Collections` handling via `sidebar.with_*`);
  - `admin_dialog_actions.rs:220`;
  - `queries_actions.rs` `profiling_answered`;
  - `code_export_actions.rs` `close_if_stale`.
- modify `state.rs` `Pending` to add `tab: Option<TabId>`, and `write_guard.rs` `ParkedWrite` to add `tab: Option<TabId>` (a confirm whose tab is gone sends nothing: `Banner::WriteNotSentConnectionChanged`).

**Produces:**
```rust
/// Applies `f` to every parked tab of `session` (the current one is handled by the caller's
/// existing code). Pure over `TabView`.
pub(super) fn parked_tabs_mapped(state: AppState, session: Uuid, f: impl Fn(TabView) -> TabView) -> AppState;
impl TabView {
    pub fn on_connection_lost(self) -> TabView;   // slice 1: same as today's `failed` for its fields
    pub fn marked_stale_if_on(self, ns: &Namespace) -> TabView;
    pub fn without_vanished(self, databases: &[String], collections: Option<(&str, &[String])>) -> TabView;
    pub fn with_sampler(self, status: SamplerStatus) -> TabView;
}
```

- [ ] Failing tests, two tabs each:
  - `lost_connection_resets_every_tab_of_the_session_and_no_other`;
  - `reconnect_keeps_every_tab_s_draft`;
  - `a_write_answer_closes_the_editor_of_the_writing_tab_only`;
  - `a_write_answer_frees_pending_even_when_its_tab_closed`;
  - `a_write_marks_other_tabs_on_the_namespace_stale`;
  - `an_index_change_relists_other_tabs_on_the_namespace`;
  - `a_dropped_collection_blanks_every_tab_that_showed_it`;
  - `a_vanished_database_blanks_its_tabs`;
  - `sampler_status_reaches_every_tab`;
  - `a_confirmed_production_write_whose_tab_closed_sends_nothing`;
  - `copy_as_code_closes_when_its_tab_is_not_in_view`.
- [ ] Implement. T0 transcripts unchanged, full suite green.
- [ ] Commit `feat(tabs): session events reach every tab of the session`.

### Slice 1 verification

- `cargo +1.87.0 fmt --check`, `cargo +1.87.0 clippy --workspace --all-targets -- -D warnings`, and `cargo +1.87.0 test --workspace --locked`.
- `cargo test -p curlybrace-ui --test snapshots -- --include-ignored`: no PNG changes.
- `cargo test -p curlybrace-ui --test layout_audit_800` and every `accessibility_*` test.
- Throwaway mongod: `local_e2e_flows`, `local_find_options_flows`, `local_aggregate_options_flows`, `local_aggregate_plan_flows`, `local_undo_flows`.
- `cargo llvm-cov -p curlybrace-ui`: new files at 80% or more.
- PR body: the list of mechanically edited assertions (T2, T3).
- **One deliberate change a one-tab session can see (T3, found in review).** A late failure of an
  older find, explain or estimated count, or an `Explain` / `EstimatedCount` answer carrying an old
  number, is now dropped by the tab that no longer awaits it. Before, a late `Failed { find }`
  raised a banner and ended the newer find's loading, and an explain or count answer was accepted by
  collection and query alone. Visible only as the absence of a spurious banner or verdict; a lost
  server still fails the root. The transcripts do not cover it, so `tab_single_tab_tests.rs` pins it.
- Size: L (about 1800 lines with tests). Highest risk of the four slices.

---

## Slice 2: tabs (branch `feat/159-tabs`)

### T7: tab actions (pure)

**Files:**
- create `crates/ui/src/app/tab_actions.rs` (≤400 lines) and `tab_actions_tests.rs`.
- modify `reducer.rs`: `Input::Tabs(TabAction)` (one variant).
- modify `reducer_dispatch.rs`: one arm.
- modify `actions.rs` `sidebar`: `SidebarAction::OpenInNewTab(Namespace)` delegates to `tab_actions`.
- modify `viewmodel/sidebar.rs`: add that variant.

**Produces:**
```rust
pub const MAX_TABS: usize = 20;
pub const CLOSED_KEPT: usize = 5;
#[derive(Clone, Debug, PartialEq)]
pub enum TabAction {
    Activate(TabId), New, OpenInNew { session: Uuid, ns: Namespace }, Duplicate(TabId),
    Close(TabId), CloseOthers(TabId), CloseRight(TabId), ReopenClosed,
    Move { tab: TabId, to: usize }, Next, Previous, NextMru, PreviousMru, Nth(usize), Last,
    Run(TabId), // a dormant tab's Run
}
pub(super) fn on_action(state: AppState, action: TabAction) -> Out;
```

Rules (from the spec):
- **Plain sidebar `Open`** is unchanged: it replaces the current tab's collection. On a session with no tab with a collection, it uses that session's empty current tab.
- **Duplicate** copies the namespace, find text, view mode, sub-tab and draft (the draft goes through `on_collection_changed`), then re-runs `page_query`.
- **Close** never sends a command, and never closes the last tab of a session (it empties it instead).
- **Activating a `Stale` tab** re-sends its `page_query` (plus its index list on Indexes). **A `Dormant` tab** sends nothing.
- **At `MAX_TABS`**, opening sets `tabs.notice = Some(LimitReached)` and sends nothing.

- [ ] Failing tests:
  - `open_in_new_tab_creates_a_tab_and_runs_the_default_query`;
  - `plain_open_replaces_the_current_tab_s_collection`;
  - `duplicate_copies_text_and_draft_not_results_and_takes_new_numbers`;
  - `close_active_focuses_the_right_neighbour`;
  - `closing_the_last_tab_of_a_session_empties_it`;
  - `close_sends_nothing_even_with_a_write_in_flight`;
  - `reopen_closed_restores_text_and_editor_without_results`;
  - `closed_stack_keeps_five`;
  - `the_21st_tab_is_refused_with_a_notice`;
  - `activating_a_tab_of_another_session_activates_that_session`;
  - `activating_a_stale_tab_refetches_its_page`;
  - `activating_a_dormant_tab_sends_nothing`;
  - `run_on_a_dormant_tab_sends_its_find`;
  - `next_previous_mru_nth_last`;
  - `move_reorders`.
- [ ] Commit `feat(tabs): open, duplicate, close, reopen and reorder tabs`.

### T8: lost connection keeps the tabs; eviction

**Files:**
- modify `crates/ui/src/app/tab_fanout.rs`: `on_connection_lost` keeps `ns`, `find`, `tab`, `view_mode`, draft and `editor`, and drops page, verdict, guard and preview. Freshness becomes `Dormant`. The current tab is covered too: a change in `session.rs` `failed` to keep those fields via `TabView` (one call).
- create `crates/ui/src/app/tab_eviction.rs` (≤250 lines) and tests.
- modify `viewmodel/page_key.rs`: `PageKey` gains `bytes: usize`, summed from the `bson::to_vec` it already does.

**Produces:**
```rust
pub const RETAINED_PARKED: usize = 6;
pub const RETAINED_BYTES: usize = 64 * 1024 * 1024;
/// Drops the page (and preview, chart series) of parked tabs beyond the MRU count or the byte
/// budget, oldest first; never the active tab or its split partner. Marks them `Stale`.
pub(super) fn evicted(state: AppState) -> AppState; // called after every reducer step that adds a page or changes the active tab
```

- [ ] Failing tests:
  - `lost_connection_keeps_every_tab_dormant_with_its_text`;
  - `reconnect_leaves_tabs_dormant_and_sends_only_list_databases`;
  - `the_seventh_parked_page_is_evicted_oldest_first`;
  - `byte_budget_evicts_before_count`;
  - `active_and_split_partner_are_never_evicted`;
  - `an_evicted_tab_is_stale_and_refetches_when_shown`;
  - `page_key_counts_bytes`.
- [ ] Update the slice 1 test `lost_connection_resets_every_tab_of_the_session_and_no_other` to the new rule. This behaviour change is intended and is named in the PR.
- [ ] Commit `feat(tabs): keep tabs through a lost connection and bound retained pages`.

### T9: titles and accessible names (viewmodel)

**Files:** create `crates/ui/src/viewmodel/tab_title.rs` (≤250 lines) and tests.

```rust
pub struct TabLabel { pub title: String, pub tooltip: String, pub accessible: String,
                      pub badge: ConnectionBadge, pub dormant: bool }
pub fn labels(state: &AppState, i18n: &I18n) -> Vec<(TabId, TabLabel)>; // strip order
```

- [ ] Failing tests:
  - `title_is_the_collection`;
  - `same_collection_in_two_databases_shows_db_dot_collection`;
  - `still_ambiguous_adds_the_connection`;
  - `duplicates_share_a_title_and_differ_by_position`;
  - `empty_tab_title`;
  - `accessible_name_names_production_and_read_only`;
  - `tooltip_names_the_sub_tab`.
- [ ] i18n keys: `tabs.accessible_name`, `tabs.badge_production`, `tabs.badge_read_only`, `tabs.empty_title`, `tabs.title_with_connection`, `tabs.title_with_db`, `tabs.tooltip`.
- [ ] Commit `feat(tabs): tab titles and accessible names`.

### T10: the tab strip view and shortcuts

**Files:**
- create `crates/ui/src/views/tab_strip.rs` (≤400 lines), `crates/ui/src/views/tab_strip_menu.rs` (≤200: the context menu and "All tabs"), and `crates/ui/src/views/tab_shortcuts.rs` (≤150).
- modify `crates/ui/src/app/browse_panel.rs`: draw the strip above `show_connection_header_with` (one call). The Dormant notice row with Run goes in `show_collection` (one call to a new `views/tab_dormant.rs`, ≤120).
- modify `crates/ui/src/views/sidebar.rs` `collection_menu`: add "Open in new tab"; middle click on a collection row; Alt+Enter on a focused row. These are mechanical edits; the logic goes in `views/sidebar_tabs.rs` (≤120).

**Shortcuts** (`tab_shortcuts.rs`, consumed only while the Browse workspace has no text field focused; Cmd = `Modifiers::COMMAND`):
- **Tabs:** Cmd+T new; Cmd+W close; Cmd+Shift+T reopen; Cmd+Shift+D duplicate.
- **Switching:** Ctrl+Tab / Ctrl+Shift+Tab MRU; Cmd+Shift+] / [ next / previous; Cmd+1..8 a position, Cmd+9 the last tab.
- **Reorder:** Ctrl+Shift+PageDown / PageUp move.
- **Check:** verify eframe binds none of these on macOS, and note it in the PR.

**Layout:**
- one row, `CONTROL_HEIGHT` high, `ROW_GAP` below;
- a horizontal `ScrollArea` with the active tab kept in view;
- tabs 96-180 pt wide;
- a 28 pt "All tabs" menu button pinned right;
- the limit notice inline at the right.

- [ ] Failing tests:
  - `crates/ui/tests/accessibility_tabs.rs`:
    - `strip_is_a_tab_list_with_named_tabs`;
    - `open_switch_close_by_keyboard`;
    - `middle_click_opens_in_new_tab`;
    - `context_menu_open_in_new_tab`;
    - `alt_enter_on_a_focused_collection_opens_in_new_tab`;
    - `focus_order_sidebar_strip_header_content`;
    - `close_button_is_named_after_its_tab`;
    - `all_tabs_menu_lists_every_tab`;
    - `limit_notice_is_announced`;
    - `dormant_tab_offers_run`.
  - `layout_audit_800.rs`: a page with 6 tabs, two of them production and read-only; with the "All tabs" menu open.
- [ ] i18n keys:
  - strip: `tabs.all_tabs`, `tabs.strip`, `tabs.new_tab`;
  - closing: `tabs.close`, `tabs.close_named`, `tabs.close_others`, `tabs.close_right`;
  - menu entries: `tabs.duplicate`, `tabs.move_left`, `tabs.move_right`, `tabs.open_in_new_tab`, `tabs.reopen_closed`;
  - notices: `tabs.dormant_run`, `tabs.limit_reached`.
- [ ] Commit `feat(tabs): the tab strip, its menu and keyboard shortcuts`.

### T11: snapshots and the real server

**Files:**
- create `crates/ui/tests/snapshot_suites/tabs.rs`;
- `snapshots.rs`: add one `#[path = "snapshot_suites/tabs.rs"] mod tabs_suite;` line after the test macro;
- create `crates/ui/tests/support/tabs_shots.rs`;
- create `crates/ui/tests/local_tabs_flows.rs` and add it to `scripts/integration-tests.sh`.

- [ ] Shots:
  - `tabs_strip`, `tabs_strip_dark`;
  - `tabs_strip_small` (800x500, 6 tabs);
  - `tabs_all_menu`;
  - `tabs_production_read_only`;
  - `tabs_dormant`.
- [ ] Throwaway mongod flows (`#[ignore]`, `CURLYBRACE_IT=1`):
  - `two_tabs_run_different_filters_and_keep_their_pages`;
  - `a_write_in_one_tab_refreshes_the_other_on_show`;
  - `closing_a_tab_drops_its_late_answer` (a `$where` sleep on the throwaway db);
  - `lost_and_reconnected_tabs_stay_dormant_until_run`.
- [ ] Commit `test(tabs): snapshots and real-server flows`.

### Slice 2 verification

The slice 1 list, plus the new accessibility, layout and snapshot tests, plus `local_tabs_flows` on a throwaway mongod. Size: L (about 1600 lines).

---

## Slice 3: split view (branch `feat/159-split`)

### T12: measure, then the layout decision (pure)

**Files:** create `crates/ui/src/viewmodel/split_layout.rs` (≤200) and tests.

```rust
pub const DIVIDER: f32 = 8.0;
pub const MIN_PANE_WIDTH: f32 = 420.0;   // fixed by the measurement step below
pub const MIN_PANE_HEIGHT: f32 = 250.0;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Orientation { SideBySide, Stacked }
#[derive(Clone, Copy, Debug, PartialEq)] pub enum SplitLayout { Side { ratio: f32 }, Stacked { ratio: f32 }, Single }
pub fn layout(central: egui::Vec2, preferred: Orientation, ratio: f32) -> SplitLayout;
pub fn clamped(ratio: f32) -> f32; // 0.25..=0.75
```

- [ ] Measurement step: a kittest in `crates/ui/tests/split_layout.rs` draws the Browse page in a pane of decreasing width and height. It records the narrowest width at which the find bar and the documents toolbar keep the layout-audit rules, and the lowest height that still shows 3 table rows. Fix `MIN_PANE_*` to those values, round up to 10 pt, and pin them in `pane_minimums_match_the_measured_page`.
- [ ] Failing tests:
  - `side_by_side_when_wide`;
  - `stacked_at_1000x700`;
  - `single_at_800x500`;
  - `preferred_stacked_is_kept_when_it_fits`;
  - `ratio_is_clamped`.
- [ ] Commit `feat(split): decide the split layout from the window size`.

### T13: split state and actions

**Files:**
- modify `crates/ui/src/app/tab_strip.rs`: `split: Option<Split>`.
- create `crates/ui/src/app/split_actions.rs` (≤250) and tests.
- add `TabAction::{ToggleSplit, OpenInOtherPane(TabId), FocusOtherPane, SetRatio(f32), SetOrientation(Orientation), ShowOtherPane}`.
- add `Input::InTab { tab: TabId, input: Box<Input> }`: the dispatch activates `tab`, then applies `input`. This is one arm in `reducer_dispatch.rs`.

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct Split { pub secondary: TabId, pub ratio: f32, pub orientation: Orientation, pub single_shows_secondary: bool }
```

- [ ] Failing tests:
  - `toggle_split_pairs_with_the_mru_other_tab`;
  - `toggle_again_closes_the_split`;
  - `closing_a_split_tab_ends_the_split`;
  - `in_tab_input_focuses_that_tab_first`;
  - `focus_other_pane_swaps_roles_not_positions`;
  - `opening_in_other_pane_with_one_tab_duplicates_it`;
  - `a_pane_on_another_connection_activates_that_session_when_focused`;
  - `run_dialog_closes_when_focus_moves_to_the_other_pane`;
  - `split_partner_is_not_evicted`.
- [ ] Commit `feat(split): two tabs side by side`.

### T14: split view drawing and the ID audit

**Files:**
- create `crates/ui/src/views/split_panes.rs` (≤350): the divider, keyboard on the divider, the single-pane chip.
- create `crates/ui/src/app/split_panel.rs` (≤250): draws the secondary pane by swapping its tab in (local `mem::take`, as `take_tab`), wraps its events in `Input::InTab`, and focuses on pointer press or focus gain.
- modify `app/panels.rs` `show_workspace`: one call.
- audit the `egui::Id::new` sites. For each one drawn inside a pane (find bar, documents, tree, table, aggregate, indexes and their helpers), switch to `ui.id().with(..)` or take the pane salt as an argument. List every site in the PR, with a "not in a pane" reason for each one left.

- [ ] Failing tests:
  - `crates/ui/tests/accessibility_split.rs`:
    - `divider_is_named_and_reports_its_value`;
    - `arrow_keys_move_the_divider_home_resets`;
    - `f6_moves_focus_to_the_other_pane`;
    - `clicking_the_other_pane_makes_it_active`;
    - `each_pane_header_names_its_connection_and_badge`;
    - `single_pane_chip_and_show_other_pane`;
    - `no_egui_id_clash_in_a_split_frame` (fails on egui's ID-clash warning, via `ctx.options` debug warnings captured in the harness).
  - `layout_audit_800.rs`: the one-pane fallback page.
  - `split_layout.rs`: the 1000x700 stacked page passes the same audit rules (no overlap, nothing past the edge, controls at least 24 pt).
- [ ] i18n keys:
  - divider: `tabs.split.divider`, `tabs.split.divider_value`;
  - menu: `tabs.split.off`, `tabs.split.open_in_other_pane`, `tabs.split.toggle`;
  - orientation: `tabs.split.orientation_side_by_side`, `tabs.split.orientation_stacked`;
  - fallback: `tabs.split.single_chip`, `tabs.split.single_show_other`.
- [ ] Shots in `snapshot_suites/tabs.rs`:
  - `split_stacked_1000x700`, `split_stacked_1000x700_dark`;
  - `split_side_1400x900`;
  - `split_single_800x500`.
- [ ] Throwaway mongod flow in `local_tabs_flows.rs`: `split_view_with_two_connections_keeps_both_pages` (two connections to the same throwaway server under different saved names).
- [ ] Commit `feat(split): draw two panes with a divider and keyboard focus`.

### Slice 3 verification

As in slice 2, plus `split_layout.rs`. Size: M (about 1000 lines).

---

## Slice 4: session restore (branch `feat/159-restore`, closes #159)

### T15: the workspace file (pure load and save)

**Files:**
- create `crates/ui/src/app/workspace_file.rs` (≤400) and `workspace_file_tests.rs`;
- modify `persistence.rs` `Recovery`:
  - add `Workspace { backed_up: bool }`, `WorkspaceNewer` and `WorkspaceSkipped { count: usize, backed_up: bool }`;
  - add their `i18n_key` arms;
  - this is a mechanical addition to a 424-line file.

**Produces:**
```rust
pub const VERSION: u32 = 1;
pub const MAX_TEXT_PER_TAB: usize = 256 * 1024;
pub const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceFile { pub version: u32, pub restore: bool, #[serde(default)] pub tabs: Vec<SavedTab>,
                           #[serde(default)] pub active: Option<usize>, #[serde(default)] pub split: Option<SavedSplit> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedTab { pub connection: Uuid, pub db: String, pub collection: String,
                      pub sub_tab: SavedSubTab, pub view_mode: SavedViewMode,
                      #[serde(default)] pub find: Option<SavedFind>,        // None when over the bound
                      #[serde(default)] pub draft: Option<SavedDraftText>,
                      #[serde(default)] pub text_dropped: bool }
pub struct WorkspaceStartup { pub file: WorkspaceFile, pub guard: PersistGuard, pub recovery: Option<Recovery> }
pub fn default_workspace_path() -> ConfigPath;           // <config>/curlybrace/workspace.json
pub fn load(path: &Path, backup: BackupFn<'_>, known: &BTreeSet<Uuid>) -> WorkspaceStartup;
pub fn save(path: &Path, guard: &PersistGuard, file: &WorkspaceFile) -> Result<(), PersistError>; // atomic_write only
pub fn snapshot(state: &AppState) -> WorkspaceFile;     // pure; never reads page/editor/secrets
```

- [ ] Failing tests:
  - `round_trip`;
  - `missing_file_restores_nothing`;
  - `corrupt_file_is_backed_up_with_a_notice`;
  - `unreadable_file_is_backed_up_with_a_notice`;
  - `failed_backup_blocks_saving`;
  - `newer_version_is_left_alone_and_saving_is_off`;
  - `unknown_connection_and_bad_namespace_entries_are_skipped_and_counted`;
  - `over_8_mib_is_refused_as_corrupt`;
  - `a_tab_over_256_kib_is_saved_without_text_and_flagged`;
  - `snapshot_holds_no_page_editor_or_uri` (the state holds a page, an editor with document JSON, and a connection whose URI has a password);
  - `saved_file_is_0600` (unix);
  - `opted_out_file_has_no_tabs`;
  - `more_than_max_tabs_are_cut_to_max`.
- [ ] i18n keys:
  - `persist.workspace_backed_up`, `persist.workspace_backup_failed`;
  - `persist.workspace_newer`;
  - `persist.workspace_skipped_backed_up`, `persist.workspace_skipped_backup_failed`;
  - `persist.workspace_write_failed`.
- [ ] Commit `feat(restore): a versioned workspace file`.

### T16: dormant tabs and materialising them on connect

**Files:**
- create `crates/ui/src/app/workspace_restore.rs` (≤300) and tests.
- modify `state.rs`: add `dormant: BTreeMap<Uuid, Vec<SavedTab>>` (by connection id), `workspace_restore: bool` and `workspace_guard: PersistGuard`.
- hook into the `Connected` path: after `after_connected` in `reducer.rs` (inside the existing `at_epoch` closure; one call), `workspace_restore::materialised(state, session)`.
- on connection delete (`connection_ops`), drop that id's dormant tabs.

**Produces:**
```rust
/// Turns `session`'s dormant tabs into `Dormant` tabs: ns, find text, sub-tab, view mode,
/// draft (preview `Idle`). Sends nothing. Restores the active position when it belongs here.
pub(super) fn materialised(state: AppState, session: Uuid) -> AppState;
```

- [ ] Failing tests:
  - `nothing_is_sent_between_launch_and_connect` (drive a whole launch: load, frames, no Connect, no commands);
  - `connect_materialises_tabs_and_sends_only_list_databases`;
  - `a_restored_documents_tab_sends_no_find_until_run`;
  - `a_restored_draft_with_out_stage_sends_nothing_and_run_still_needs_typed_confirmation`;
  - `a_restored_indexes_tab_lists_indexes_when_shown`;
  - `a_failed_connect_keeps_tabs_dormant_and_shows_the_failed_root`;
  - `a_deleted_connection_drops_its_dormant_tabs`;
  - `restored_tab_on_production_connection_carries_the_badge`;
  - `restored_tab_on_read_only_connection_refuses_writes`;
  - `max_tabs_respected_when_materialising_on_top_of_open_tabs`;
  - `text_dropped_tab_shows_its_note`.
- [ ] i18n key: `tabs.restore.text_dropped`.
- [ ] Commit `feat(restore): reopen saved tabs when their connection connects, without running anything`.

### T17: writing the file (debounce, exit) and the app wiring

**Files:**
- create `crates/ui/src/app/workspace_app.rs` (≤250; an `impl CurlybraceApp` block like `pipelines_app.rs`): `with_workspace_path`, `save_workspace_if_changed(now)` and `save_workspace_now`.
- modify `crates/ui/src/app/mod.rs` (1021; allowed lines only): the `mod` line, `.with_workspace_path(default_workspace_path())` in `launch`, `self.save_workspace_if_changed(..)` at the end of `run_frame`, and `self.save_workspace_now()` in `on_exit`. That is 4 added lines. Compensate by moving the free function `purges_after` (lines 157-162) into `connection_ops.rs`, so mod.rs does not grow, and say so in the PR.

Write rules:
- **At once:** when the structural snapshot (tabs, namespaces, sub-tab, view mode, split, restore flag) differs from the last written one, and on Run.
- **Text:** 1 s after the last change.
- **Never:** when the guard blocks or no path is set.
- A failed write raises `Banner::Persist`-style `WorkspacePersist` (key `persist.workspace_write_failed`) once per failure streak.

- [ ] Failing tests:
  - `new_app_never_writes_the_real_config_dir`;
  - `structural_change_writes_once`;
  - `typing_writes_after_a_second_only_once`;
  - `exit_writes`;
  - `blocked_guard_never_writes`;
  - `write_failure_raises_one_banner`;
  - `no_log_line_contains_query_text` (install a capturing test logger the way the core's private `log_capture.rs` does; the UI needs its own small copy under `#[cfg(test)]`. Assert that a sentinel filter value never appears).
- [ ] Commit `feat(restore): save the open tabs as they change`.

### T18: the opt-out

**Files:**
- modify `crates/ui/src/views/tools_settings.rs` (53): a "Reopen tabs at launch" switch with its hint.
- create `crates/ui/src/app/workspace_settings_actions.rs` (≤120): `ToolsAction::SetRestore(bool)` or a new `Input::Workspace(..)` (one variant). Off rewrites the file at once with no tabs and clears `dormant`.

- [ ] Failing tests:
  - `turning_restore_off_erases_saved_text_immediately`;
  - `turning_restore_off_clears_dormant_tabs`;
  - `restore_off_file_restores_nothing_next_launch`;
  - `switch_is_named_and_keyboard_operable` (kittest).
- [ ] i18n keys: `tabs.restore.setting`, `tabs.restore.setting_hint`.
- [ ] Commit `feat(restore): an opt-out that erases saved query text`.

### T19: real server, secrets audit, close-out

**Files:**
- create `crates/ui/tests/workspace_secrets_audit.rs` (like `connections_secrets_audit.rs`);
- add flows to `crates/ui/tests/local_tabs_flows.rs`;
- add shots to `snapshot_suites/tabs.rs`.

- [ ] Throwaway mongod flows:
  - `restore_after_relaunch_reopens_tabs_and_text`;
  - `restore_sends_no_find_before_run`: set profiling level 2 on the throwaway database; relaunch, connect, show the restored tab; assert `system.profile` has no `find` on the collection until Run is pressed; then exactly one.
  - `restore_never_runs_a_write` (a restored draft with `$out`: the target collection does not exist after connect and show).
- [ ] Secrets audit: `workspace_file_never_holds_a_uri_or_secret`.
- [ ] Shots: `tabs_restored_dormant`, `settings_restore_switch`.
- [ ] Commit `test(restore): real-server restore flows and the secrets audit`.

### Slice 4 verification

As in slice 2. Also: a manual `cargo run` against a throwaway mongod. Open 3 tabs, quit, relaunch: the strip is empty until Connect, then the 3 tabs come back dormant. Size: M (about 1100 lines).

---

## i18n summary (all new keys, nested and alphabetical)

```
persist.workspace_backed_up, workspace_backup_failed, workspace_newer,
        workspace_skipped_backed_up, workspace_skipped_backup_failed, workspace_write_failed
tabs.accessible_name, all_tabs, badge_production, badge_read_only, close, close_named,
     close_others, close_right, dormant_run, duplicate, empty_title, limit_reached, move_left,
     move_right, new_tab, open_in_new_tab, reopen_closed, strip, title_with_connection,
     title_with_db, tooltip
tabs.restore.setting, setting_hint, text_dropped
tabs.split.divider, divider_value, off, open_in_other_pane, orientation_side_by_side,
           orientation_stacked, single_chip, single_show_other, toggle
```

Suggested English (the implementer may polish it; the keys are binding):

- **Names:**
  - accessible name: "{title}, {namespace} on {connection}{badges}, tab {position} of {count}";
  - title with connection: "{collection} · {connection}".
- **Notices:**
  - limit reached: "{max} tabs are open. Close one to open another.";
  - dormant run: "Run the query to load documents";
  - text dropped: "This tab's query text was too long to keep.".
- **Split:** single chip: "Split view needs a larger window. Showing {title}.".
- **Restore setting:** "Reopen tabs at launch", with the hint "Keeps your open tabs and their query text on this computer. Never documents or passwords."

## Verification (per slice, in order, all with `cargo +1.87.0`)

1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --locked -- -D warnings`
3. `cargo test --workspace --locked`, which includes `i18n_usage_tests`, the `en.json` order and unused-key tests, `density_tests` and `layout_audit_800`.
4. `cargo test -p curlybrace-ui --test snapshots --locked -- --include-ignored` (regenerate only with `CURLYBRACE_SNAPSHOTS=1 UPDATE_SNAPSHOTS=1`, and review every PNG).
5. Throwaway mongod (above): `scripts/integration-tests.sh` with `CURLYBRACE_IT_MONGODB_URI` pointing at it.
6. `cargo llvm-cov -p curlybrace-ui -p curlybrace-core --locked`: new files at 80% or more.
7. `wc -l` on every touched file against the size rules. `git status` shows no stray PNGs.

## Calls made here (the spec's list; the defaults are implemented)

1. A second level of the existing swap; `SessionView` untouched.
2. Process-wide request numbers. The core echoes a number on `Find`, `ExplainFind` and `EstimatedCount`. `owner_of` routes by what each tab awaits, and a miss falls back to today's path. A process-wide static counter, not an `AppState` field, so that pure viewmodel methods do not change signature. `aggregate_counts::NEXT_REQUEST` is the precedent.
3. One strip for every connection. The Shell stays per connection. A plain click replaces the current tab's collection.
4. Writes stay single-flight per connection, and other tabs go stale.
5. Lost connection keeps the tabs dormant. Restored and reconnected tabs never send a find until Run.
6. Split is the active tab plus one secondary. Focus follows interaction. The layout falls back from side by side to stacked to a single pane, by measured minimums.
7. `workspace.json` v1, with the draft text and without the editor text. Restore is on by default, and the opt-out erases the file's text at once.
8. Bounds: 20 tabs, 6 parked pages or 64 MiB, 5 closed tabs, 256 KiB of text per tab, an 8 MiB file.
