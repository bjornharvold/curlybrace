# Data Compare & Sync: Design

Date: 2026-10-12
Status: Draft, awaiting user review
Issue: #8 (epic). Reused later by #9 (masking and reschema) and #11 (task scheduler).

## Context and intent

Compare two collections, on the same server or on different servers, show exactly how they differ, and
(in later slices) apply the differences or export them. The motivating use is moving data between
environments (staging vs production, production vs a local copy) without throwaway scripts.

The feature is designed as a whole here, because apply shapes the data model, but it is **built in
three slices**. Only slice 1 is planned in detail now.

| Slice | Delivers | Writes to a database? |
|---|---|---|
| 1 | Diff engine, compare job, read-only Compare workspace | **No** |
| 2 | Apply (copy source to target, target to source, delete extras) | Yes, behind confirmation |
| 3 | Export as a `mongosh` script or extended JSON | No |

## Decisions already made with the user

1. **Scope:** design all slices now, plan and build slice 1 first.
2. **Connections:** a compare is a self-contained **job** that opens its own connections to the source
   and the target from saved connections. It does not use or disturb the browsing session.
3. **Match key:** `_id` by default, or one or more chosen fields. The key must be unique per side; a
   duplicate stops the compare with an error naming an example.
4. **Rules (all four offered):** ignore listed fields, ignore array order, numbers equal across types,
   ignore field order in documents. Strict BSON equality is the default, except that field order is
   ignored by default.
5. **Result storage:** match keys plus fingerprints held in memory under a hard cap. Field-level
   detail is fetched on demand. **No document data is written to disk.** Apply re-checks fingerprints.
6. **UI:** a Compare workspace next to Browse, a top-level switch in the main window.
7. **Engine:** sorted merge-join over two keyset-paged streams. Not a hash of one whole side.
8. **`_id` on copy:** when the key is not `_id`, a copied document keeps its source `_id` by default and
   a collision on the target is a per-document failure; the user may instead choose "let the target
   assign new `_id`s". (Proposed in the design review and approved with it.)

## Non-goals

