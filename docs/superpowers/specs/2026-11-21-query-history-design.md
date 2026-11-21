# Query history and saved queries (#72)

## Intent

A person who runs the same `find` again and again should not retype it. The app remembers the queries that were run, lets the person name and star the useful ones, and reloads one with a click. Queries can hold personal values, so the file is private, the setting text says so, and query text is never logged.

## Goals

- **History:** per connection and namespace, last 100, deduplicated, newest first. Recorded when a find is actually sent from the find bar (Run, Enter, Run anyway after a guard warning). Not recorded for Previous, Next, Clear, restored tabs or reloads.
- **Favourites:** a named copy of a query, per connection and namespace, searchable by name and text. Up to 100 per connection. Names are unique per connection and namespace (same rule as saved pipelines: `validate_name`, `same_name`).
- **Load:** picking an entry fills the current tab's find bar (filter, sort, projection, skip, limit, query options) and does **not** run it. The person presses Run. Loading never changes the saved copy: editing the bar and running makes a new history entry only.
- **Setting:** "Remember queries" (default on). Off: nothing is recorded, and the history entries are erased at once. Favourites are kept, because the person made them on purpose (ruling 2).
- **Privacy:** file mode 0600, written atomically, `query_history.json` beside `connections.json`. The settings text and the History popup state that queries are stored as typed and may contain personal values. No log line contains query text, a name or a namespace value (log only error kinds).
- Every string is an i18n key. Accessibility tests pass at 1000x700 and 800x500.

## Non-goals

- Aggregation pipelines (saved pipelines already exist; history for them is a later issue).
- The per-connection History policy of #45/#81 (always / never / global default). This issue ships a single global switch and leaves a clear place for the per-connection policy.
- Sharing or exporting favourites, folders or tags, syncing between machines.
- Recording shell runs (they have their own history).

## Rulings (my calls, not agreed with the user)

1. One file `query_history.json` holds history and favourites, versioned (`VERSION = 1`), modelled on `workspace_file.rs` and `pipelines_store.rs`. Entries reuse `SavedFind` (filter, sort, projection, skip, limit, options).
2. Switch off erases history entries (like "Reopen tabs at launch" erases text) but keeps favourites; switching back on starts an empty history. The switch also stops recording at once.
3. Dedup key is the typed text plus options of an entry; a repeat moves it to the front and updates its time.
4. Caps: 100 history entries per (connection, namespace); 100 favourites per connection; 256 KiB total text per entry (over: not recorded); file 5 MiB (over on load: backed up and treated as damaged, like the other files). Whole-file size fit must never make `save` fail: record trims oldest history first.
5. Damage handling follows the existing pattern: corrupt/unreadable file is backed up with a notice, a failed backup blocks saving, a newer-version file is left alone with saving off. New `Recovery::QueryHistory*` variants and `banner.rs` arms.
6. Deleting a connection drops its history and favourites; duplicating a connection does not copy history (copies favourites, like saved pipelines).
7. The History button sits in the existing find-bar button row (`views/findbar.rs` `buttons()`), opening a popup (like `views/aggregate_saved.rs`). No new header row: `tests/browse_layout.rs` and `tests/layout_audit_800.rs` budgets must stay green. Width is added to `TRAILING_GUESS`.
8. Record point is where a find actually goes out (`explain_actions::run` and the guard `run_anyway` path), not `issue_find` (also used by Prev/Next/Clear). Write point is the `dispatch` state-diff in `app/mod.rs` (one hook call only; `mod.rs` is already over 1000 lines).

## Popup behaviour

- Button "History" next to Find/Clear. Disabled with a stated reason when there is no collection selected.
- Popup has a search field, two sections, Favourites then Recent. Each row shows the filter text (one line, elided), when it ran (Recent), and per-row actions: Load (row click), Star/Unstar (Recent: save as favourite with a name dialog), Rename and Delete (Favourites). "Clear history" asks for confirmation (first focus on Cancel).
- Empty states are plain sentences. Search matches names and text, case-insensitively, with a match count announced.
- Keyboard: open with the button, Up/Down within the list, Enter loads, Esc closes, Tab order follows the visual order. Each row has a screen-reader name like "Load query {text}".

## Slices

Single PR, tasks in order, each red-first:

1. **T1 Core data** (`crates/core/src/query_history.rs`, no I/O): `Store { history: BTreeMap<(Uuid, db, coll), Vec<Entry>>, favourites: BTreeMap<Uuid, Vec<Favourite>> }`, copy-on-write `recorded`, `starred`, `renamed`, `unstarred`, `cleared`, caps, dedup, name validation. Unit tests.
2. **T2 File layer** (`app/query_history_store.rs`): load/save, 0600, atomic, version, size caps, damage handling, `Recovery` variants and banner arms, i18n `persist.*` keys. Disk tests incl. corrupt, failed backup, newer version, off writes nothing.
3. **T3 App wiring** (`app/query_history_app.rs`): state field (boxed), `dispatch` hook, recording from the actual run path, setting action `ToolsAction::SetRememberQueries(bool)`, connection delete/duplicate hooks. Reducer tests including: Prev/Next/Clear do not record, restored tab does not record, guard warn then run anyway records once, off records nothing.
4. **T4 Popup UI** (`views/query_history.rs`, `viewmodel/query_history.rs`): button, popup, dialogs, search, load into bar without running, settings checkbox with the privacy text in `views/tools_settings.rs`. kittest tests, accessibility suite `tests/accessibility_query_history.rs`, layout guards, one snapshot (popup open) and the settings snapshot if it changes.
5. **T5 Audit and real flow:** log audit (capturing logger: no query text, name or namespace in any log line), permissions test, a relaunch flow against a throwaway mongod (record, restart, load, Run) in `tests/local_*`, README/docs mention.

## Done when

- [ ] Persistence tests; history off means nothing is written (test reads the file bytes).
- [ ] Every string is an i18n key; accessibility tests pass.
- [ ] Layout budgets and existing snapshots unchanged (only new PNGs).
- [ ] No log line contains query text (capturing logger).

## Size rules

Files under 800 lines. Do not grow: `app/mod.rs` (1002; one hook only), `app/bulk_actions.rs`, `app/reducer.rs` (764; one arm only), `tests/snapshots.rs`, `tests/accessibility_bulk.rs`, `app/workspace_fit_tests.rs`, `app/workspace_app_tests.rs`, `app/workspace_restore_tests.rs`. Put new code in new files.
