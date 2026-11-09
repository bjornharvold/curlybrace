# Bulk update and delete by filter — design

Issue: #51. Related: #38 (GUI layout), read-only connections (#109), production flag (#108).

## Intent

Today documents are edited, cloned and deleted one at a time. A user needs to change or remove
every document that matches the current filter, without the risk that a slip wipes a collection.
The feature is a guarded flow: see how many documents match, see exactly what the first three
would look like after the change, confirm, run, read a report of what happened.

The thing to protect the user from, in their words: **accidentally wiping the whole collection
with an empty filter.** Everything else in the design serves that.

## Goals

- **Update many** with an update document (`$set`, `$unset`, `$inc`, and other update operators),
  an aggregation-pipeline update, and an optional `upsert`.
- **Delete many** by filter.
- A preview before anything runs: the count of matching documents (bounded by `maxTimeMS`), and
  the first 3 matches shown as before/after (update) or as the documents that would go (delete).
- A visible "this cannot be undone" warning in every confirmation.
- Cancel while running, where the server allows it.
- A report: matched, modified (or deleted), upserted, errors. A failure part-way leaves a clear
  statement of what was applied and what was not.
- The core enforces read-only and rejects an empty filter unless explicitly flagged, so a UI bug
  cannot bypass either.
- Every user-facing string is an i18n key in `crates/ui/locales/en.json`; accessibility tests pass.

## Non-goals

- **Undo.** Deliberately deferred to a follow-up issue (snapshot the affected documents before the
  run, offer "Undo" afterwards, detect documents changed since). This version says "cannot be
  undone" and means it. The design must not make undo harder: a run has one report value that a
  later undo feature can extend.
- Bulk insert, import, find-and-modify, or changing documents across several collections.
- Server-side enforcement. This is a client guard, like read-only.

## Anti-goals

- Any path by which an empty filter reaches `update_many` or `delete_many` without the explicit
  "all documents" flag, including from a UI bug or a saved/pasted query.
- Operating from a projected view. Its documents are partial; the count and preview would lie
  about what the write touches.
- A read-only connection performing a bulk write by any path.
- A preview that differs from what runs: the preview filter, the update and the options must be the
  exact values sent to the write.

## Decisions (made with the user)

- **Empty filter** is an explicit, separately confirmed choice ("all documents").
- **Preview shows 3 documents**, not N.
- **Bulk delete always requires a typed confirmation**, whatever the count.
- **Update, upsert, pipeline update and cancel are all in the first version.**
- **Undo is a follow-up**, not part of this work.
- **The "after" preview is exact, computed by running the real update inside a transaction on the
  first 3 matching documents and rolling it back.** It is not an emulation, so it is right for
  every operator and for pipeline updates.

## Decisions (my calls, flagged for review)

- **What is typed** is the collection name, the same rule as the existing typed confirmation
  (`typed_text_matches`, trimmed, exact, a blank requirement never passes).
- **Bulk update needs the typed confirmation** only when the matched count is over
  `BULK_UPDATE_TYPED_CONFIRM_THRESHOLD` (100, a named constant) or the connection is production.
  Otherwise a confirm button after the 3-document preview is enough.
- **"All documents"** (empty filter) on either operation adds a separate step before the normal
  confirmation: an explicit tick, then the typed confirmation as above (always required for
  delete; for update it is required too, since "all documents" is the dangerous case).
- **Production** already warns before every write through `AppState.write_warning`. Bulk writes go
  through that same parking, so a production connection sees that warning and then the bulk flow's
  own typed confirmation; the bulk flow must not show two stacked dialogs saying the same thing.
- **Where transactions are unavailable** (a standalone server, a capped collection, a server too
  old for them, or any transaction error) the update preview degrades to the 3 "before" documents
  with a visible note that the after-state could not be previewed. The rest of the flow is
  unchanged; the typed confirmation is then required for update too, because the user is
  confirming blind. The preview never silently shows a guessed "after".