- Two-way merge or conflict resolution. Every apply is one direction with an explicit selection.
- Comparing indexes, collection options, validators, views, GridFS or schemas. Documents only.
- Scheduling or unattended runs (that is #11).
- Comparing more than two collections at once.
- Duplicate keys: documents sharing a key are an error, never paired up by guesswork.

## Architecture

### Core units (`curlybrace-core::compare`)

No UI dependency. Each unit has one purpose and is tested on its own.

| Module | Purpose | Depends on |
|---|---|---|
| `key` | `KeySpec`, `KeyTuple`, and `compare_bson`, the client-side comparator that reproduces MongoDB's sort order | `bson` |
| `normalize` | Turns a document into a canonical form under `CompareRules` | `key` |
| `fingerprint` | 128-bit fingerprint of a normalized document | `normalize`, `sha2` |
| `diff` | Field-level difference between two documents, with BSON paths and types | `normalize` |
| `scan` | `KeyScanner`: a keyset-paged, ordered stream of one side, with order and duplicate checks | `DocumentStore`, `key` |
| `engine` | Merge-join of two scanners into a capped `ResultSet` | `scan`, `fingerprint` |
| `job` | Runtime integration: commands, events, job registry, cancellation | `engine`, runtime |

### UI units (`curlybrace-ui`)

`viewmodel/compare/` holds every decision as pure functions on immutable state (setup form and its
validation, results filter and selection, detail selection). `views/compare/` is the thin egui layer.
This is the same split the Browse feature already uses.

### Types

```rust
pub struct KeySpec { pub fields: Vec<String> }        // dotted paths; non-empty; default ["_id"]
pub struct KeyTuple(pub Vec<Bson>);                   // one value per key field, in order

pub struct CompareRules {
    pub ignore_paths: Vec<String>,                    // dotted; a path through an array maps over its elements
    pub ignore_array_order: bool,                     // default false
    pub numbers_equal_across_types: bool,             // default false
    pub ignore_field_order: bool,                     // default true
    pub include_id: bool,                             // default true when the key contains `_id`, else false
}

/// What to compare inside an open compare session (the session owns the connections).
pub struct CompareRequest {
    pub source: Namespace,
    pub target: Namespace,
    pub key: KeySpec,
    pub rules: CompareRules,
    pub limits: CompareLimits,
}

pub struct CompareLimits {
    pub batch_size: u32,                              // default 1_000 documents per scan batch
    pub max_differences: u64,                         // default 1_000_000
    pub max_result_bytes: u64,                        // default 256 MiB, estimated
    pub max_unindexed_documents: u64,                 // default 50_000 (see "Index requirement")
}

pub struct Fingerprint(pub [u8; 16]);

pub enum Category { OnlyInSource, OnlyInTarget, Changed }   // identical documents are only counted

pub struct DiffEntry {
    pub key: KeyTuple,
    pub source: Option<Fingerprint>,                  // None for OnlyInTarget
    pub target: Option<Fingerprint>,                  // None for OnlyInSource
}

pub struct CompareSummary {
    pub scanned_source: u64, pub scanned_target: u64,
    pub identical: u64, pub only_in_source: u64, pub only_in_target: u64, pub changed: u64,
}

pub struct FieldChange {
    pub path: String,                                 // a.b[2].c
    pub kind: ChangeKind,                             // Added | Removed | Changed
    pub source: Option<Bson>, pub target: Option<Bson>,
}
```

`include_id` exists because, when the key is a business key such as `email`, `_id` values differ across
environments by construction. Comparing `_id` would then mark every document as changed. So `_id` is
excluded from the comparison by default unless it is part of the key, and the user can switch it back on.

### `DocumentStore` additions

```rust
async fn estimated_count(&self, ns: &Namespace) -> Result<u64>;

/// Documents whose key tuple is strictly greater than `after` (all of them when `None`), in ascending
/// key order under BINARY (simple) collation, at most `limit`. A document missing a key field yields
/// `CoreError::CompareKeyMissing`.
async fn scan_sorted(&self, ns: &Namespace, key: &KeySpec, after: Option<&KeyTuple>, limit: u32)
    -> Result<Vec<Document>>;

/// Whether an index can serve `scan_sorted` for this key (prefix match, simple or no collation).
async fn key_index_status(&self, ns: &Namespace, key: &KeySpec) -> Result<IndexStatus>; // Indexed | NotIndexed

// Slice 2 only:
async fn bulk_write(&self, ns: &Namespace, ops: Vec<WriteOp>) -> Result<BulkOutcome>;
```

`MongoStore::scan_sorted` builds the keyset filter as an `$or` chain (for key `(a, b)` and
`after = (x, y)`: `a > x`, or `a == x and b > y`), sorts by the key fields ascending, forces
`collation: { locale: "simple" }`, and sets `allowDiskUse`. `InMemoryStore` implements the same
contract with the shared comparator, so engine tests need no server.

## The comparator and why it is the main risk

The merge-join is only correct if the client orders keys exactly as the server sorts them. The comparator
`compare_bson` follows MongoDB's documented BSON order: MinKey, Null, numbers (compared by exact numeric
value across Int32, Int64, Double and Decimal128), strings (by bytes under simple collation), objects,
arrays, binary, ObjectId, boolean, date, timestamp, regex, MaxKey. It starts from the ordering already
written for the in-memory test store (`test_support/bson_order.rs`) and fixes its known gaps: that code
compares numbers through `f64`, which is wrong above 2^53 for Int64 and for Decimal128, so it is replaced
by exact comparison.

Two defences make a mismatch loud instead of silent:

1. **Conformance test against a real server** (local-server tier): insert keys of every BSON type, including
   boundary numbers, into a throwaway collection, read them with `sort({k: 1})` and simple collation, and
   assert that the server order equals `compare_bson` order.
2. **Runtime order check:** `KeyScanner` verifies that each side arrives strictly increasing. An
   out-of-order key raises `CompareKeyOrder`; an equal adjacent key raises `CompareDuplicateKey`. The job
   fails instead of reporting false differences.

## Index requirement

Keyset paging with a sort is only cheap when an index serves the sort; otherwise every batch re-sorts the
remaining collection, which is quadratic. Before starting, a preflight (`CompareCheck`) asks each side for
`key_index_status` and `estimated_count`:

- Indexed (a usable prefix, simple or no collation): proceed.
- Not indexed and the estimated count is at most `max_unindexed_documents`: proceed with a visible warning.
- Not indexed and larger: refuse with `CompareKeyNotIndexed`. The error tells the user which index to create.

The limit applies to each side separately, so the compare is refused if either side is unindexed and above it.

An index whose collation is not simple counts as not indexed, because it cannot serve a simple-collation sort.

## Normalization and fingerprints

