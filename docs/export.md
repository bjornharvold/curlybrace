# Export documents

Export writes the documents of one collection to a file, as NDJSON or as a JSON array of extended
JSON. Use it to keep a copy, to inspect data in other tools, or to move documents between
databases. The short version is in the [README](../README.md#what-it-does-today).

- [Start an export](#start-an-export)
- [Formats and modes](#formats-and-modes)
- [What is written](#what-is-written)
- [The file](#the-file)
- [Moving data between databases](#moving-data-between-databases)
- [Limits](#limits)

## Start an export

Right-click a collection in the sidebar:

- **Export documents** writes the whole collection.
- **Export documents matching the filter...** writes the documents the applied filter matches. It
  is offered on the collection shown in the documents pane, while a filter is applied. The applied
  filter, not the text still being typed in the filter box, is the source. It is disabled when the
  view uses a collation.

Choose the format and mode, then pick the file in the save dialog. Progress is shown while the
export runs, and **Cancel** stops it.

## Formats and modes

| Format | Layout |
| --- | --- |
| NDJSON | One compact JSON document per line. |
| JSON array | One JSON array, one document per line. |

| Mode | Meaning |
| --- | --- |
| Canonical | Extended JSON that keeps every BSON type: `Int32`, `Int64`, `Double`, `Decimal128`, dates, `ObjectId`, binary, regular expressions, timestamps, `MinKey` and `MaxKey` all come back as the same type. |
| Relaxed | Easier to read, but not type-exact: numbers lose their BSON width (an `Int64` reads back as a plain number), and dates and `Double` values are written in their readable form. |

Choose canonical when the file will be loaded into a database again. Choose relaxed when people or
scripts will read it.

## What is written

- Documents come out whole, in ascending `_id` order. Sort, projection, skip and limit set in the
  documents pane are not applied.
- The export is not a point-in-time snapshot. Documents that change while it runs may be written
  in either state, and documents added or removed during the run may or may not be included.
- A document without a usable `_id` stops the export with an error.

## The file

- If the file exists, the save dialog asks you to confirm, and the export then overwrites it.
- The file is readable and writable by its owner only.
- If you cancel, or the export fails, the file is removed. A file is kept only when every document
  was written.

## Moving data between databases

1. Export the collection as NDJSON in canonical mode.
2. Load the file into the other database with `mongoimport`, which reads the extended JSON this
   export writes, or with Curlybrace import once it exists
   ([#55](https://github.com/bjornharvold/curlybrace/issues/55)). For a JSON array file, tell
   `mongoimport` the input is an array (`--jsonArray`).

To copy between connections without a file, see [copy](copy.md).

## Limits

- One export runs at a time.
- Relaxed mode does not preserve BSON types, so do not use it for a round trip.
