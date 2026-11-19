# Collection tabs, split view and session restore (#159)

Status: Draft. Written from issue #159 (related: #38, #67, #80, #82) and the code on `main` (`7dcbd8b3`, v0.17.0), without a Q&A round. Calls that are mine are marked **(my call)**. Owner questions are listed at the end; none of them blocks the plan, because each has a default. Plan: `docs/superpowers/plans/2026-11-19-collection-tabs-plan.md`. Earlier designs that still bind: `2026-10-14-multiple-sessions-design.md` (the active-view-plus-parked-views model this builds on), `2026-10-23-read-only-connections-design.md`, `2026-11-13-aggregation-builder-design.md` (the one-preview-in-flight rule), `2026-11-18-browse-header-design.md` (28 pt controls, 4 pt row gap, the 800x500 and 1000x700 budgets).

## What the code does today

These facts come from reading the code. They drive every decision below.

- **One view per connection.** `AppState` (`crates/ui/src/app/state.rs`, 505 lines) holds the *active* session's browsing fields directly: `status`, `failure`, `topology`, `latest_requested_epoch`, `sidebar`, `loading`, `find`, `issued_query`, `page_query`, `loading_page`, `page`, `page_key`, `verdict`, `guard`, `tree`, `view_mode`, `tab`, `indexes`, `aggregate`, `aggregate_view`, `editor`, `pending`, `ssh_endpoint`, `production` and `read_only`. The other open sessions are parked in `parked: BTreeMap<Uuid, SessionView>` (`app/session.rs`, 452 lines). `take_view` and `restore_view` swap a view in and out, and `restore_view` destructures exhaustively. `activated(id)` switches the session. `with_session(id, f)` runs a background event as if its session were active, then puts the old one back.
- **This pattern is why the code stays small.** Production code reads the foreground fields in hundreds of places: `selected_ns()` 45 times in 21 files (57 with tests), `indexes` 105 times (415 with tests), `aggregate` 69 (426), `pending` 36 (173), `find` 18 (109), `guard` 9 (61). None of these sites knows which session it is in.
- **The selected collection is `sidebar.selected`.** `SidebarState` holds the database tree *and* `selected: Option<Namespace>`. `selected_ns()` reads it. `with_databases` drops a selection whose database vanished, and `admin_dialog_actions.rs:220` clears it when the selected collection is dropped.
- **The sub-tabs are Documents | Indexes | Aggregate** (`CollectionTab`, `views/collection_tabs.rs`). They sit on the connection-header row in `app/browse_panel.rs`. **Shell is not a sub-tab.** It is a `Workspace` (Browse | Compare | Shell, switched in the sidebar) with one `ShellState` per session in `ShellTabs`, and it drives one `mongosh` process per session.
- **How answers are matched to requests** (`app/reducer.rs`, 715 lines; the runtime protocol is in `crates/core/src/runtime/mod.rs`, 1064 lines):
  - Session events carry `session`. A closed session's late answer is dropped (`!state.is_open(session)`). Connect-family outcomes are gated by epoch (`at_epoch`).
  - Numbered reads echo a `request: u64`. The UI drops an answer whose number is not the newest it sent (`request != state.indexes.request` style checks). These are `ListIndexes`, `IndexHistory`, `QueryShapes`, `GuardCheck`, `AggregatePreview`, `AggregateExplain`, `AggregateCount`, `BulkCount` and `BulkSample`, plus `AggregateRun`'s `run`. The runtime turns a failure of any numbered command into `RequestFailed { request }` (`handlers.rs:105`, using `Command::request()`).
  - **Some reads carry no number.** `Find` is matched by `selected_ns() == ns && issued_query == query`, and its failure is a bare `Failed { context: "find" }`. `ExplainFind` is matched by ns+query. `EstimatedCount` is matched by ns.
  - Writes are single-flight per session: `pending` for document writes, `indexes.writing` for index writes. `Mutated`, `IndexChanged` and `AdminChanged` carry only `ns` or `db`.
  - **Most counters are per sub-state.** `indexes.request`, `queries.request`, `aggregate.request` (a draft version), `aggregate.explain.request` and `aggregate.run.run` each start at 0 and only grow. `guard_request` and `bulk_requests` are already app-wide, "so the answer of a closed flow never matches a later one".