Normalization produces a canonical document so that equal-under-the-rules documents hash identically.
Applied in this order:

1. **Ignore fields.** Remove `ignore_paths`. A path segment applied to an array applies to every element,
   so `items.price` removes `price` from each element of `items`. Remove `_id` unless `include_id`.
2. **Numbers across types** (when enabled). Every numeric value maps to one canonical form: a value that is
   integral and fits `i64` becomes `Int64`; a finite non-integral `Double` stays `Double`; `Decimal128` is
   normalised (trailing zeros stripped) and becomes `Int64` when integral and in range. Equality is exact:
   `Double 0.1` is **not** equal to `Decimal128 0.1`, because the double is not exactly one tenth. `NaN`
   equals `NaN`, and `-0.0` equals `0.0`.
3. **Field order** (when ignored). Sort document keys recursively by byte order. When not ignored, order is
   kept and is part of equality.
4. **Array order** (when ignored). Normalize each element first, then sort the elements by their canonical
   BSON bytes. This is total and deterministic, including for arrays of documents.

The fingerprint is the first 16 bytes of SHA-256 over the canonical BSON bytes of the normalized document.
`sha2` is already in the dependency tree through the MongoDB driver. The fingerprint is computed on the
**normalized** document, so ignored fields never affect it.

## Engine

`engine::run(source_scanner, target_scanner, rules, limits, progress, cancel) -> Result<(ResultSet, CompareSummary)>`

Merge-join of two ascending streams. At each step compare the two head keys:

- source key < target key: the source document is **only in source**; record its key and fingerprint.
- source key > target key: the target document is **only in target**.
- equal: fingerprint both normalized documents; equal fingerprints count as **identical**, otherwise record a
  **changed** entry with both fingerprints.

Key matching always uses `compare_bson`, so numeric keys match across types (`Int32 1` and `Int64 1` are the
same key) whatever the rules say. If the key field itself differs in type, the pair is matched and reported as
**changed** under strict rules, with that field's path in the detail.

Memory is bounded by two batches plus the result set. The result set holds `DiffEntry` values and stops the
job with `CompareCapExceeded` when it passes `max_differences` or `max_result_bytes` (estimated from key sizes
plus 40 bytes per entry). Only counts are kept for identical documents.

## Runtime integration and job lifecycle

A compare is a **job** with its own id, independent of the browsing Connect/Disconnect epoch.

### Sessions, commands and events

A compare is a **session** (the "job") with its own id. `OpenCompare` connects both sides once; every later
command names the job id, so the setup form, the preflight, the run, the results and the detail all use the
same two connections. The same saved connection may be both sides (a same-server compare opens two
connections to it).

| Command | Single answer event | Meaning |
|---|---|---|
| `OpenCompare { source: ConnectionConfig, target: ConnectionConfig }` | `CompareOpened { job }` | Registers the job and connects both sides |
| `CompareList { job, side, db: Option<String> }` | `CompareNamespaces { job, side, db, names }` | Databases when `db` is `None`, else collections of `db`; fills the setup form's pickers |
| `CompareCheck { job, request }` | `CompareChecked { job, index: (IndexStatus, IndexStatus), estimated: (u64, u64) }` | Preflight: index status and sizes for both sides |
| `StartCompare { job, request }` | `CompareStarted { job }` | Starts the merge-join in the background |
| `CancelCompare { job }` | `CompareCancelRequested { job }` | Sets the job's cancel flag |
| `CompareRows { job, category, offset, limit }` | `CompareRowsPage { job, category, offset, total, rows }` | A page of keys for one category |
| `CompareDetail { job, key }` | `CompareDetailLoaded { job, key, source, target, changes }` | Fetches both documents by key and diffs them |
| `CloseCompare { job }` | `CompareClosed { job }` | Drops the job, its results and its connections |

Failures of these commands arrive as the usual `Failed { context, error }` with new contexts
(`compare_open`, `compare_list`, `compare_check`, `compare_start`, `compare_rows`, `compare_detail`).
A job that is Idle, Running or Finished accepts only the commands that make sense in that state; anything else
answers `Failed` with `CoreError::Unsupported` and a static message.

**Job events** carry the job id and are keyed to the job, not to a command:
`CompareProgress { job, summary, estimated_total }` (throttled to about four per second) and **exactly one**
terminal event: `CompareFinished { job, summary }`, `CompareFailed { job, error, detail }` or
`CompareCancelled { job }`. This extends the existing "exactly one event per command" rule on purpose: each
command still gets exactly one answer, and each job gets exactly one terminal event. The UI reducer drops job
events for unknown or closed job ids, the same way it drops answers from a replaced session.