- **Preview mechanics.** Read the first 3 matching documents with `find` (limit 3, the filter
  alone). Open a session transaction; run the user's real update with `_id: {$in: those ids}`
  merged into the filter; re-read those ids inside the transaction; always abort, on success and
  on every error path. For an upsert that matches nothing, run the upsert once inside the
  transaction with the user's filter, read the inserted document, and abort. The preview is
  therefore not a lock-free read: it takes brief write locks on 3 documents and can fail on a
  write conflict, which is reported as "preview unavailable", not as a failed update.
- **The preview never commits.** An abort that itself fails is reported loudly, because the
  transaction is then left to time out on the server; no code path calls commit.
- **Count** uses `countDocuments` with `maxTimeMS`. If it times out, the flow shows "count
  unavailable" and treats the count as over the threshold, so the typed confirmation is required
  for update as well as delete.

- **Production and the typed confirmation.** The bulk flow's typed confirmation is the
  acknowledgement. The run command is dispatched carrying an "already acknowledged" marker, so
  `write_warning` does not park it a second time. The bulk dialog itself shows the production notice
  when the connection is production. One dialog, not two.
- **What "empty filter" means.** The parsed filter document has no keys (`{}`). Anything else,
  including `{"$expr": true}`, counts as an explicit filter the user wrote. The core checks the
  parsed document, never the text, so whitespace in the query bar is empty too.
- **Which part of the view is used.** Only the filter. The write ignores sort, skip and limit, so
  the count and the preview use the filter alone, and the dialog says "all documents matching the
  filter, not just this page".
- **Unknown outcome.** `update_many` and `delete_many` are single non-atomic commands. A network
  drop or timeout yields no counts, so the report has an explicit "outcome unknown" variant that
  says the write may or may not have been applied. No follow-up count is attempted. `max_time`
  applies to the count and the preview, not to the write: aborting a write at a time limit would
  leave a partial state, and cancel already covers stopping it.

## Current state (from code reading, 2026-11-09)

- Workspace crates: `core`, `ui`, `app`, `testkit`.
- `DocumentStore` (`crates/core/src/store.rs`) has per-`_id` `insert`/`replace`/`delete` and
  `bulk_write` over specific `_id`s (used by Data Compare apply). There is no count-by-filter,
  update-many or delete-many.
- `ReadOnlyStore` (`crates/core/src/read_only_store.rs`) wraps the store and forwards **every**
  trait method explicitly, refusing writes with `CoreError::ReadOnly`. `store.rs` documents the
  rule: adding a write method means updating `ReadOnlyStore`, the `WRITES` list and
  `every_write_method_is_covered` in `read_only_store_tests.rs`.
- `Connection` has `production` and `read_only` flags, loaded leniently and failing safe.
- `Query` has `filter` and `projection`, so a projected query is detectable.
- `find` already supports a limit, which gives the 3-document preview with no new method.
- UI: `viewmodel/typed_confirmation.rs` holds the shared typed-text rule; the index drop flow
  (`index_write.rs`, `DropConfirm`) is the pattern for a confirm that waits on typed text.
- Cancellation of long jobs uses a shared flag (`runtime/job_flag.rs`, `runtime/mod.rs`).
- Prior specs and plans live in `docs/superpowers/specs` and `docs/superpowers/plans`.

## Design

### Core

Three new `DocumentStore` methods, all with a safe default that refuses (like the other optional
methods), implemented by `MongoStore`, forwarded by `ReadOnlyStore`:

- `count_matching(ns, filter, max_time) -> Result<u64>`: a read, never guarded.
- `preview_update(ns, request) -> Result<UpdatePreview>`: returns up to 3 before/after pairs, or
  the before documents with a "no after-state" reason when transactions are unavailable. It runs
  a transaction that is always aborted, so it **writes nothing**, but it executes the user's update
  code, so it is classified as a write: `ReadOnlyStore` refuses it, and it is listed in `WRITES`.
  It applies the same empty-filter rule as `update_many`.
