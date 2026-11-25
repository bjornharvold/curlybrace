# Export documents to a file (#54)

Status: draft for approval. Size: a project, delivered as four PRs (one refactor, three slices).

## Intent and why

Today the only thing Curlybrace writes to a file is a Data Compare plan or report. There is no way to
export a normal query result. The person asking wants to **copy and move data between MongoDB
databases**. Copy and Paste (`docs/copy.md`) already does that live between two open connections; a
file export covers what it cannot: no second connection open, a server not reachable from this
machine, or handing someone a file. So the export has to be **type-exact by default**: what goes in
a file must come back as the same BSON types.

## Goals

- Export a whole collection, or the documents matching the current filter, to a file the person
  chooses.
- Formats: newline-delimited JSON (default), JSON array, CSV. Extended JSON in two modes: canonical
  (default, type-preserving) and relaxed (readable, labelled as not type-exact).
- CSV: field selection, nested fields flattened to dotted paths, arrays as JSON text, formula guard
  (`=`, `+`, `-`, `@`, tab, CR get a leading `'`), with a warning that types can be lost.
- Stream with bounded memory, show progress, allow cancel. Owner-only file permissions. A partial
  file is removed on failure or cancel.
- Selected documents as a source (slice 3).

## Non-goals

- Import (#55). Aggregation results (until the builder exists and its results are a stable source).
  BSON/`mongodump` format. Compression. Scheduling.

## Anti-goals (a failure even if it technically works)

- A default that silently changes types (relaxed mode, CSV) for someone moving data.
- A partial or half-written file left behind after a cancel or an error.
- A document value or the chosen path in a log, an error message or a `Debug` print.

## Constraints

- Rust 1.87 (CI pin): no let-chains.
- Every visible string is an i18n key in `crates/ui/locales/en.json`; report the new keys.
- Immutable view-model state, files under 800 lines, TDD with coverage at or above 80%.
- Reuse, do not copy: `csv_cell` (`compare/export/sanitize.rs`), the keyset reader
  `DocumentStore::scan_sorted`, the compare export's configure → choose path → run → finish state
  machine (`ExportUi`).
- Read-only connections can be exported from (it only reads).

## Approaches considered

- **A. New export job modelled on the compare export, with its own file handling.** Fastest to the
  first user-visible slice, but leaves two near-identical copies of the safest, most security
  sensitive code in the product (symlink refusal, `O_NOFOLLOW`, single-link check, `0o600`, removal
  on drop).
- **B. Extract the generic file lifecycle from the compare export first, then build on it. Chosen.**
  One PR of pure refactoring before any new behavior. Costs one extra PR and touches working Compare
  code, so it must be behavior-preserving and covered by the existing compare export tests.
- **C. Export from what the UI already holds in memory.** Rejected: cannot export a whole
  collection.

Chosen by the requester: B first.

## The plan in four PRs

**PR 0, `refactor(export)`: extract the file lifecycle.** Move out of
`core/src/runtime/compare/` into a shared module (name is the builder's call, for example
`core/src/runtime/export_file/`): `validate_path`, `FileGuard` (create / commit / remove / drop),
the buffered sink (`ExportSink`, `buffered`, and a `into_file` that returns `io::Error` instead of
the compare-only `RenderError`), and their test hooks (`fail_after_open`, `fail_writes_after`).
Compare keeps what is about its own documents: `overwrite_header` and the incomplete-header trick,
`Resolved`, `Section`, `Tally`, `FormatWriter`, `header_bytes`, `render_error`. No behavior change:
the existing compare export tests pass unmodified apart from import paths. Test-only hooks stay
test-only.

**PR 1, slice 1: collection or query to NDJSON / JSON array.** A new core job reads documents with
`scan_sorted` in ascending `_id` order in batches, renders each with canonical or relaxed extended
JSON (`ejson.rs`), and writes through the shared file module. JSON array is streamed
(`[`, comma between documents, `]`). Progress is documents written against a count taken at the
start (an estimate, said so). Cancel removes the file. The UI is a modal that mirrors the compare
export: options, system file dialog, running with progress and cancel, result. Entry points: the
documents pane and the collection context menu.

**PR 2, slice 2: CSV.** Field selection (default: the union of top-level fields seen in a sample,
editable), dotted paths, arrays as JSON text, `csv_cell` for every cell, the type-loss warning.

**PR 3, slice 3: selected documents.** The same job with a selection of `_id`s as the source, the
way Copy handles its selection (including select-all across pages).

## Decisions I made (not yet confirmed by the requester)

- **Defaults:** NDJSON, canonical extended JSON.
- **Order and shape in slice 1:** documents come out in ascending `_id` order and whole (the query's
  sort and projection are not applied), the same as Copy. Reason: `scan_sorted` is the only reader
  that is stable and fast on a large collection while data changes, and nobody moving data needs a
  display sort. The issue lists sort and projection as sources; this is a deliberate narrowing for
  slice 1, and projection is covered by field selection in slice 2.
- **Round trip test** ("export then import preserves types"): #55 does not exist, so the test
  re-parses the file with the existing `editor_json_to_doc` / `Bson::try_from` path and compares
  documents for equality.
- **No in-place header trick** (compare's "incomplete until finished" header). A cancelled or failed
  export removes the file; a crash can leave a cut-short file, which for NDJSON is still valid line
  by line and for a JSON array is invalid JSON (so it cannot pass as whole).
- **Aggregation results** are out of scope for these four PRs.

## More decisions I made after the builder check (not yet confirmed by the requester)

1. **What "the current query" exports.** The *applied* filter (the one last run, not text still being
   typed). Every matching document: skip and limit are ignored, like Copy's "select all". The modal
   says so. `scan_sorted` matches under binary collation, so when the applied query sets a
   collation the export is disabled with the reason shown, rather than exporting a different set
   than the person saw. A collection whose default collation is not simple gets the same treatment
   if the builder finds `scan_sorted` cannot honor it.
2. **Unusable `_id` (`CompareKeyMissing` / `CompareKeyInvalid`) mid-export:** abort, remove the
   file, show a value-free error. Skipping would silently drop documents, which is the anti-goal.
3. **CSV cells (PR 2).** The formula guard applies to text cells only (strings, and the text form of
   other types). Finite numbers are written as plain numbers and are exempt, so `-5` stays `-5`;
   a spreadsheet reads it as a number, not a formula. Other types: ObjectId as hex, dates as UTC
   ISO-8601, Decimal128 as its decimal string, Int64 as digits, Binary as base64, null and a missing
   field as an empty cell. The type-loss warning stays.
4. **Collection kinds.** Anything the browse pane can open is exportable (views, capped, time-series,
   `system.*`). The builder verifies `scan_sorted` and the count on each kind and adds a test per
   kind; where one cannot work, the entry is disabled with a reason.
5. **Destination exists:** overwritten, as in compare (the system save dialog already asks). **Empty
   source:** a valid empty file (`[]` for a JSON array, zero bytes for NDJSON) and a result that says
   0 documents, not an error.
6. **Not a point-in-time snapshot.** Documents changed during a long export can be missed or
   repeated, the same as Copy. The modal says so in one line, progress never exceeds 100%, and the
   result shows the number written.

## Left to the builder

Module and type names, the exact modal layout and i18n key names, the default file name, batch size
and buffer size, how the document count is taken, and the test layout.

## Done when

- [ ] PR 0 merged with compare export behavior unchanged.
- [ ] Round trip in canonical mode preserves every BSON type (test using the exotic fixture
      document).
- [ ] A large collection streams in bounded memory; cancel works and leaves no file.
- [ ] Every string is an i18n key; accessibility tests pass.
- [ ] Docs: a short `docs/export.md` and the README feature list.