`CompareFailed.detail` can carry the offending key so the user can see it in the UI. It is never logged.

### Connections, secrets and prompts

The job connects with the same `Connector`, secret store and SSH tunnel code as Connect, using the connection
configs in the request. Secrets still come only from the Keychain. The interactive host-key prompt is reused
through the existing per-connect gate, keyed to the job: a prompt for a cancelled or closed job is rejected
without asking, and a late "trust" answer is turned into a rejection so nothing is written to `known_hosts`.
After `CompareFinished` the job **keeps its two connections open** so the detail view (and, in slice 2, apply)
can re-read documents; `CloseCompare`, cancellation and app exit release them and their tunnels. A connection
that drops while a job is idle or finished is re-established by the driver on the next command; one that drops
mid-run ends the job with `CompareFailed`.

### Independence from browsing

Connect, Disconnect and the browser's epoch counters are untouched. A job's `Failed` or terminal events can
never end the browsing session, and a browsing Disconnect never cancels a job. Closing the app cancels all jobs.

## UI (slice 1)

`AppState` gains `workspace: Workspace { Browse, Compare }` and `compare: CompareUi`. A top-level switch
changes the main area. A running job keeps running while the user switches back to Browse.

- **Setup form:** source and target (saved connection, database, collection), key fields, the four rules,
  `include_id`. Picking both saved connections sends `OpenCompare`; the database and collection pickers then
  fill from `CompareList` on the open session. Changing a connection closes and reopens the session. Validation is a pure function: both sides chosen, key non-empty, ignore paths
  parse, source and target not identical.
- **Preflight:** shows the index status and sizes, with a warning or a refusal as above.
- **Running:** progress counters per category and per side, estimated percent, Cancel.
- **Results:** category tabs with counts (Only in source, Only in target, Changed; Identical shown as a
  number). A virtualised list of keys for the active category. Selecting a row loads the detail.
- **Detail:** both documents as canonical extended JSON, with the changed paths listed and highlighted,
  so a type difference (Int32 vs Int64) is visible. Documents are fetched on demand and only for the
  selected row.
- **Selection** (used by apply): per row, per filtered category, or all. Selection is a set of keys in the
  view-model; no apply happens in slice 1.

All text uses i18n keys in `en.json` (nested, alphabetical). Failures are typed `CoreError` variants.

## Errors

New typed variants, each with an `errors.*` i18n key and a static, secret-free message:

| Variant | When |
|---|---|
| `CompareDuplicateKey` | two documents on one side share a key tuple |
| `CompareKeyMissing` | a document lacks a key field |
| `CompareKeyOrder` | a side's keyset stream is not strictly increasing (comparator or collation mismatch) |
| `CompareKeyNotIndexed` | no usable index and the collection is above the unindexed limit |
| `CompareCapExceeded` | the result cap was reached |

Connection and authentication failures reuse the existing variants. Cancellation is an outcome
(`CompareCancelled`), not an error.

## Apply (slice 2, designed now)

- **Plan.** From the selection, build an `ApplyPlan`. For direction *source to target*: insert documents only in
  source, replace changed ones, delete documents only in target. *Target to source* is the mirror. The user
  switches each operation kind on or off, and can narrow the selection. Unselected documents are never touched.
- **Re-verify before writing.** For each planned operation, re-read the document from the side being written
  and from the side being copied, recompute the fingerprints with the compare's rules, and compare them with the
  recorded ones. A mismatch skips the operation with the reason "changed since the compare"; apply never
  overwrites a document that changed in the meantime. A small window between the re-read and the write
  remains and is documented.
- **Writes.** Unordered batches of 500 through `bulk_write`. Every operation reports `Applied`,
  `Skipped(Stale)` or `Failed(reason)`. A re-run after a partial failure converges, because it plans only what
  still differs.
- **Replace and `_id`.** `_id` is immutable, so when the key is not `_id` the replacement document carries the
  destination's existing `_id`. Replace matches by the key fields.
- **Insert and `_id`.** Keep the source `_id` by default; a collision is a per-document failure. The user can
  choose "let the target assign new `_id`s".
- **Confirmation.** A dialog shows the counts per operation kind and names the target connection, database and
  collection. If any delete is planned, the user must type the target collection's name to proceed.