- **The one-preview-in-flight rule** lives in the draft itself (`viewmodel/aggregate_flight.rs`: `in_flight`, `queued`, newest wins). `on_connection_reset` keeps the draft and makes its preview stale. A reconnect carries the draft (`aggregate_for_reopen`).
- **Global, not per session:** `workspace`, `compare`, `conn`, `selection` (the sidebar multi-select), `clipboard`, `copy`, `code_export`, `sampling`, `explain`, `tools`, `pipelines` (saved pipelines, per connection id, in `pipelines.json`), `shell` (keyed by session), `write_warning` (a `ParkedWrite` naming its session and epoch), `admin`, `bulk` (they name their session), `banner` and `persist`.
- **Persistence** (`app/persistence.rs`, `app/settings_file.rs`, `app/pipelines_store.rs`): `atomic_write` writes a 0600 temp file in the same directory, syncs it and renames it. `back_up(path, tag)` copies a bad file aside (0600, never overwriting a backup). A failed backup sets `PersistGuard::blocked()`, and from then on every save is refused. A `Recovery` variant per file gives the startup notice, and notices queue behind the single banner slot (`with_recovery_notice`). `saved_pipeline::VERSION = 1`. A file from a newer version is left alone and saving is turned off (`Recovery::PipelinesNewer`). `CurlybraceApp::new` sets no path, so tests never write to the real config directory. `launch` sets every path.
- **Debug-build stack.** `AppState` is moved by value through every reducer step. `reducer.rs` and `code_export_actions.rs` mark frames `#[inline(never)]` because "the deepest event path is near the debug-build stack limit". Any extra swap layer adds to that path.
- **Files already over 800 lines** that this work would touch: `crates/ui/src/app/mod.rs` (1021), `crates/ui/src/app/bulk_actions.rs` (832), `crates/core/src/runtime/mod.rs` (1064) and `crates/core/src/runtime/handlers.rs` (801). `crates/ui/tests/snapshots.rs` is at 797, so new snapshot tests cannot go in it directly.
- **Shared egui IDs:** 48 `egui::Id::new(..)` calls in 38 view and app files. `Id::new` is absolute: `ui.push_id` does not salt it. Two panes drawing the same view in one frame would share those IDs.

## Decisions

### 1. Data model: a second level of the same swap **(my call)**

A **tab** is one collection view on one open session: `TabId(u64)`, from a counter that never reuses a number. It holds the namespace, the find bar (filter, sort, projection, skip, limit, options form), the issued and shown queries, the page and its key, the verdict, the guard, the tree, the view mode (Table / Tree / JSON) with its scroll and selection, the sub-tab (Documents / Indexes / Aggregate), the Indexes tab state (list, chart, Queries view, builder), the Aggregate draft and its view, and the document editor.

**What stays per connection** (one copy, shared by every tab of that session): `status`, `failure`, `topology`, the epochs, the sidebar's database tree and `loading`, `ssh_endpoint`, the `production` and `read_only` snapshots, `pending` (one document write in flight per connection, as today), the Shell (`ShellTabs`, one `mongosh` per connection), the sampler status, and everything that is already global (saved pipelines, history, tool paths, compare, copy, bulk, admin, write warning, banner).

**How it is stored: two levels of the existing swap.** The multiple-sessions design kept the active session's fields in `AppState` so that about 300 call sites did not change. This one does the same one level down:

- Every open session has exactly one **current tab**. Its tab fields live where they live today: in `AppState` when the session is active, or inside its parked `SessionView` when it is not. `SessionView`, `take_view`, `restore_view`, `activated` and `with_session` **do not change**.
- The session's other tabs are parked in `AppState.parked_tabs: BTreeMap<TabId, Box<TabView>>`. `TabView` is the tab-level subset of the fields listed above.
- `take_tab` and `restore_tab` swap the tab fields of the foreground. `restore_tab` destructures exhaustively, like `restore_view`, so a field added to `TabView` but not restored fails to compile.
- `with_tab(tab, f)` first makes the tab's session active (`with_session`), then makes the tab current (a tab swap), runs `f`, and swaps both back.
- **`sidebar.selected` stays the storage of the current tab's namespace (my call).** The tab swap moves it in and out. So `selected_ns()`, `SidebarState::selecting` (used by test fixtures) and the "selection inside a vanished database is dropped" rule keep working unchanged. A cleaner `AppState.tab_ns` field can follow later. It would touch 57 call sites for no change in behaviour.
- A `TabStrip` holds the order, the active tab, each tab's owning session, the current tab of each session, the most-recently-used list, the split (decision 4) and the closed-tab stack.

