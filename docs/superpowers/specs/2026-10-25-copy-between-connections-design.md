# Copy and paste between connections: design

Issue: [#48](https://github.com/bjornharvold/curlybrace/issues/48). Builds on simultaneous
connections (#46) and the Data Compare apply pipeline (#8). Status: draft for review.

Items marked **(my call)** were decided by the author of this document, not by the product owner.
Everything else was stated or chosen by the product owner in the brainstorming conversation.

## Intent

Moving data from one MongoDB to another (staging to local, one cluster to another) today means
`mongodump` and `mongorestore` on the command line. Studio 3T offers copy-paste between instances,
but only in its paid PRO plan. The goal is that a user can do the same by selecting things with the
mouse, copying, and pasting into another connection, or by dragging them there. It should feel like
copying files in a file manager, with a safety step because the write goes to a real server.

## Goals

- Copy **a whole database, one or more collections, or one or more documents** from any open
  connection to any open connection, **including the same connection** (for example `orders` to
  `orders_copy` in the same database).
- Selecting, Copy (right-click menu and the keyboard shortcut), Paste (right-click on the target),
  and drag and drop all work.
- A confirm dialog is shown for **every** paste (see "The paste dialog"). Nothing is written before
  the user confirms.
- Large collections copy with bounded memory, with progress and Cancel, and a clear statement of
  what was written if a copy is cancelled or fails.
- Respect read-only connections. Never log document values.

## Non-goals

- Copying views or their definitions. Time-series and capped collections are not special-cased
  beyond what is listed below. **(my call)**
- Users, roles, GridFS metadata, validation rules beyond what "collection options" covers.
- A snapshot-consistent copy. The copy reads the source as it goes; writes that happen to the
  source during the copy may or may not be included. The dialog says so. **(my call)**
- Choosing key fields other than `_id` for the conflict policy. The issue mentions it; v1 uses
  `_id` only. **(my call)**
- Putting data on the operating system clipboard. **(my call)**

## Anti-goals (failure even if it technically works)

- A paste that writes to a server the user did not look at, or to a read-only connection.
- Loading a whole collection into memory.
- Document values appearing in logs, error text or the report.
- A copy that looks finished after Cancel or an error but silently wrote only part.

## What the user does

1. **Select** in the sidebar (databases, collections; several at once) or in the documents pane
   (documents). Selecting every document of a filtered query counts as copying the **query
   result** (the whole result set, not only the visible page).
2. **Copy**: right-click then "Copy", or the keyboard shortcut. The menu entry names what is
   copied ("Copy 2 collections").
3. **Paste**: right-click the target and choose Paste, or drop a dragged selection on it.
   - Collections: paste onto a database (same or other connection).
   - A whole database: paste onto a connection.
   - Documents: paste onto a collection in the sidebar (not into the open documents pane).
   - Paste is disabled with a reason when it cannot work: read-only target, source connection
     closed, a database onto a database, documents onto a database.

## The paste dialog (always shown)

Shows source and target (connection, database), and per collection the counts of **new**,
**changed** and **unchanged** documents. Controls:

- **When a document already exists:** Skip, Overwrite, or Upsert by `_id`. Default Skip.
  **(my call, default)**
- **Name** for each collection or database when the target already has one with that name and the
  paste is within the same connection (suggested `<name>_copy`). Pasting a collection into a
  database that has one of the same name on another connection is allowed and is governed by the
  conflict policy above.
- **Also copy indexes and collection options** (validator, collation, capped settings). Default
  off. **(my call, default)** The `_id` index is never copied.
- Copy and Cancel buttons. Overwrite, and any target that the connection manager marks as
  production, additionally requires the typed confirmation the Compare apply already uses; plain
  Skip/Upsert to a non-production target needs only the Copy button. **(my call)**

The counts come from the compare engine (merge-join on `_id`, fingerprints). For large
collections they are computed while the dialog is open and shown as "counting…" with a running
number. Copy stays available while counting. **(my call)**

## Behavior details (all **my call**, settled after a builder review)

- **Conflict policies.** A "conflict" is a target document with the same `_id`. Nothing is ever
  deleted from the target by any policy.
  - **Skip:** insert documents that are new, leave existing ones alone.
  - **Overwrite:** replace existing documents with the source document (whole document, no field
    merge); documents that are new are **not** inserted.
  - **Upsert by `_id`:** replace existing documents and insert new ones.
- **Documents pasted into their own collection** (same connection, same namespace): Paste is
  disabled with a reason. Duplicating a document with a fresh `_id` is what the existing Clone
  action is for.
- **Document selection in the clipboard.** Hand-picked documents are kept as a list of their
  `_id`s (any BSON type, not only ObjectId), capped at 10,000; beyond that the user is asked to
  select the whole result instead. "The whole query result" keeps the filter and sort as they were
  when the user copied. Counts in the dialog are computed at paste time, so the user sees what
  will actually be copied, even if the source changed in between.
- **Whole-database paste.** The dialog lists every collection with its counts, in a scrollable
  list; counts are computed one collection after another in the background. `admin`, `local`,
  `config` and `system.*` collections are never copied and the database is not offered for copy
  when it is one of them. Pasting into an existing database on another connection merges per
  collection under the chosen policy.
- **Indexes and options order.** The collection is created with its options first (only when it
  does not exist yet on the target), the data is copied next, and indexes are built last. If the
  target collection already exists, options are not applied and only indexes it lacks (by name)
  are added. An index that fails to build is reported; the copied data stays.
- **Counts versus the copy.** The counts inform the user; the copy applies the chosen policy to
  whatever it finds when it writes and does not depend on the counts. If the user confirms while
  counting is still running, the dialog says counts were incomplete. The progress denominator is
  the estimated document count.
- **Delivery in slices.** One design, built in this order, each slice working on its own:
  (1) the copy engine and collection and database paste, (2) documents, (3) indexes and options,
  (4) drag and drop. The keyboard and menu path is required; drag and drop is a nice-to-have
  and yields if the UI toolkit makes it hard. Only the part of #52 needed to create a collection
  with options belongs in this work.

## Progress, cancel and the report

- A progress view per collection with documents written so far, and Cancel at all times.
- On cancel or error the message states exactly what was written, per collection ("12,300
  documents were written to orders. Nothing else was changed."). Documents already written are
  not rolled back.
- After the run, a per-document report of failures (key only, bounded like the compare report,
  never values) is available.

## Constraints and what to reuse

- Streaming and batching: reuse the compare engine's key handling, `scan_sorted` and the
  `APPLY_BATCH` write batching. Memory stays bounded however large the collection.
- Read-only: reuse the `ReadOnlyStore` guard and the existing `ReadOnly` error path; the target
  store refuses the writes in the core, the UI only disables the entries.
- Confirmation: reuse the Compare apply typed-confirmation mechanism and its state handling.
- The clipboard holds **references** (source connection, namespaces, selection or filter), not
  data, so copying is instant and the data is read at paste time. If the source connection has
  been closed when pasting, Paste is disabled.
- Same connection: source and target use the same session; the scan and the writes must not
  deadlock or read their own writes. Pasting a collection onto its own database requires a new
  name.
- i18n: every string is a key in `en.json` and new keys are reported. Accessibility tests must
  pass; the whole flow must be possible from the keyboard, not only by mouse or drag.
- No document values in logs.

## Known gaps in the current code (the builder should confirm)

- `DocumentStore` has no operation to create a collection with options, and no way to read a
  collection's options. Both are needed for "also copy options". Issue #52 (create, rename and
  drop collections) is not built; this work may need a small part of it.
- Multi-select in the sidebar and in the documents pane may not exist yet.
- Drag and drop between sidebar nodes may not exist yet.

## Done when

- Copy works between two real MongoDB containers (integration test), and within one container.
- The dialog shows counts; cancel leaves the target consistent or says exactly what was written.
- Read-only connections are refused in the core; every string is an i18n key; accessibility tests
  pass.

## Left to the builder

Names and layout of types and modules, how the clipboard state is held, how counts are computed
for huge collections beyond the approach above, keyboard shortcut choice, drag-and-drop mechanics
in the UI toolkit, and the integration test fixtures.