- **Progress and cancel** like the compare job, as a second job kind.

## Export (slice 3, designed now)

Two formats of the same plan, written only to a file the user chooses (never to a temp file):

- A `mongosh` script that applies the plan with `bulkWrite` and the fingerprints as comments.
- A JSON file: `{ header, operations }` with canonical extended JSON documents.

Neither contains credentials, connection strings or hosts; the header names the database and collection and
carries a free-text label. The UI warns that the file contains document data.

## Security and privacy

- No document data is written to disk by this feature. Results are keys and fingerprints in memory only.
- Logs carry job ids, counts, durations and error kinds. They never carry keys, field values or documents.
  This matters because keys may be personal data such as emails.
- Secrets come only from the Keychain. Each job's tunnels and connections are closed on completion,
  cancellation, close and app exit.
- The host-key trust rules from the Browse feature apply unchanged to jobs.
- Apply never writes without the re-verification step and the confirmation.

## Testing

1. **Comparator:** table-driven tests for every BSON type pair and boundary numbers (Int64 above 2^53, Decimal128
   with differing exponents, `NaN`, `-0.0`); **the server conformance test** described above.
2. **Normalize and fingerprint:** each rule alone and combined, nested arrays, arrays of documents, ignored paths
   through arrays, exact numeric equality including the `0.1` case.
3. **Diff:** nested documents, arrays with and without order, every change kind, type-only differences.
4. **Scanner:** keyset paging over the in-memory store, compound keys, missing key, duplicate key, out-of-order
   stream, batch boundaries (exactly one batch, one more than a batch).
5. **Engine:** a property test that, for randomly generated pairs of collections and random rules, the
   merge-join result equals a brute-force set comparison; a memory-bound test with a counting store that asserts
   at most two batches of documents are ever in flight.
6. **Job and session:** open, list, check, start, cancel at each stage, exactly one terminal event per job,
   commands in the wrong state answer `Failed`, events for closed jobs are dropped, two jobs at once, browsing
   Disconnect while a job runs, a connection that drops mid-run, prompt gating for a cancelled job, and
   connections released on close, cancel and app exit.
7. **View-models:** form validation, category switching, selection, detail loading and stale detail answers.
8. **Integration:** tier 1 (two throwaway databases on one local server); tier 2 (two containers, cross-server,
   including an SSH-tunnelled side). Click-driven UI flows and snapshots for the Compare views use the
   existing headless UI test infrastructure.
9. **Scale:** a synthetic one-million-document compare against the in-memory store runs within the memory bound
   and finishes in reasonable time (measured and recorded, not asserted to a wall-clock number).

The default `cargo test --workspace` stays offline, database-free and GPU-free, as before.

## Slice 1 plan outline

Small pull requests, each independently tested and useful:

1. `key`, `normalize`, `fingerprint`, `diff` (pure core, no store).
2. `DocumentStore` additions (`estimated_count`, `scan_sorted`, `key_index_status`) for the in-memory and Mongo
   stores, plus the comparator conformance test.
3. `scan` and `engine`, with the property and memory-bound tests.
4. Runtime compare session: open, list, check, start, cancel, rows, detail and close, with job events.
5. Compare view-models.
6. Compare views, the workspace switch, headless UI flows and snapshots.
7. Docs: README, site, testing notes.

Slices 2 and 3 get their own plans after slice 1 ships.

## Acceptance for the whole feature (from #8)

- A design doc is merged and linked from #8 (this document).
- Two collections of one million documents compare with bounded memory, a progress indicator and cancel.
- Field-level differences are correct for nested documents, arrays and every BSON type.
- Apply is previewed, confirmed, batched, reports per-document failures, and a re-run converges (slice 2).
- Cross-server compare works with two saved connections and secrets never leave the Keychain.
- Unit tests, view-model tests and integration tests as listed above.
- All text uses i18n keys; new failures are typed errors with i18n keys.
- The README roadmap and the landing page are updated when each slice ships.

## Risks

- **Comparator drift from the server's order** is the main risk. Mitigated by the conformance test and the
  runtime order check, which turn it into a loud failure.
- **Unindexed keys** make keyset paging slow. Mitigated by the preflight, which refuses large unindexed compares.
- **Very long keys** raise the memory per difference. Mitigated by the byte-based cap, not only an entry count.
- **Drift between compare and apply** is bounded by re-verification, with a documented residual window.