- `update_many(ns, request) -> Result<BulkWriteReport>`: a write.
- `delete_many(ns, request) -> Result<BulkWriteReport>`: a write.

A request value carries the filter, the update (or none for delete), the options (`upsert`), a
`max_time`, and a `MatchScope` that is `Filtered` or `AllDocuments`. **The empty-filter rule lives
in the core:** an empty filter with `MatchScope::Filtered` fails with a typed `CoreError`
(a new variant with its own i18n key and a distinct-key test). `AllDocuments` with a non-empty
filter is also rejected, so the flag always means what it says.

The update is a validated enum: an operator document (every top-level key must be an update
operator) or a pipeline (an array of stages). Validation fails fast with a clear error and never
echoes document values. Pipeline updates are limited to the update-pipeline stage set the server
accepts (`$addFields`/`$set`, `$project`/`$unset`, `$replaceRoot`/`$replaceWith`); anything else is
rejected before sending.

The report is one value: matched, modified or deleted, upserted ids, and the failure if any. A
failure after a partial write returns the report of what was applied, not only the error. It
never copies server text that could quote document values (same rule as `mongo_write.rs`).

Cancel: the run takes a cancellation flag in the style of `job_flag.rs`. Cancelling stops the work
the client controls and, where the server allows, kills the operation by its op id; the report
says "cancelled" and what had been applied by then. `update_many`/`delete_many` are single server
commands, so mid-run cancel is best-effort: the report must say when it could not stop the server
and the outcome is therefore unknown, rather than claiming "nothing happened".

### Runtime

A new command family on the existing runtime (`runtime/handlers.rs`) for: count + preview, run
update, run delete, cancel. `Command::is_write()` classifies run update/delete as writes so the
existing read-only and production handling applies. Count + preview is a read.

### UI

- An entry point on the collection's data view that acts on the **current filter**, offered only
  when the view is not projected and the connection is not read-only; otherwise it is disabled
  with the reason (the existing disabled-hover pattern, new i18n keys).
- A step flow: (1) define: update document or pipeline + upsert, or delete; (2) preview: count,
  3 before/after (or the 3 that would be removed), the cannot-be-undone warning; (3) confirm:
  typed confirmation when required, plus the extra "all documents" step when the filter is empty;
  (4) running with a Cancel button; (5) report.
- View-model logic (step state, whether typed confirmation is required, the threshold, the
  scope) is pure and egui-free, with its own tests, following `index_write.rs`.
- All text goes through the translation layer with new keys; report new keys when done.

## Testing

- Core unit tests: empty filter rejected without the flag; flag with a non-empty filter rejected;
  update validation (operators, pipeline stages, rejected stages); report shape on partial failure;
  every new write method refused by `ReadOnlyStore` and listed in `WRITES`.
- Preview tests: after-state matches what `update_many` then produces, for `$set`, `$inc`,
  `$push` and a pipeline update; the database is unchanged after the preview, including after
  an error mid-preview; a standalone server and a capped collection degrade to the before-only
  preview with the note; an upsert that matches nothing previews the inserted document.
- Core integration tests against a real MongoDB (the existing docker/local test setup; the
  preview and transaction tests need a replica set, the degradation tests a standalone):
  update many, delete many, upsert, pipeline update, count with `maxTimeMS`, a failure leaving an
  accurate report, cancel.
- View-model tests: when typed confirmation is required, the threshold boundary (99/100/101),
  production, all-documents, projection and read-only gating.
- Accessibility tests for the new dialogs and screenshot baselines, as the other tabs have.
- Coverage stays at or above 80%.

## Left to the builder

- Exact names and module layout, the precise shape of the request and report types, and the
  exact cancel mechanism, provided the behaviours above hold.
- Where in the data view the entry point sits visually.
- The split into plan tasks.
