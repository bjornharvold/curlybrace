# Aggregation builder, slice 2: saved pipelines and a confirmed write run

Status: Draft, written from issue #67 and the slice 1 design without a Q&A round (the owner said "just start"). Calls that are mine are marked **(my call)**. Slice 1: `2026-11-13-aggregation-builder-design.md`.

## Intent

Keep a pipeline you built (slice 2a), and run one that ends in `$out` or `$merge` for real, with the same care the app gives a bulk delete (slice 2b). Two PRs, in this order: 2a first (no new safety design), then 2b.

## Slice 2a: saved pipelines

### Goals
- **Save** the current draft under a name; **Open** a saved pipeline into the draft; **Rename**; **Delete**. A saved pipeline belongs to one connection and is offered on every collection of that connection. **(my call)** It records the collection it was saved from as a hint, shown in the list, never enforced.
- The draft header shows the name of the opened pipeline and a "modified" mark when the draft differs from what was saved.
- Saved pipelines survive restarts and are never lost to a failed write: a failed save keeps the draft and says so.

### Non-goals
Sharing or syncing between machines, import/export of the file, folders, versions or history of a pipeline, saving the preview result.

### Behaviour
- **Storage (my call):** one file, `pipelines.json`, next to `connections.json` (`dirs::config_dir()/curlybrace/`), a JSON object `{ "version": 1, "connections": { "<uuid>": [ { "id", "name", "ns_hint", "saved_unix", "stages": [ { "text", "enabled" } ] } ] } }`. Written through the app's existing `atomic_write` (0600, temp + rename); a corrupt or unreadable file is backed up and replaced by an empty store with a recovery banner, exactly like `tools.json` (`app/tool_settings.rs`, `app/settings_file.rs`); a failed backup blocks later saves (`PersistGuard`) so a good file is never overwritten.
- **Ownership:** the UI owns the store (no Command/Event), like the tool settings. The path is injectable for tests (`with_pipelines_path`).
- **Names:** trimmed, 1 to 80 characters, unique per connection (case-insensitive). Saving under an existing name asks to replace it. **(my call)** At most 200 saved pipelines per connection and 1 MB of stage text per pipeline; the file is refused at load above 5 MB (treated as corrupt).
- **Open** replaces the draft. When the draft has unsaved changes the app asks first (Replace / Cancel). Opening never sends a preview by itself beyond the normal "draft changed" auto refresh.
- **Connection lifecycle:** deleting a connection removes its pipelines; duplicating a connection copies them (same hooks as the tool settings overrides).
- **Secrets:** pipelines can hold values from the user's data, so the file is 0600; nothing is logged but counts and error variants.
- **UI:** the aggregate toolbar gets **Save** (opens a small dialog with a name field), **Open** (a menu listing names with the collection hint; Rename and Delete per item; Delete asks for confirmation). Every label is an i18n key (`aggregate.saved.*`); every control is named for accessibility; works at 800x500.
- **Errors in words:** name empty/too long/duplicate, store full, file could not be written (the draft is kept), file recovered at start.

### Safety
A saved pipeline is data; opening it runs nothing. The slice 1 guarantees are unchanged: previews never write.

## Slice 2b: a confirmed `$out` / `$merge` run

### Goals
- A **Run** button is available when the pipeline's last enabled stage is `$out` or `$merge`. It never exists for a pipeline without one (previews stay the only way to look at the others).
- Pressing it opens a confirmation dialog that states: the source collection, the **target** (`db.collection`, resolved from the stage, including the `{ db, coll }` and `$merge: { into }` forms), what the stage does (`$out` **replaces the whole target collection**; `$merge` inserts, merges, replaces or fails per its options), the estimated document count of the target before the run, and that this run **cannot be undone** by the app (no undo snapshot in this slice).
- The user must **type the target collection name** to enable the final button, always (not only on production). A production connection additionally shows the production badge in the dialog. A read-only connection cannot run (the button is disabled with the reason).
- The run is one command with a time limit (`AGGREGATE_RUN_MAX_TIME`, **(my call)** 60 s server side; the client waits a little longer), reports success with the target's count after, or a clear error; a timeout says the server may or may not have finished.

### Non-goals
Undo of a run, progress streaming, Cancel mid-run (the time limit bounds it), running a write stage that is not last, several write stages, `allowDiskUse` and the other options (slice 4), scheduling.

### Behaviour
- Core builds the run pipeline from the enabled stages **without** the preview `$limit` and **with** the trailing write stage kept; anything else (a write stage not last, nested, a target that cannot be resolved, an empty pipeline) is a refusal with a message and nothing is sent.
- New `DocumentStore::aggregate_write(ns, pipeline, max_time) -> Result<()>`: accepts only a pipeline whose **last** stage is the single write stage (defence in depth, checked again in the store), runs it, drains the cursor. `ReadOnlyStore` refuses with `ReadOnly` before anything else. `Command::AggregateRun` is a write in `Command::is_write`; the production guard is satisfied by the typed dialog (`Guard::Confirmed`, as bulk does); a new `WriteKind` names it for banners.
- The preview command and `aggregate_preview` keep refusing every write stage: the two paths never mix.
- After a successful run the app asks for the target count (`estimated_count`) and shows it; the sidebar's collection list is refreshed when the target is new.
- Interrupted connection: the outcome is unknown and is reported as such (same wording family as an interrupted bulk save).

### Safety
No code path sends a write stage except `aggregate_write`, which needs the typed confirmation in the UI flow and re-checks the shape in core. Tests: the in-memory store records what it received; a preview never forwards a write stage; a run never happens without the confirmation state; read-only and production rules.

## Constraints
Rust 1.87.0 (CI pin, no let-chains); every user-facing string an i18n key (nested in `crates/ui/locales/en.json`); files under 400 lines (800 max); no new dependency; real objects over mocks; real `mongod` only as a throwaway instance on a spare port (never an existing one), for the integration test of the write run. No hardcoded secrets; error messages do not echo server text beyond the existing mapped messages.

## Left to the builder
Dialog layout, the Open menu's ordering (most recently saved first), exact wording within the i18n keys, keyboard flow.

## Later
Slice 3: export as driver code (#73), explain (#69), per-stage counts. Slice 4: options (collation, hint, allowDiskUse, read preference; #75).