Why not one `Vec<Tab>` with `state.tabs[active].find` everywhere? Every one of those hundreds of call sites would change in one PR, along with every test that builds a state. The swap keeps the refactor additive. The risk moves to a few "for every tab of this session" sites, which are listed below and tested on their own.

**Sites that must reach every tab of a session** (today they act on the one view):

- `failed()` and `connection_lost`;
- connect and reconnect (`session_actions::connect`, `aggregate_for_reopen`);
- `session_dropped` (disconnect: the session's tabs go with it);
- `session_cleared` (the runtime stopped);
- `Mutated`, `IndexChanged` and `AdminChanged` (other tabs on the same namespace or database);
- the `Sampler` status;
- `ProfilingState` for a database;
- `with_databases` and `with_collections` (a tab whose database or collection vanished);
- `code_export_actions::close_if_stale`.

### 2. How an answer finds its tab **(my call)**

- **One process-wide request number source.** Every number a tab sends comes from one `app::request_numbers::next()`: find, explain, estimated count, index list and history, query shapes, Aggregate preview version, explain, hint list (still offset by `HINT_REQUEST_BASE`) and run. It is a static `AtomicU64`, the same pattern `aggregate_counts::NEXT_REQUEST` already uses for the counts. It is not an `AppState` field, because the numbers are taken inside pure viewmodel methods that have no access to `AppState`, and threading a counter through them would change about nine signatures and their callers. `guard_request` and `bulk_requests` are already app-wide and stay. The per-feature fields stay: they still mean "the newest number I sent", and every existing staleness check stays as it is. Only the source of the next number changes. Then **a number never repeats across tabs, features or sessions**, which is the property the routing relies on.
- **The core echoes a number on the three reads that lack one.** `Command::Find`, `Command::ExplainFind` and `Command::EstimatedCount` gain `request: u64`. `Event::Results`, `Event::Explain` and `Event::EstimatedCount` echo it. Adding them to `Command::request()` makes a failure of any of them `RequestFailed { context, request }` through the existing generic path. A new `Event::request()` mirrors `Command::request()`. In the first slice the UI converts `RequestFailed { context: "find" | "explain_find" | "estimated_count" }` back into the `Failed` handling it has today, so behaviour does not change.
- **Routing by "who is waiting for it", with no route table.** For a session event with a number, the owner is **the tab of that session whose own state awaits that number**. `TabView::awaits(request)` is one exhaustive function over the features: `issued_request`, `verdict.request`, the guard, `indexes.request`, `indexes.queries.request`, the draft's `in_flight`, explain, counts, hint list and run. If a non-current tab awaits it, the event runs inside `with_tab`. If no tab awaits it, the event goes to the session's current tab, exactly as today, where its own staleness check drops it. That fallback also keeps today's bookkeeping, such as `indexes.answered`, the rule that decides whether an old request's lost-connection failure still counts.
- **A stale answer for a closed tab is dropped** because a closed tab is no longer in `parked_tabs`, so it awaits nothing, and the number is unique, so no other tab's check accepts it. Nothing has to be removed when a tab closes. Nothing can leak.
- **Writes** stay single-flight per connection. `Pending` gains `tab: Option<TabId>`. A `Mutated` closes the editor of *that* tab (if it still exists) and frees `pending` regardless. Then every other tab of the session on the same namespace is marked **stale**: an active tab re-fetches at once, as today; a parked tab re-fetches when it is next shown. `IndexChanged` acts on the tab whose `indexes.writing` matches and marks the other tabs on that namespace for a re-list. `AdminChanged` reaches every tab of the session, so a dropped collection blanks every tab that showed it. `ParkedWrite` (the production warning) gains the tab too, and a confirm whose tab has closed sends nothing (`Banner::WriteNotSentConnectionChanged`, reused).
- **Events with no number and no tab** (`Connected`, `ConnectFailed`, `Databases`, `Collections`, lost-server failures, `Sampler`) go to the session as today. Where they change tab state, they reach every tab of the session (the list above).

### 3. Bounds and eviction **(my call)**

- **At most 20 tabs** in the whole app (`MAX_TABS`). Opening a 21st shows an inline notice in the tab strip (`tabs.limit_reached`) and opens nothing.
- **Result retention.** The active tab, its split partner and the 6 most recently used parked tabs (`RETAINED_PARKED = 6`) keep their page. A parked page is also dropped, oldest first, once the parked pages together exceed 64 MiB (`RETAINED_BYTES`). The size is measured once when a page arrives: `page_key` already serialises every document, and it now also returns the byte count. An evicted tab keeps its query and text and becomes **stale**: it re-fetches the same `page_query` when shown, a read it has already run once on this connection. The same rule applies to an Aggregate preview (it goes back to `Idle` with its draft intact) and to the Indexes chart series.
- **Closed tabs:** the last 5 closed tabs are kept without results, so they can be reopened (Cmd+Shift+T). An editor's unsaved text comes back with the tab, so closing a tab never loses typing for good.
- A page is never larger than today: `Query::limit` is capped at 1000 by `MAX_LIMIT`, and the default page is 50.

### 4. Tabs UI **(my call unless noted)**

- **One tab strip for the whole app**, across connections. It sits above the connection header in the Browse workspace. Selecting a tab of another connection makes that session active, so the sidebar root, header, badge and Shell follow. The strip is hidden in Compare and Shell. Choosing a tab switches to Browse.
- **Opening:**
  - **A plain click on a collection replaces the current tab's collection**, as today. Someone who never opens a second tab sees no change.
  - **Open in new tab** is a new collection context-menu entry (`tabs.open_in_new_tab`), a middle click, and **Alt+Enter** on a focused collection row. Cmd+click and Shift+click are already taken by multi-select (`sidebar_copy::click_mode`).
  - **Cmd+T** opens an empty tab on the active connection, showing "Select a collection".
- **Duplicate** (context menu; Cmd+Shift+D): copies the namespace, the find text, the view mode, the sub-tab and the Aggregate draft. It copies no results and no editor, and its numbers are fresh. The copy re-runs the query, a read the source already ran.
- **Close** (× button, middle click on the tab, Cmd+W, context menu "Close", "Close other tabs", "Close tabs to the right"). Closing the active tab focuses its right neighbour, else its left one. Closing a session's last tab leaves it an empty current tab. Closing never cancels an in-flight write: its answer is still applied to the connection (`pending` is freed).
- **Reorder:** drag a tab; Ctrl+Shift+PageUp / PageDown; context menu "Move left" and "Move right".
- **Switch:** Ctrl+Tab and Ctrl+Shift+Tab (most recently used order); Cmd+Shift+] and Cmd+Shift+[ (strip order); Cmd+1 to Cmd+8 for a position, Cmd+9 for the last tab. Arrow keys move between tabs while the strip has focus.
- **Titles:**
  - The collection name.
  - If two open tabs share it, `db.collection`.
  - If those still collide, `collection · connection`.
  - Duplicates of the same namespace share the title. The accessible name tells them apart by position.
  - Width is 96-180 pt, truncated with an ellipsis. The tooltip gives the full "connection / db.collection: sub-tab".
  - An empty tab is "Select a collection" (`tabs.empty_title`), and only the active empty tab is shown.
- **Badges on every tab:**
  - the connection's colour dot;
  - the production chip;
  - the read-only lock.
  These are the same `session_badge` the header uses, so they cannot drift, and a restored tab follows the same rules.
- **Overflow at 800x500.** The sidebar is 280 pt by default, leaving about 520 pt for the strip, so about four 120 pt tabs fit. The strip scrolls sideways and keeps the active tab in view. A 28 pt "All tabs" menu button at the right lists every tab with its badges. Only one row, 28 pt high (`CONTROL_HEIGHT`), with a 4 pt gap (`ROW_GAP`) below it, comes out of the documents area.
- **Accessibility.**
  - The strip is a tab list and each tab is a tab. Kittest checks roles and names.
  - Accessible name: `tabs.accessible_name` = "{title}, {db}.{collection} on {connection}{production}{read_only}, tab {n} of {count}". The suffixes are keys too.
  - The close button is "Close {title}".
  - Focus order: sidebar, then the tab strip (the active tab first), then the active tab's header and sub-tabs, then its content. In split view, the divider and the other pane come next.
- **Sidebar highlight** follows the active tab's namespace (it is `sidebar.selected`, decision 1).
- **Lost connection keeps the tabs.** This changes behaviour on purpose, from slice 2. Today a failed or lost session clears its view, collection included. With several tabs that would throw away every query on a blip. Each tab now keeps its namespace, find text, sub-tab, view mode and draft, and drops its page, verdict and preview. After the person reconnects, the tabs are **dormant** (decision 6).

### 5. Split view **(my call)**

- **Two panes:** the active tab and one secondary tab (`Split { secondary: TabId, ratio: f32, orientation }`). Cmd+\ splits the active tab against the most recently used other tab, or toggles the split off. A tab's context menu has "Open in other pane". The panes may be on different connections, and each pane's header names its own connection and badge.
- **Focus follows interaction.** A pointer press or keyboard focus inside the secondary pane makes it the active tab. The panes swap roles; nothing moves on screen. The secondary pane is drawn by swapping its tab into the foreground for the duration of the draw, the same swap the reducer uses. Its events are sent as `Input::InTab { tab, input }`, so the find bar's direct edits land in the right tab. **F6** moves focus to the other pane.
- **Modals belong to the active tab:** the document editor, index builder, bulk dialog and run confirmation. Interacting with the other pane makes it active, which closes an open run confirmation, as switching sessions already does.
- **Divider:** draggable, 8 pt hit area; the ratio is clamped to 0.25-0.75 and saved. When the divider has focus, the arrow keys move it in 5% steps and Home resets it to 50%. Accessible name `tabs.split.divider`, with its value given as a percentage.
- **Fallback by size**, decided by a pure function of the central area (`viewmodel/split_layout.rs`):
  - side by side if `width >= 2 * MIN_PANE_WIDTH + divider`;
  - else stacked if `height >= 2 * MIN_PANE_HEIGHT + divider`;
  - else one pane plus a chip "Split view needs a larger window: showing {title}" with a "Show other pane" toggle (`tabs.split.single_*`).
  - The starting values are `MIN_PANE_WIDTH = 420` and `MIN_PANE_HEIGHT = 250`. Slice 3's first task measures the Browse page's find bar and toolbar at the 28 pt density, as the browse-header spec did, and fixes the numbers in a test.
  - With the 280 pt sidebar, **1000x700** leaves about 720x660, so the panes **stack** (about 2x320 pt high). **800x500** leaves about 520x460, so it shows **one pane**. Side by side starts at about 1150 pt of window width.
  - The person's chosen orientation (`tabs.split.orientation_*`) is used whenever it fits.
- **egui IDs:** every pane is drawn under `ui.push_id(("pane", tab_id))`, and slice 3 audits the 48 `Id::new` sites. Each site that is drawn inside a pane changes to an ID derived from the pane. A kittest fails on egui's ID-clash warning in a split frame.

### 6. Session restore **(my call unless noted)**

- **File:** `<config dir>/curlybrace/workspace.json`, 0600. It is written only through `atomic_write`, and it is never written unless a path was set (`with_workspace_path`; `launch` sets it, `new` does not).
- **Saved** (schema `version: 1`):
  - the restore flag;
  - each tab's connection **id** (never a URI, user or secret), `db` and `collection`;
  - the sub-tab and view mode;
  - the find bar's typed text (filter, sort, projection, skip, limit and the options form) and the Aggregate draft's stage texts, enabled flags, mode and options form;
  - the tab order and active position;
  - the split's secondary position, ratio and orientation.
- **Never saved:**
  - documents, pages, previews, verdicts, explain output, index lists or charts;
  - the document editor's text (it is document data);
  - bulk, admin, run or index-builder dialogs;
  - passwords, passphrases, URIs and anything in `SecretStore`.
- **Bounds:** at most `MAX_TABS` entries, and at most 256 KiB of text per tab. A tab over that is saved without its texts, and on restore it shows `tabs.restore.text_dropped`. The file as a whole is refused above 8 MiB.
- **Load errors** follow `settings_file::load_read`:
  - **missing:** nothing to restore;
  - **corrupt or unreadable:** backed up (tags `corrupt` / `unreadable`), with a recovery notice (`Recovery::Workspace { backed_up }`); if the backup failed, the guard blocks saving;
  - **written by a newer version:** left as it is and saving turned off (`Recovery::WorkspaceNewer`, like `PipelinesNewer`);
  - **bad entries:** an entry whose connection id is no longer saved, or whose namespace fails `curlybrace_core::copy::names::validate_namespace`, is skipped and counted in the notice.
- **When it is written:**
  - at once on a change of tab structure (open, close, reorder, namespace, sub-tab, view mode, split) and on Run;
  - typing is debounced by 1 s against the last written snapshot (a small value compared for equality);
  - and once more in `on_exit`.
  - A failed write is a banner (`persist.workspace_write_failed`), like the other settings files. The log line carries only the `PersistError`, which is an `io::ErrorKind` with no path text or content.
- **When it is restored: never before the person connects, and never with an automatic connect, query or write.**
  - At launch the tabs are loaded as **dormant tabs** grouped by connection id. Nothing is sent, and nothing appears in the strip.
  - When the person connects that connection and `Connected` arrives, its dormant tabs become real tabs with their text, sub-tab and view mode. The active position is restored if it belongs to this connection.
  - **Nothing runs.** A dormant Documents tab shows "Run the query to load documents" (`tabs.dormant_run`) above an empty table, and its Run button is the only way to send the find. A dormant Aggregate tab shows its draft with an `Idle` preview, and its first preview goes out on the first edit or on the existing Refresh. An Indexes tab lists the indexes when it is shown, which is the metadata read every collection open already does.
  - **(my call)** A dormant tab stays dormant when it is focused. Restored query text may be expensive, and on a production connection an explicit Run is the safe default.
  - **Production and read-only:** restore changes no rule. The badge comes from `session_badge`. The production write warning, read-only refusals and the Aggregate preview's write-stage stripping all still apply. A restored draft holding `$out` or `$merge` can only be run through the typed confirmation.
  - **A connection that fails** shows its Failed root, as any failed connect does. Its dormant tabs wait until a connect succeeds. A connection that is deleted drops its dormant tabs, and the next write removes them from the file.
- **Opt-out:** a "Reopen tabs at launch" switch in the Settings dialog (`tabs.restore.setting`), on by default **(my call)**. Turning it off rewrites the file at once as `{"version":1,"restore":false,"tabs":[]}`, so saved query text is erased immediately rather than at the next launch.
- **Schema versioning:** `version` is required. 1 is the only version this build reads and writes. Fields added later are optional (`#[serde(default)]`) and do not change the version. A bump means an old build refuses to touch the file.

### 7. Detach

Detaching a tab into its own window is **out of scope**. egui's multi-viewport would need a second `AppState` view and a second runtime sink. Nothing in this design blocks it: a detached window would be a pane of a third kind.

## Slicing into PRs (my call)

Each slice ships on its own, behind no flag, on its own branch with a PR to `main`. Slices 1-3 say "Part of #159". Slice 4 says "Closes #159".

1. **Slice 1: the tab model, no behaviour change** (`refactor/159-tab-model`).
   - Request echo in the core.
   - One app-wide request counter.
   - `TabId`, `TabView`, `TabStrip`, take/restore/`with_tab`, the `awaits` routing and the per-session "every tab" sites.
   - Each session has exactly one tab, so users see nothing new.
   - **Left out:** any way for a person to open a second tab; the lost-connection-keeps-tabs change; eviction.
   - **Proof that nothing changed:**
     1. Every existing unit, kittest, snapshot and accessibility test stays green **without editing its assertions**. The only edits allowed are mechanical: the new `request` field on event literals, and a number read from the state instead of a literal where a test asserted a counter value. Both are listed in the PR.
     2. New **characterisation tests are written first, on unchanged code**:
        - a scripted browse transcript (connect, open, run, page, edit and save, Indexes, Aggregate preview, explain, lost connection, reconnect, a second session) recording every command sent and the key state after each step, with request numbers normalised to their order of first appearance;
        - a second transcript of a session's background events while another is active.
        Both must produce byte-identical output after the refactor.
     3. Reducer-level tests build two tabs through the model API (no UI) and prove the routing, which is Done-when 1 at the model level.
     4. The throwaway-mongod e2e flows (`local_e2e_flows.rs`, `local_find_options_flows.rs`, `local_aggregate_*_flows.rs`) pass.
2. **Slice 2: tabs** (`feat/159-tabs`). The strip, open in new tab, duplicate, close, reopen, reorder, keyboard, titles, badges, accessible names, the 20-tab limit, eviction, stale and dormant states, lost connection keeps the tabs. **Left out:** split, restore.
3. **Slice 3: split view** (`feat/159-split`). Two panes, the divider, focus following interaction, F6, the size fallback, the egui ID audit. **Left out:** restore, detach.
4. **Slice 4: session restore** (`feat/159-restore`). `workspace.json`, the dormant restore on connect, the opt-out. Closes #159. **Left out:** detach (follow-up issue to be filed), restoring across several app instances.

Done-when, per slice:

- "Two tabs keep independent queries, results and selections": slice 1 (reducer) and slice 2 (UI, e2e).
- "Split view at 1000x700, degrades at 800x500; keyboard; accessibility tests for opening, switching, closing and splitting": slices 2 and 3.
- "Restoring never runs a write and never sends a query before the person connects": slice 4.
- "Every string is an i18n key": every slice.

## Safety and security

- **No secrets saved:** `workspace.json` holds connection ids only. A test (`workspace_file_never_holds_a_uri_or_secret`) writes a workspace for a connection whose URI carries a password and an SSH passphrase in the secret store, then asserts that none of the URI, the user, the host or the secrets appear in the bytes. It sits beside the existing `connections_secrets_audit.rs`.
- **Query text is user data.** It can hold values (an email in a filter). It is never logged, never in an error, banner or `Debug` of a log line. The file is 0600. Log lines carry only `PersistError` kinds and counts. `TabView`'s `Debug` is derived but never logged. A test greps the new modules for `log::` lines that format a `FindBarState`, `QueryText` or `AggregateState`.
- **Bounded files and memory:**
  - 20 tabs, 256 KiB of text per tab, an 8 MiB file;
  - 6 parked pages or 64 MiB;
  - 5 closed tabs.
- **No new server load at rest:**
  - restore sends nothing before a connect, and after it only the metadata reads every open does;
  - a parked tab sends nothing until it is shown;
  - the one-preview-in-flight rule holds per tab (a parked draft sends no preview);
  - stale re-fetches are reads the tab already ran.
- **Writes keep every guard.** `pending` stays one per connection. The production warning names its session and now its tab. A confirm after the tab closed sends nothing. Read-only refusals are unchanged.
- **Error handling:**
  - every file error goes through `PersistGuard` and the recovery queue;
  - a restore entry that cannot be used is skipped and counted, never silently;
  - a route miss falls back to today's path, never to another tab.

## Tests

- **Unit:**
  - `TabView` take/restore round trip (exhaustive);
  - the request counter never repeating;
  - `awaits` for every feature;
  - routing with 1 and 2 tabs;
  - the closed-tab drop;
  - write fan-out;
  - lost connection on every tab;
  - eviction (count and bytes);
  - title disambiguation;
  - `split_layout` at 800x500, 1000x700 and 1400x900;
  - the workspace file: round trip, corrupt, unreadable, newer, oversized, unknown connection, bad namespace, opt-out wipe, 0600 mode, `atomic_write` only;
  - dormant materialisation sends nothing.
- **Kittest accessibility and layout:**
  - `accessibility_tabs.rs`: roles and names, open / switch / close / reorder by keyboard, the focus order;
  - `accessibility_split.rs`: divider name and value, F6, focus following interaction;
  - `layout_audit_800.rs` gains the strip with 6 tabs (no overlap, nothing past the right edge, controls at least 24 pt) and the split's one-pane fallback;
  - a 1000x700 stacked-split audit in a new `split_layout.rs`;
  - no egui ID clash in a split frame.
- **Snapshots** (in a new `crates/ui/tests/snapshot_suites/tabs.rs`, included from `snapshots.rs` by one `#[path] mod` line, because that file is at 797):
  - the strip in light and dark;
  - the overflow menu;
  - production and read-only badges on tabs;
  - split stacked at 1000x700;
  - the one-pane fallback at 800x500;
  - a dormant tab.
- **Throwaway mongod flows** (`local_tabs_flows.rs`, added by hand to `scripts/integration-tests.sh`):
  - two tabs on different collections run different filters and keep their pages;
  - a write in one tab refreshes the other on the same namespace;
  - closing a tab while its find is slow drops the late answer;
  - split view with two connections;
  - restore: launch, connect, the tabs are back, and the server's `system.profile` (profiling level 2 on the throwaway database) shows no find before Run.
  These run only against `/opt/homebrew/bin/mongod` on a spare port with a temp dbpath, never against a server already running.
- **Coverage:** at least 80% of new lines (`cargo llvm-cov`).

## Out of scope

- Detaching a tab into its own window (follow-up issue).
- Several Shells per connection, or a Shell per tab (the Shell stays per connection).
- Tabs for Compare, the connection manager or saved copies.
- Restoring results, scroll positions inside a page, or open dialogs.
- Restoring across several running app instances (last writer wins).
- The start screen with recent connections (#82) and a command palette (#80). Both can call the tab actions later. #80's action registry would list `tabs.*`.
- Pinned or "preview" tabs (VS Code style).
- More than two panes.

## Risks (biggest first)

1. **The refactor (slice 1).** The swap keeps call sites unchanged, but the per-session sites that must now reach every tab are where bugs will hide: lost connection, reconnect, disconnect, runtime stopped, write fan-out, `with_databases` and `close_if_stale`. A missed one leaves a parked tab with a stale page or a stuck `loading_page`. Mitigation: the exhaustive `TabView` destructure; one named test per site with two tabs; the normalised transcripts written before any change.
2. **Debug-build stack depth.** `with_tab` nests a tab swap inside `with_session` on the deepest event path, which is already near the limit. Mitigation: `TabView` is boxed; `with_tab` and its helpers are `#[inline(never)]`; the deepest existing path (lost connection during an Aggregate count with an editor open) runs in a debug test with two tabs. Fallback if it still overflows: run the reducer on a thread with a larger stack in debug only. That is a known pattern, but it is a last resort.
3. **Changing the request counter's source** touches about 9 allocation sites and every test that asserted a literal counter value. Mitigation: the transcript test normalises numbers; the PR lists every edited assertion.
4. **The core protocol change** touches about 80 `Find`/`Results` literals and two files already over 800 lines. Mitigation: a mechanical commit of its own. `runtime/mod.rs` and `handlers.rs` must end with net 0 lines (a self-contained type moves out to compensate), checked with `wc -l`.
5. **egui ID clashes in split view** (48 absolute IDs). Mitigation: the audit task and a clash-detecting test.
6. **Memory with many tabs.** It is bounded by count and bytes, but a single 1000-document page of large documents in the *active* tab is not bounded by this work, just as it is not today.

## Calls I made (the owner may overrule; the defaults are what gets built)

1. A second level of the existing swap (current tab in the session view, others in `parked_tabs`) instead of rewriting `AppState` around a tab list.
2. One app-wide request counter, and the core echoes a number on `Find`, `ExplainFind` and `EstimatedCount`. Answers are routed to the tab that awaits the number, with no route table. A miss falls back to today's path.
3. One tab strip across connections. The Shell stays per connection. A plain sidebar click replaces the current tab's collection; a new tab comes from the context menu, a middle click, Alt+Enter or Cmd+T.
4. Writes stay single-flight per connection. Other tabs on the same namespace are marked stale and refresh when shown.
5. Lost connection keeps the tabs (dormant). Restored and reconnected tabs never send a find until Run, even when focused.
6. Split view is the active tab plus one secondary. Focus follows interaction. Side by side falls back to stacked, then to one pane, by measured minimum pane sizes.
7. `workspace.json` v1, 0600, through `atomic_write`, with the backup, guard and newer-version rules. The Aggregate draft text is saved and the editor text is not. Restore is on by default, and the opt-out erases the file's text at once.
8. Bounds: 20 tabs, 6 parked pages or 64 MiB, 5 closed tabs, 256 KiB of text per tab.

## Owner questions (not blocking; the defaults above are built)

1. The issue lists "Documents / Indexes / Aggregate / **Shell** sub-tab". The Shell is a per-connection workspace with one `mongosh` process. Is one Shell per connection right, or do you want a Shell per tab (one process per tab)?
2. The one-preview-in-flight rule: per tab (the issue's wording, and the default) or per connection (protects the server more, with two visible panes at most)?
3. Should a restored or reconnected tab run its find when it is focused, at least on non-production connections? The default is never: explicit Run only.
4. Should the Aggregate draft text be restored? The default is yes; `pipelines.json` already stores stage text at the same privacy level.
5. Restore on by default?
6. Should a plain sidebar click focus an existing tab that already shows that collection, instead of replacing the current tab's?
7. Are 20 tabs and 6 retained parked pages the right defaults?
8. With two app instances open, is last-writer-wins on `workspace.json` acceptable?
