# Copy between connections

Copy and paste moves a database, collections or documents from one open connection to another, or
within the same one, with a dialog before anything is written. It is an alternative to
`mongodump` and `mongorestore` for moving data between servers (staging to local, one cluster to
another) without leaving the app. The short version is in the
[README](../README.md#what-it-does-today).

- [Select and copy](#select-and-copy)
- [Paste](#paste)
- [The paste dialog](#the-paste-dialog)
- [Conflict policies](#conflict-policies)
- [Indexes and collection options](#indexes-and-collection-options)
- [Progress, cancel and the report](#progress-cancel-and-the-report)
- [When Paste is disabled](#when-paste-is-disabled)
- [Limits](#limits)

## Select and copy

- **In the sidebar**, select databases or collections with Cmd/Ctrl-click, Shift-click or the
  keyboard. Select one database, or collections of one database.
- **In the documents pane**, tick documents with their checkboxes. **Select all documents of the
  query** copies the whole result, not only the page shown. The filter and sort are the ones in
  place when you copy, and a projection is ignored, so full documents are copied.

Then right-click and choose **Copy** (the entry names what is copied, for example "Copy 2
collections"), or press Cmd/Ctrl+C in the sidebar.

Nothing goes on the operating system clipboard. The app keeps a *reference* to what you copied (the
connection, the names, or the selected keys or filter), and the data is read when you paste. So
the copy itself is instant, and the dialog shows what the source holds at paste time.

## Paste

Right-click the target and choose **Paste**, press Cmd/Ctrl+V, or drag the selection onto the
target.

| You copied | Paste it onto |
| --- | --- |
| One or more collections | A database, on the same or another connection |
| A whole database | A connection |
| Documents | A collection in the sidebar |

Pasting collections onto their own database needs a new name, because a collection cannot be
copied onto itself.

## The paste dialog

The dialog appears on **every** paste. Nothing is written until you press **Copy**. It shows:

- **From** and **To**: the connection and database of each side.
- **Documents per collection**: how many are *new*, *changed* and *unchanged* in the target. The
  counts are computed while the dialog is open. **Copy** stays available while they are being
  counted; the report then says the counts were incomplete. If a collection is too large to count
  (the same cap as [Compare](compare.md#requirements-and-limits)), it shows "counts unavailable"
  and **Copy** stays enabled.
- **Names in the target**, editable. The suggestion is `<name>_copy`. When
  you paste within the same connection onto a name that already exists in the target database,
  even when that database is another one, the suggested name is a new one; you can type the
  original name back to merge into the existing collection under the policy. Across connections the names are kept and a collection
  that already exists is merged under the policy.
- **When a document already exists in the target**: the [policy](#conflict-policies).
- **Also copy indexes and collection options**, off by default.
- A **typed confirmation** when the policy is Overwrite or Upsert (both replace existing documents) or the target connection is marked as
  production. Type the target collection name when one collection is pasted, otherwise the target
  database name. A production warning names the connection.

The counts inform you, but the copy does not depend on them: it applies your policy to whatever it
finds when it writes.

## Conflict policies

A document *exists* when the target holds one with the same `_id`. Only `_id` is supported as the
key.

| Policy | Existing documents | New documents |
| --- | --- | --- |
| **Skip** (default) | left alone | inserted |
| **Overwrite** | replaced by the source document (the whole document, no field merge) | **not** inserted |
| **Upsert by `_id`** | replaced | inserted |

**No policy ever deletes anything from the target.** Documents that only exist in the target stay.

Overwrite into a collection that does not exist yet creates nothing, since it never adds documents.
The report says so.

## Indexes and collection options

With **Also copy indexes and collection options** ticked:

- The collection is created with the source's options first, but only when it does not exist in the
  target. Validator, collation and capped settings are copied. Other creation options (clustered
  indexes, encrypted fields) are not.
- If the target collection already exists, its options are never touched.
- Indexes are built last. Only indexes the target lacks, by name, are added. The `_id` index is
  never copied.
- An index that fails to build is reported by name with the reason, and the copied data stays.
- **Views and time-series collections** that you paste explicitly, with this option ticked, copy
  their documents only, with a note in the report. They get no options or indexes. Without the
  option there is no note. See [Limits](#limits) for what a whole-database paste does with them.

## Progress, cancel and the report

A progress view lists each collection with the documents written, skipped and failed so far, and
**Cancel copy** is available at all times.

When the copy finishes, is cancelled or stops on an error, the report states exactly what was
written to each collection, and which collections were not started. **Documents already written
are not rolled back.**

- Documents that could not be copied are listed by **key only**, never by value, up to 100. More
  failures are counted but not listed.
- If the copy was cancelled, or a connection was closed or reconnected during it, indexes that were
  not yet built are not built, and the report says so.

## When Paste is disabled

Paste (and Copy) are disabled with the reason shown, when:

- the target connection is read-only, or the source or target connection is closed. Read-only
  connections are also refused in the core, not only in the UI;
- nothing has been copied, or no document is selected;
- you paste a database onto a database, collections onto a collection, or documents onto a
  database;
- you paste documents onto the collection they came from (use **Clone** to duplicate a document);
- the source is a `system.*` collection, or one of the databases `admin`, `local` and `config`,
  which are server-owned. They are not written to either;
- more than 10,000 documents are picked by hand (see below).

## Limits

- **Hand-picked documents are capped at 10,000.** Beyond that, use **Select all documents of the
  query**.
- **"Select all documents of the query" matches with the simple collation**, so its results (and
  the preview's counts) can differ from the browse view on collections with a case-insensitive
  collation. The copy reads by `_id` order, which has to use the same collation on both sides.
- **No snapshot consistency.** The source is read as the copy goes, so writes to it during the copy
  may or may not be included.
- **Top-level field names starting with `$`** cannot be replaced by the server (error 52). Such
  documents fail to overwrite or upsert, and are reported by key.
- **Only `_id` is supported** as the key for the policies.
- **A whole-database paste leaves out views and time-series collections**, silently and with no
  note: only plain collections are copied. Paste such a collection by itself to copy its documents.
- **One copy at a time.**
- Creation options other than validator, collation and capped settings are not copied (see
  [above](#indexes-and-collection-options)).
