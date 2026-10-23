# Compare collections and databases

Data Compare shows what differs between two collections or two whole databases, on the same server
or on different ones. It can then copy or delete the differences, or export them, with a review step
before anything is written. This guide covers every part of it. The short version is in the
[README](../README.md#what-it-does-today).

- [Start a compare](#start-a-compare)
- [Pairs, settings and runs](#pairs-settings-and-runs)
- [How documents are compared](#how-documents-are-compared)
- [Apply changes](#apply-changes)
- [Copy single fields](#copy-single-fields)
- [Export](#export)
- [Requirements and limits](#requirements-and-limits)

## Start a compare

Switch from **Browse** to **Compare** next to the Disconnect button, or right-click a collection and
choose **Compare to...** to start with it as the source. Pick a saved connection for each side (the
two sides can be on different servers) and press **Connect**, then pick a database and collection
for each side, press **Add pair**, then **Check** and **Run**.

**Comparing never writes.** Writing only happens when you apply changes (see
[Apply changes](#apply-changes)). The results are kept in memory for the session and can be
discarded.

**Read-only connections are never written to.** A connection saved as read-only can be compared on
either side, but it cannot be picked as the target, and no change is ever copied into it or deleted
from it. When the source is read-only, **Copy to source**, **Delete from source** and **Copy selected
fields to source** are disabled with the reason, and changes already queued for it are marked
"Read-only" and keep **Review changes** and **Apply** closed until you remove them.

## Pairs, settings and runs

A compare is a **list of pairs**.

- Add one collection pair, or hand-pair collections with different names.
- Press **Add all same-named** to pair every collection two databases share. Views are skipped,
  system collections only if you switch them on, and collections without a counterpart are listed so
  you can pair them by name.
- Right-click a database and choose **Compare database to...** to start a whole-database compare.
  When the target is on a different connection, the same database name is picked there for you.

The key, rules and filters at the top are **defaults for every pair**. Open a pair and choose
**Edit settings** to override them for just that pair.

The **check** shows a verdict per pair:

- **Run** starts the included pairs that can be checked, and needs each of them to pass.
- **Run passing** starts only the pairs that passed, skipping invalid pairs, failed checks and
  refused pairs.

The run overview shows each pair's status and counts, with totals, and you can cancel the run. For a
finished pair you can **Open** its results, **Discard** it, or **Re-run** it (it opens in Edit
settings, for instance to change the key).

## How documents are compared

- Documents are matched by a **key** (default `_id`). The results list what is only in the source,
  only in the target, or different, with the changed fields of each document.
- Optional **filters** limit each side.
- The **Rules** section controls array order, field order, number types, the `_id` field and which
  fields count.
- The key must be **unique** on each side. A duplicate key stops the run with an error, and **Find
  duplicate keys** then shows which values repeat.
- Each document must have a value for every key field, and a key value cannot be an array or a
  regular expression.

## Apply changes

1. Tick rows of a results tab (or **Select all** in the tab, which covers every row of it, not only
   the page shown).
2. Choose an action:
   - **Copy to target** or **Copy to source** for documents that exist on one side or differ.
   - **Delete from source** or **Delete from target** for extras.
3. Actions are queued under **Pending changes**. Nothing is written until you press **Review
   changes**. The review names the connection, database and collection each action writes to, with
   the counts per pair.
4. Deleting needs you to type the collection name before **Apply** is enabled. When several
   collections are affected, type the database name instead; when they span several databases or
   connections, type a confirmation word.
5. Each document is checked again just before it is written. A document that changed since the
   compare is skipped and left alone.
6. The result lists what was **applied**, **skipped**, **failed** (with the document's key) and **not
   run**. You can cancel a running apply. The pair then shows **Applied**: run it again to confirm
   what is left.

## Copy single fields

To change only some fields of a changed document, open it, tick the fields and choose **Copy
selected fields to target** or **Copy selected fields to source**.

- Only those fields are written. A field that exists on one side only is removed from the other. The
  rest of the destination document stays as it is.
- A field that cannot be copied on its own is shown disabled, with the reason. These are the `_id`, a
  name with a dot or starting with `$`, an item of an array whose order is ignored, and an array item
  that was added or removed.
- Field copies are queued and reviewed like the other actions.

## Export

Instead of (or before) applying, you can write the result to a file you choose. Exporting never
writes to a database.

**Export changes...** in the pending list writes the queued changes as:

- a **mongosh script**: one `bulkWrite` per collection, the fingerprints as comments, for one side of
  the comparison at a time; or
- a **JSON plan** in canonical extended JSON, which also works for changes to both sides.

Each document is checked again when you export. Anything that changed since the compare, or that the
shell cannot express (for example an `undefined` or DBPointer value, or a document with number-like
field names that JavaScript would reorder), is listed as skipped in the file and never written as an
operation.

**Export report...** in the overview writes the differences as a **CSV file**, or copies them to the
**clipboard** (up to 10,000 rows). A row has the pair, key and category and, if you ask for it, the
changed field paths. Values are never included.

Things to know about exported files:

- They contain **document data**. A report contains keys, which can be personal data. They do not
  contain connection names, hosts or passwords.
- They are created with owner-only permissions and written only where you choose.
- If an export fails or is cancelled, no unfinished file is left. Once an export has started, a file
  that was already at that path has been replaced and is removed too, so choose a new name if you
  want to keep the old one.
- A script stops at the first failing collection (exit code 2) and runs nothing after it.
- A script is a snapshot of an earlier comparison: the documents may have changed again by the time
  you run it, so read it first.

## Requirements and limits

- The key fields need an **index** on each side. A collection with up to 50,000 documents is compared
  without one, but a larger one is refused until you create an index on the key fields. The check
  tells you which side lacks it, and how many documents each side has.
- A run stops at 1,000,000 differences or 256 MiB of kept results. Narrow the comparison with a
  filter or fewer fields.
